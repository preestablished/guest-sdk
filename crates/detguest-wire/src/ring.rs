//! SPSC byte rings over shared channel memory (ARCHITECTURE.md §2).
//!
//! This is the **only** module in `detguest-wire` permitted to contain unsafe
//! code (IMPLEMENTATION-PLAN M6 permitted-unsafe list): it does the raw-pointer
//! arithmetic over the mapped channel page that safe Rust cannot express, and
//! encapsulates the acquire/release index discipline so no other module needs
//! to think about it.
//!
//! Protocol (both sides, both directions):
//! - Producer: write record bytes, then `Release`-store the new producer index.
//! - Consumer: `Acquire`-load the producer index, read records, then
//!   `Release`-store the new consumer index.
//! - Indices are free-running `u32`s masked by `size - 1` (sizes are powers of
//!   two). Records are 8-byte aligned and never wrap: a record that does not
//!   fit in the tail is preceded by a `Pad` (kind 0) covering the whole tail.
//!
//! Soundness argument for the slices handed out below: the producer exclusively
//! owns the free region `[prod, cons + size)` and the consumer exclusively owns
//! the used region `[cons, prod)`; the regions are disjoint, and ownership of
//! bytes is transferred between sides only through the release/acquire pairs on
//! the index cells — the same split-borrow reasoning as `split_at_mut`, with
//! the atomics providing the happens-before edges.
#![allow(unsafe_code)]

use core::marker::PhantomData;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::record::{encode_pad, RecordHeader, MAX_RECORD_LEN, PAD_MIN_LEN, RECORD_ALIGN};
use crate::{DecodeError, EncodeError};

/// Bytes currently in the ring (free-running index math; wrapping-safe).
pub const fn used(prod: u32, cons: u32) -> u32 {
    prod.wrapping_sub(cons)
}

/// Free bytes in the ring. Saturates (instead of underflowing) on a
/// corrupt/forged index pair where `used > size` — consistent with the
/// crate's posture that arbitrary bytes never cause a panic.
pub const fn free(prod: u32, cons: u32, size: u32) -> u32 {
    size.saturating_sub(used(prod, cons))
}

/// Contiguous bytes from the producer position to the ring end.
pub const fn contiguous_tail(prod: u32, size: u32) -> u32 {
    size - (prod & (size - 1))
}

/// Total ring bytes a record of `len` will consume at producer position
/// `prod`: `len`, plus the whole tail when a `Pad` is needed first.
pub const fn bytes_needed(prod: u32, size: u32, len: u32) -> u32 {
    let tail = contiguous_tail(prod, size);
    if len > tail {
        tail + len
    } else {
        len
    }
}

/// Push failure: not enough free space for the record (plus any tail pad).
///
/// The caller decides policy: droppable events bump the drop counters and
/// return; critical events doorbell and retry (ARCHITECTURE.md §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RingFull;

/// Producer half of one ring. Single-owner: exactly one `Producer` may exist
/// per ring side (the SPSC contract is the caller's to uphold via ownership).
///
/// Deliberately `Send` but **not** `Sync`: a half may move to another thread,
/// but `&Producer` must never be shared across threads — all mutation goes
/// through `&mut self`.
pub struct Producer<'a> {
    data: *mut u8,
    size: u32,
    prod: &'a AtomicU32,
    cons: &'a AtomicU32,
    next_seq: u32,
    _marker: PhantomData<&'a mut [u8]>,
}

// The raw data pointer is only dereferenced inside the disciplined push path;
// moving the half to another thread is exactly the intended SPSC usage.
unsafe impl Send for Producer<'_> {}

