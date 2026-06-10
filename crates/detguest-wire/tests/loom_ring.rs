//! Loom interleaving tests for the SPSC producer/consumer protocol
//! (IMPLEMENTATION-PLAN M0 / risk table: "loopback test runs under miri for
//! the index logic and under loom for the producer/consumer interleavings").
//!
//! Run with: `RUSTFLAGS="--cfg loom" cargo test -p detguest-wire --test loom_ring --release`
//!
//! `ring::Producer`/`Consumer` hold `core::sync::atomic` cells inside mapped
//! channel memory, which loom cannot instrument (loom requires its own atomic
//! types). So this test models the exact same protocol — the release-store of
//! the producer index after slot writes, the acquire-load before slot reads,
//! free-running u32 indices, and the wrap/pad placement math — using loom
//! atomics over per-slot loom `UnsafeCell`s, with all placement decisions
//! delegated to the crate's pure `ring` math functions (`used`, `free`,
//! `contiguous_tail`, `bytes_needed`), so the logic under loom is the logic
//! the real halves execute.
#![cfg(loom)]

use detguest_wire::ring::{bytes_needed, contiguous_tail, free, used};
use loom::cell::UnsafeCell;
use loom::sync::atomic::{AtomicU32, Ordering};
use loom::sync::Arc;
use loom::thread;

const SLOT: usize = 8; // bytes per slot; records are 8-byte aligned
const SLOTS: usize = 8; // 64-byte ring
const SIZE: u32 = (SLOT * SLOTS) as u32;

/// Slot value: `len << 32 | marker`. len/marker == 0 ⇒ pad/continuation slot.
struct ModelRing {
    slots: Vec<UnsafeCell<u64>>,
    prod: AtomicU32,
    cons: AtomicU32,
}

// SAFETY (model): slots are only written by the producer thread in the free
// region and only read by the consumer in the used region; the prod/cons
// release/acquire pairs transfer ownership — exactly the property loom is
// asked to verify (it will report any execution where that does not hold).
unsafe impl Sync for ModelRing {}

impl ModelRing {
    /// `start` pre-winds the free-running indices (must be a multiple of
    /// SIZE so the masked offset starts at slot 0).
    fn new(start: u32) -> ModelRing {
        assert_eq!(start & (SIZE - 1), 0);
        ModelRing {
            slots: (0..SLOTS).map(|_| UnsafeCell::new(0)).collect(),
            prod: AtomicU32::new(start),
            cons: AtomicU32::new(start),
        }
    }

    /// Producer side: mirror of `Producer::try_push` placement.
    fn try_push(&self, len: u32, marker: u64) -> bool {
        let prod = self.prod.load(Ordering::Relaxed); // sole writer
        let cons = self.cons.load(Ordering::Acquire);
        let needed = bytes_needed(prod, SIZE, len);
        if free(prod, cons, SIZE) < needed {
            return false;
        }
        let mut pos = prod;
        if needed > len {
            // Pad the whole tail: first pad slot carries the pad length.
            let tail = contiguous_tail(prod, SIZE);
            self.write_slot(pos, (tail as u64) << 32); // marker 0 = pad
            for off in (SLOT as u32..tail).step_by(SLOT) {
                self.write_slot(pos + off, 0);
            }
            pos = pos.wrapping_add(tail);
        }
        self.write_slot(pos, (len as u64) << 32 | marker);
        for off in (SLOT as u32..len).step_by(SLOT) {
            self.write_slot(pos + off, 0);
        }
        self.prod
            .store(prod.wrapping_add(needed), Ordering::Release);
        true
    }

    /// Consumer side: mirror of `Consumer::pop_into`.
    fn try_pop(&self) -> Option<(u32, u64)> {
        let prod = self.prod.load(Ordering::Acquire);
        let cons = self.cons.load(Ordering::Relaxed); // sole writer
        if used(prod, cons) == 0 {
            return None;
        }
        let first = self.read_slot(cons);
        let len = (first >> 32) as u32;
        let marker = first & 0xFFFF_FFFF;
        assert!(
            len >= SLOT as u32 && len % SLOT as u32 == 0,
            "corrupt len {len:#x}"
        );
        assert!(len <= used(prod, cons), "len exceeds published bytes");
        assert!(
            len <= contiguous_tail(cons, SIZE),
            "record wrapped the ring end"
        );
        for off in (SLOT as u32..len).step_by(SLOT) {
            assert_eq!(self.read_slot(cons + off), 0, "body slots are zero-filled");
        }
        self.cons.store(cons.wrapping_add(len), Ordering::Release);
        Some((len, marker))
    }