impl<'a> Producer<'a> {
    /// Build the producer half over a mapped ring.
    ///
    /// # Safety
    /// - `data` must point to `size` valid bytes that live for `'a`.
    /// - `size` must be a power of two, ≥ [`MAX_RECORD_LEN`].
    /// - `prod`/`cons` must point to the ring's index cells (valid, 4-aligned,
    ///   live for `'a`), shared with the consumer side only.
    /// - At most one `Producer` per ring; the caller owns the SPSC contract.
    /// - `next_seq` must continue the ring's record sequence (0 on a fresh ring).
    ///
    /// # Panics
    /// If `size` is not a power of two at least [`MAX_RECORD_LEN`]. This is a
    /// real assert (not debug-only) because the invariant guards the pointer
    /// arithmetic behind every slice this type hands out.
    pub unsafe fn from_raw(
        data: *mut u8,
        size: u32,
        prod: *mut u32,
        cons: *mut u32,
        next_seq: u32,
    ) -> Producer<'a> {
        assert!(size.is_power_of_two() && size as usize >= MAX_RECORD_LEN);
        Producer {
            data,
            size,
            // prod is this side's cell (load + store); cons is the peer's
            // cell: load-only here, never stored.
            prod: AtomicU32::from_ptr(prod),
            cons: AtomicU32::from_ptr(cons),
            next_seq,
            _marker: PhantomData,
        }
    }

    /// Ring size in bytes.
    pub fn size(&self) -> u32 {
        self.size
    }

    /// The seq the next pushed record will carry.
    pub fn next_seq(&self) -> u32 {
        self.next_seq
    }

    /// Current free bytes (snapshot; only grows concurrently).
    pub fn free_bytes(&self) -> u32 {
        let p = self.prod.load(Ordering::Relaxed);
        let c = self.cons.load(Ordering::Acquire);
        free(p, c, self.size)
    }

    /// Push one record of exactly `total_len` bytes, encoded in place by
    /// `encode(buf, seq)` (which must fill `buf` completely and is handed the
    /// seq allocated for the record — pads consume their own seq first).
    ///
    /// On success returns the record's seq. On [`RingFull`] nothing is written
    /// and no seq is consumed. A single `Release` store publishes the pad and
    /// record together.
    pub fn try_push(
        &mut self,
        total_len: usize,
        encode: impl FnOnce(&mut [u8], u32) -> Result<usize, EncodeError>,
    ) -> Result<u32, RingFull> {
        debug_assert!(
            total_len % RECORD_ALIGN == 0 && (PAD_MIN_LEN..=MAX_RECORD_LEN).contains(&total_len)
        );
        let len = total_len as u32;
        let prod = self.prod.load(Ordering::Relaxed); // sole writer of prod
        let cons = self.cons.load(Ordering::Acquire);
        let needed = bytes_needed(prod, self.size, len);
        if free(prod, cons, self.size) < needed {
            return Err(RingFull);
        }
        let mask = self.size - 1;
        let mut pos = prod;
        if needed > len {
            // Pad the whole tail, then start the record at offset 0.
            let tail = contiguous_tail(prod, self.size) as usize;
            let seq = self.alloc_seq();
            let dst = self.slice_mut(pos & mask, tail);
            encode_pad(dst, tail, seq).expect("tail pad fits by construction");
            pos = pos.wrapping_add(tail as u32);
            debug_assert_eq!(pos & mask, 0);
        }
        let seq = self.alloc_seq();
        let dst = self.slice_mut(pos & mask, total_len);
        let written = encode(dst, seq).map_err(|_| RingFull)?;
        debug_assert_eq!(written, total_len, "encoder must fill the claimed length");
        self.prod
            .store(prod.wrapping_add(needed), Ordering::Release);
        Ok(seq)
    }

    fn alloc_seq(&mut self) -> u32 {
        let s = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        s
    }

    fn slice_mut(&mut self, off: u32, n: usize) -> &mut [u8] {
        debug_assert!(off as usize + n <= self.size as usize);
        // SAFETY: range lies inside the free region exclusively owned by this
        // producer (checked against cons above); see module-level argument.
        // `&mut self` keeps the borrow checker's aliasing net: the returned
        // slice borrows the producer, so two live slices cannot coexist.
        unsafe { core::slice::from_raw_parts_mut(self.data.add(off as usize), n) }
    }
}