    fn write_slot(&self, pos: u32, v: u64) {
        let idx = ((pos & (SIZE - 1)) as usize) / SLOT;
        self.slots[idx].with_mut(|p| unsafe { *p = v });
    }

    fn read_slot(&self, pos: u32) -> u64 {
        let idx = ((pos & (SIZE - 1)) as usize) / SLOT;
        self.slots[idx].with(|p| unsafe { *p })
    }
}

/// Producer pushes 3 records (24 B each — the third forces a tail pad in a
/// 64 B ring); consumer pops concurrently. Loom explores every interleaving;
/// the asserts in `try_pop` check no record is seen before its bytes.
#[test]
fn spsc_interleavings_publish_complete_records() {
    loom::model(|| {
        let ring = Arc::new(ModelRing::new(0));
        let p = Arc::clone(&ring);
        let producer = thread::spawn(move || {
            for marker in 1u64..=3 {
                while !p.try_push(24, marker) {
                    thread::yield_now();
                }
            }
        });
        let mut seen = Vec::new();
        while seen.len() < 3 {
            match ring.try_pop() {
                Some((_len, 0)) => {} // pad
                Some((len, marker)) => {
                    assert_eq!(len, 24);
                    seen.push(marker);
                }
                None => thread::yield_now(),
            }
        }
        producer.join().unwrap();
        assert_eq!(seen, [1, 2, 3], "records arrive in order, exactly once");
    });
}

/// Full-ring path: a producer blocked on a full ring makes progress after the
/// consumer frees space, under every interleaving (the doorbell-retry shape).
#[test]
fn spsc_full_ring_unblocks_after_pop() {
    loom::model(|| {
        let ring = Arc::new(ModelRing::new(0));
        // Fill: 2 × 24 B + 16 B = 64 B exactly. (16 B is the real format's
        // minimum non-pad record; nothing here uses sub-spec 8 B records.)
        assert!(ring.try_push(24, 1));
        assert!(ring.try_push(24, 2));
        assert!(ring.try_push(16, 3));
        assert!(!ring.try_push(16, 4), "ring is exactly full");

        let p = Arc::clone(&ring);
        let producer = thread::spawn(move || {
            while !p.try_push(16, 5) {
                thread::yield_now();
            }
        });
        let (len, marker) = loop {
            if let Some(x) = ring.try_pop() {
                break x;
            }
            thread::yield_now();
        };
        assert_eq!((len, marker), (24, 1));
        producer.join().unwrap();
    });
}

/// Free-running u32 wraparound under concurrency: indices pre-wound to just
/// below `u32::MAX` wrap past it mid-test. The x86-invisible failure mode the
/// research note flags — `used`/`free`/placement must stay correct across the
/// numeric wrap under every interleaving.
#[test]
fn spsc_interleavings_across_u32_wrap() {
    loom::model(|| {
        // Largest multiple of SIZE below u32::MAX: masked offset starts at 0,
        // and the third 24 B record pushes prod numerically past u32::MAX.
        let start = u32::MAX - (SIZE - 1); // 0xFFFF_FFC0 for SIZE=64
        let ring = Arc::new(ModelRing::new(start));
        let p = Arc::clone(&ring);
        let producer = thread::spawn(move || {
            for marker in 1u64..=3 {
                while !p.try_push(24, marker) {
                    thread::yield_now();
                }
            }
        });
        let mut seen = Vec::new();
        while seen.len() < 3 {
            match ring.try_pop() {
                Some((_len, 0)) => {} // pad
                Some((len, marker)) => {
                    assert_eq!(len, 24);
                    seen.push(marker);
                }
                None => thread::yield_now(),
            }
        }
        producer.join().unwrap();
        assert_eq!(seen, [1, 2, 3]);
        assert!(
            ring.prod.load(Ordering::Relaxed) < start,
            "prod index must have wrapped past u32::MAX"
        );
    });
}