/// Consumer half of one ring.
///
/// Like [`Producer`]: `Send` but **not** `Sync` — move it, don't share it.
pub struct Consumer<'a> {
    data: *const u8,
    size: u32,
    prod: &'a AtomicU32,
    cons: &'a AtomicU32,
    _marker: PhantomData<&'a [u8]>,
}

unsafe impl Send for Consumer<'_> {}

impl<'a> Consumer<'a> {
    /// Build the consumer half over a mapped ring.
    ///
    /// # Safety
    /// Same requirements as [`Producer::from_raw`], consumer side: at most one
    /// `Consumer` per ring, valid `data`/`size`/index cells for `'a`.
    ///
    /// # Panics
    /// If `size` is not a power of two at least [`MAX_RECORD_LEN`] (real
    /// assert — the invariant guards the pointer math below).
    pub unsafe fn from_raw(
        data: *const u8,
        size: u32,
        prod: *const u32,
        cons: *const u32,
    ) -> Consumer<'a> {
        assert!(size.is_power_of_two() && size as usize >= MAX_RECORD_LEN);
        Consumer {
            data,
            size,
            // prod is the peer's cell: load-only here, never stored. cons is
            // this side's cell (load + store).
            prod: AtomicU32::from_ptr(prod as *mut u32),
            cons: AtomicU32::from_ptr(cons as *mut u32),
            _marker: PhantomData,
        }
    }

    /// Bytes currently readable (snapshot).
    pub fn used_bytes(&self) -> u32 {
        let p = self.prod.load(Ordering::Acquire);
        let c = self.cons.load(Ordering::Relaxed);
        used(p, c)
    }

    /// Pop the next record (including `Pad`s — callers skip kind 0) into
    /// `scratch`, returning the record length, or `Ok(None)` when empty.
    ///
    /// The consumer index is `Release`-stored after the copy, handing the bytes
    /// back to the producer. Framing violations return `Err` and consume
    /// nothing — on a correctly-produced ring they indicate memory corruption,
    /// and the caller should stop draining.
    pub fn pop_into(&mut self, scratch: &mut [u8]) -> Result<Option<usize>, DecodeError> {
        let prod = self.prod.load(Ordering::Acquire);
        let cons = self.cons.load(Ordering::Relaxed); // sole writer of cons
        let avail = used(prod, cons);
        if avail == 0 {
            return Ok(None);
        }
        let mask = self.size - 1;
        let off = cons & mask;
        let tail = contiguous_tail(cons, self.size);
        // Peek the 8-byte header prefix to learn the record length.
        if avail < PAD_MIN_LEN as u32 || tail < PAD_MIN_LEN as u32 || scratch.len() < PAD_MIN_LEN {
            return Err(DecodeError::Truncated);
        }
        self.copy_out(off, &mut scratch[..PAD_MIN_LEN]);
        let len = u16::from_le_bytes(scratch[0..2].try_into().unwrap()) as usize;
        let kind = scratch[2];
        let min = if kind == 0 {
            PAD_MIN_LEN
        } else {
            crate::record::MIN_RECORD_LEN
        };
        if len % RECORD_ALIGN != 0 || len < min || len > MAX_RECORD_LEN {
            return Err(DecodeError::BadLen);
        }
        if len as u32 > avail || len as u32 > tail {
            // Records never wrap and are published whole; this is corruption.
            return Err(DecodeError::Truncated);
        }
        if scratch.len() < len {
            return Err(DecodeError::Truncated);
        }
        self.copy_out(off, &mut scratch[..len]);
        // Defense-in-depth re-parse over the local copy: the inline checks
        // above already validated the framing, but they read ring memory the
        // producer side could theoretically be racing; the local copy is the
        // version of record. Keep both.
        RecordHeader::read_from(&scratch[..len])?;
        self.cons
            .store(cons.wrapping_add(len as u32), Ordering::Release);
        Ok(Some(len))
    }

    fn copy_out(&self, off: u32, dst: &mut [u8]) {
        debug_assert!(off as usize + dst.len() <= self.size as usize);
        // SAFETY: range lies inside the used region exclusively owned by this
        // consumer (between cons and the acquired prod); see module argument.
        unsafe {
            core::ptr::copy_nonoverlapping(
                self.data.add(off as usize),
                dst.as_mut_ptr(),
                dst.len(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{encode_event, encoded_event_len, EventPayload};
    use crate::record::EventKind;
    use std::boxed::Box;
    use std::vec;
    use std::vec::Vec;

    #[test]
    fn index_math_boundaries() {
        let size = 1u32 << 12;
        // empty and full
        assert_eq!(free(0, 0, size), size);
        assert_eq!(used(0, 0), 0);
        assert_eq!(free(size, 0, size), 0);
        assert_eq!(used(size, 0), size);
        // free-running wrap across u32::MAX
        assert_eq!(used(4, u32::MAX - 3), 8);
        assert_eq!(free(4, u32::MAX - 3, size), size - 8);
        // corrupt index pair (used > size) saturates instead of underflowing
        assert_eq!(free(0, 1, size), 0);
        // tail math at the ring end
        assert_eq!(contiguous_tail(0, size), size);
        assert_eq!(contiguous_tail(size - 8, size), 8);
        assert_eq!(contiguous_tail(size.wrapping_mul(3) - 8, size), 8);
        // record exactly filling the tail needs no pad; one byte over pads
        assert_eq!(bytes_needed(size - 16, size, 16), 16);
        assert_eq!(bytes_needed(size - 8, size, 16), 8 + 16);
        assert_eq!(bytes_needed(0, size, 16), 16);
    }

    /// A self-contained ring for tests: data buffer + index cells.
    struct TestRing {
        data: Vec<u8>,
        prod: Box<u32>,
        cons: Box<u32>,
    }

    impl TestRing {
        fn new(size: usize) -> TestRing {
            TestRing {
                data: vec![0u8; size],
                prod: Box::new(0),
                cons: Box::new(0),
            }
        }

        fn halves(&mut self) -> (Producer<'_>, Consumer<'_>) {
            let size = self.data.len() as u32;
            let data = self.data.as_mut_ptr();
            let prod: *mut u32 = &mut *self.prod;
            let cons: *mut u32 = &mut *self.cons;
            // SAFETY: buffers and cells outlive the halves; one of each.
            unsafe {
                (
                    Producer::from_raw(data, size, prod, cons, 0),
                    Consumer::from_raw(data, size, prod, cons),
                )
            }
        }
    }

    fn push_frame_mark(p: &mut Producer<'_>, frame_index: u32) -> Result<u32, RingFull> {
        let ev = EventPayload::FrameMark { frame_index };
        p.try_push(encoded_event_len(&ev), |buf, seq| {
            encode_event(buf, seq, 7, 0, &ev)
        })
    }

    #[test]
    fn push_pop_round_trip() {
        let mut r = TestRing::new(4096);
        let (mut p, mut c) = r.halves();
        for i in 0..10 {
            push_frame_mark(&mut p, i).unwrap();
        }
        let mut scratch = [0u8; MAX_RECORD_LEN];
        for i in 0..10 {
            let n = c.pop_into(&mut scratch).unwrap().unwrap();
            let (hdr, ev) = crate::events::decode_event(&scratch[..n]).unwrap();
            assert_eq!(hdr.seq, i);
            assert_eq!(ev, EventPayload::FrameMark { frame_index: i });
        }
        assert_eq!(c.pop_into(&mut scratch).unwrap(), None);
    }

    #[test]
    fn wrap_inserts_pad_and_seq_stays_monotonic() {
        // Ring sized so records hit the tail mid-stream. FrameMark records are
        // 24 bytes; a 4096 ring fits 170 of them with 16 tail bytes left.
        let mut r = TestRing::new(4096);
        let (mut p, mut c) = r.halves();
        let mut scratch = [0u8; MAX_RECORD_LEN];
        let mut expected_seq = 0u32;
        let mut popped_frames = 0u32;
        let mut pushed_frames = 0u32;
        // Push/pop interleaved long enough to wrap several times.
        for _ in 0..1000 {
            while push_frame_mark(&mut p, pushed_frames).is_ok() {
                pushed_frames += 1;
            }
            while let Some(n) = c.pop_into(&mut scratch).unwrap() {
                let hdr = RecordHeader::read_from(&scratch[..n]).unwrap();
                assert_eq!(hdr.seq, expected_seq, "seq gap — pads must consume seqs");
                expected_seq = expected_seq.wrapping_add(1);
                if hdr.kind == EventKind::Pad as u8 {
                    continue;
                }
                let (_, ev) = crate::events::decode_event(&scratch[..n]).unwrap();
                assert_eq!(
                    ev,
                    EventPayload::FrameMark {
                        frame_index: popped_frames
                    }
                );
                popped_frames += 1;
            }
        }
        assert_eq!(popped_frames, pushed_frames);
        assert!(
            expected_seq > pushed_frames,
            "at least one pad must have occurred"
        );
    }

    #[test]
    fn full_ring_reports_ring_full_and_drains_clean() {
        let mut r = TestRing::new(4096);
        let (mut p, mut c) = r.halves();
        let mut pushed = 0;
        while push_frame_mark(&mut p, pushed).is_ok() {
            pushed += 1;
        }
        assert!(push_frame_mark(&mut p, 999).is_err());
        // Drain one, push must succeed again.
        let mut scratch = [0u8; MAX_RECORD_LEN];
        c.pop_into(&mut scratch).unwrap().unwrap();
        // One 24-byte slot may not be enough if a pad is required; drain until it fits.
        let mut ok = false;
        for _ in 0..4 {
            if push_frame_mark(&mut p, 999).is_ok() {
                ok = true;
                break;
            }
            c.pop_into(&mut scratch).unwrap().unwrap();
        }
        assert!(ok);
    }

    #[test]
    fn free_running_indices_survive_u32_wrap() {
        let mut r = TestRing::new(1 << 12);
        // Pre-wind the indices to just below u32::MAX so pushes wrap them.
        let start = u32::MAX - 4096;
        let aligned = start & !((1 << 12) - 1); // multiple of ring size, 8-aligned
        *r.prod = aligned;
        *r.cons = aligned;
        let (mut p, mut c) = r.halves();
        let mut scratch = [0u8; MAX_RECORD_LEN];
        for i in 0..500 {
            while push_frame_mark(&mut p, i).is_err() {
                c.pop_into(&mut scratch).unwrap().unwrap();
            }
        }
        // The free-running indices have wrapped past u32::MAX without issue.
        assert!(p.prod.load(Ordering::Relaxed) < aligned);
    }

    #[test]
    fn two_thread_smoke() {
        // 64 KiB ring hammered from a real second thread.
        let mut r = TestRing::new(1 << 16);
        let (mut p, mut c) = r.halves();
        // Miri executes this test too (it is the strongest UB check we have
        // for the raw-pointer paths) — just much slower, so shrink the load.
        const N: u32 = if cfg!(miri) { 300 } else { 200_000 };
        std::thread::scope(|s| {
            s.spawn(move || {
                for i in 0..N {
                    loop {
                        match push_frame_mark(&mut p, i) {
                            Ok(_) => break,
                            Err(RingFull) => std::thread::yield_now(),
                        }
                    }
                }
            });
            let mut scratch = [0u8; MAX_RECORD_LEN];
            let mut seen = 0u32;
            while seen < N {
                match c.pop_into(&mut scratch).unwrap() {
                    None => std::thread::yield_now(),
                    Some(n) => {
                        let hdr = RecordHeader::read_from(&scratch[..n]).unwrap();
                        if hdr.kind == EventKind::Pad as u8 {
                            continue;
                        }
                        let (_, ev) = crate::events::decode_event(&scratch[..n]).unwrap();
                        assert_eq!(ev, EventPayload::FrameMark { frame_index: seen });
                        seen += 1;
                    }
                }
            }
        });
    }
}
