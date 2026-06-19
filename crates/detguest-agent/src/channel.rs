//! The agent's side of the detchannel: hugetlbfs allocation, header init,
//! ring A producer / ring C consumer, event emission policy, quiesce relay
//! (ARCHITECTURE.md §2–§4).
//!
//! Permitted-unsafe module: mmaps the shared 2 MiB channel page and builds
//! the `wire::ring` halves over it. Everything after construction goes
//! through the safe ring API; the doorbell is injected as a function so the
//! emission policy is host-testable without hardware port I/O.
#![allow(unsafe_code)]

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

use detguest_wire::events::{encode_event, encoded_event_len, EventPayload};
use detguest_wire::header::{
    self, ChannelHeader, CHANNEL_SIZE, FLAG_AGENT_READY, OFF_MANIFEST, OFF_RESERVED,
};
use detguest_wire::manifest::{init_manifest, read_generation, MANIFEST_TOTAL_SIZE};
use detguest_wire::record::EventKind;
use detguest_wire::ring::{self, Consumer, Producer, RingFull};
use detguest_wire::{ports, RingId};

/// Default hugetlbfs path for the channel file.
pub const CHANNEL_PATH: &str = "/dev/hugepages/detchannel";

/// The agent's channel handle. The mapping lives for the process lifetime
/// (never unmapped — the workload inherits the fd and maps it again).
pub struct AgentChannel {
    fd: OwnedFd,
    base: *mut u8,
    prod_a: Producer<'static>,
    cons_c: Consumer<'static>,
    doorbell: fn(u32),
    scratch: Box<[u8; detguest_wire::MAX_RECORD_LEN]>,
}

#[cfg(test)]
pub(crate) fn test_channel(doorbell: fn(u32)) -> AgentChannel {
    let page: &'static mut [u8] = Box::leak(vec![0u8; CHANNEL_SIZE].into_boxed_slice());
    let fd = std::fs::File::open("/dev/null").unwrap();
    // SAFETY: leaked zeroed CHANNEL_SIZE buffer, exclusively owned by the test.
    unsafe { AgentChannel::init_at(OwnedFd::from(fd), page.as_mut_ptr(), doorbell) }
}

// Single-threaded agent; the handle never crosses threads, but Send keeps
// composition options open (same argument as the ring halves).
unsafe impl Send for AgentChannel {}

impl AgentChannel {
    /// Allocate the channel on hugetlbfs (ARCHITECTURE.md §4 step 3): open,
    /// ftruncate to 2 MiB, map shared, initialize header + manifest.
    /// `doorbell` is [`crate::pio::doorbell`] in production.
    pub fn alloc(doorbell: fn(u32)) -> io::Result<AgentChannel> {
        let path = std::ffi::CString::new(CHANNEL_PATH).unwrap();
        // SAFETY: plain libc open/ftruncate/mmap of a fresh hugetlbfs file.
        // O_EXCL guards the zero-fill invariant init_at relies on: ftruncate
        // does NOT zero an existing file's contents. A leftover file (there
        // should never be one — PID 1 never restarts) is unlinked first.
        let fd = unsafe {
            let mut raw = libc::open(
                path.as_ptr(),
                libc::O_CREAT | libc::O_EXCL | libc::O_RDWR,
                0o700,
            );
            if raw < 0 && io::Error::last_os_error().raw_os_error() == Some(libc::EEXIST) {
                libc::unlink(path.as_ptr());
                raw = libc::open(
                    path.as_ptr(),
                    libc::O_CREAT | libc::O_EXCL | libc::O_RDWR,
                    0o700,
                );
            }
            if raw < 0 {
                return Err(io::Error::last_os_error());
            }
            let fd = OwnedFd::from_raw_fd(raw);
            if libc::ftruncate(fd.as_raw_fd(), CHANNEL_SIZE as i64) != 0 {
                return Err(io::Error::last_os_error());
            }
            fd
        };
        // SAFETY: mapping the whole file shared; hugetlbfs guarantees one
        // 2 MiB physical page, unswappable and unmigratable by construction.
        let base = unsafe {
            let p = libc::mmap(
                std::ptr::null_mut(),
                CHANNEL_SIZE,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd.as_raw_fd(),
                0,
            );
            if p == libc::MAP_FAILED {
                return Err(io::Error::last_os_error());
            }
            p as *mut u8
        };
        // SAFETY: base points to CHANNEL_SIZE zeroed bytes we exclusively own
        // until CHANNEL_INIT hands the host its halves.
        Ok(unsafe { Self::init_at(fd, base, doorbell) })
    }

    /// Build a channel over caller-provided memory (tests use a leaked Vec;
    /// production uses the hugetlbfs mapping).
    ///
    /// # Safety
    /// `base` must point to [`CHANNEL_SIZE`] writable bytes, zero-initialized,
    /// valid for `'static`, exclusively owned by the caller at this point.
    pub unsafe fn init_at(fd: OwnedFd, base: *mut u8, doorbell: fn(u32)) -> AgentChannel {
        // Header + manifest (ARCHITECTURE.md §4 step 3 "write ChannelHeader").
        let mut hdr = [0u8; OFF_RESERVED];
        ChannelHeader::canonical().write_to(&mut hdr).unwrap();
        std::ptr::copy_nonoverlapping(hdr.as_ptr(), base, hdr.len());
        let mut area = vec![0u8; MANIFEST_TOTAL_SIZE];
        init_manifest(&mut area).unwrap();
        std::ptr::copy_nonoverlapping(area.as_ptr(), base.add(OFF_MANIFEST), area.len());

        let a = RingId::A.canonical_desc();
        let c = RingId::C.canonical_desc();
        let prod_a = Producer::from_raw(
            base.add(a.offset as usize),
            a.size,
            base.add(RingId::A.prod_offset()) as *mut u32,
            base.add(RingId::A.cons_offset()) as *mut u32,
            0,
        );
        let cons_c = Consumer::from_raw(
            base.add(c.offset as usize),
            c.size,
            base.add(RingId::C.prod_offset()) as *const u32,
            base.add(RingId::C.cons_offset()) as *const u32,
        );
        AgentChannel {
            fd,
            base,
            prod_a,
            cons_c,
            doorbell,
            scratch: Box::new([0u8; detguest_wire::MAX_RECORD_LEN]),
        }
    }

    /// The channel file descriptor (inherited by workloads as
    /// `DETGUEST_CHANNEL_FD`).
    pub fn fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }

    /// The mapped base (for pagemap translation of the channel GPA).
    pub fn base_ptr(&self) -> *const u8 {
        self.base
    }

    pub(crate) fn manifest(&self) -> &[u8] {
        // SAFETY: in-bounds manifest area within the live channel mapping.
        unsafe { std::slice::from_raw_parts(self.base.add(OFF_MANIFEST), MANIFEST_TOTAL_SIZE) }
    }

    #[cfg(test)]
    pub(crate) fn manifest_mut(&mut self) -> &mut [u8] {
        // SAFETY: in-bounds manifest area; tests and the SDK are the only
        // writers after init, and production uses this only for snapshots.
        unsafe { std::slice::from_raw_parts_mut(self.base.add(OFF_MANIFEST), MANIFEST_TOTAL_SIZE) }
    }

    pub(crate) fn copy_manifest_stable(&self) -> Result<Vec<u8>, detguest_wire::DecodeError> {
        let manifest = self.manifest();
        for _ in 0..1024 {
            let before = read_generation(manifest)?;
            if before % 2 != 0 {
                std::hint::spin_loop();
                continue;
            }
            core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
            let copy = manifest.to_vec();
            core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
            let after = read_generation(manifest)?;
            if before == after && after % 2 == 0 {
                return Ok(copy);
            }
        }
        Err(detguest_wire::DecodeError::BadField)
    }

    /// Set `header_flags.agent_ready` (ARCHITECTURE.md §4 step 6).
    pub fn set_agent_ready(&mut self) {
        // SAFETY: in-bounds header field; agent is the only flags writer.
        unsafe {
            let p = self.base.add(header::OFF_HEADER_FLAGS) as *mut u32;
            p.write_volatile(p.read_volatile() | FLAG_AGENT_READY);
        }
    }

    /// Emit one event on ring A, applying the §3 flow-control policy:
    /// critical ⇒ doorbell + retry until it fits; droppable ⇒ on a full ring
    /// bump the ring-A drop counters and skip (no doorbell, no spin).
    /// Returns true if the event landed.
    pub fn emit(&mut self, vnanos: u64, flags: u8, ev: &EventPayload<'_>) -> bool {
        let len = encoded_event_len(ev);
        let kind = event_kind(ev);
        let critical = kind.is_critical();
        loop {
            match self
                .prod_a
                .try_push(len, |buf, seq| encode_event(buf, seq, vnanos, flags, ev))
            {
                Ok(_) => return true,
                Err(RingFull) if critical => {
                    // Deterministic guest-initiated wait: the doorbell exit
                    // makes the host drain + bump the consumer index.
                    (self.doorbell)(ports::DOORBELL_RING_A);
                }
                Err(RingFull) => {
                    self.bump_drop_counters(len as u64);
                    return false;
                }
            }
        }
    }

    /// Emit + doorbell (used for Hello/Ready/WorkloadExited where the spec
    /// calls for an explicit doorbell after the record).
    pub fn emit_with_doorbell(&mut self, vnanos: u64, flags: u8, ev: &EventPayload<'_>) -> bool {
        let landed = self.emit(vnanos, flags, ev);
        (self.doorbell)(ports::DOORBELL_RING_A);
        landed
    }

    fn bump_drop_counters(&mut self, bytes: u64) {
        // SAFETY: in-bounds header counters; the ring-A producer (us) is
        // their only writer (ARCHITECTURE.md §2).
        unsafe {
            let recs = self.base.add(header::OFF_RING_A_DROPPED_RECORDS) as *mut u64;
            recs.write_volatile(recs.read_volatile() + 1);
            let b = self.base.add(header::OFF_RING_A_DROPPED_BYTES) as *mut u64;
            b.write_volatile(b.read_volatile() + bytes);
        }
    }

    /// Poll one command off ring C (ARCHITECTURE.md §4 step 8). Pads are
    /// consumed silently; unknown kinds are skipped (forward compat);
    /// malformed framing stops the poll (host bug — fail loud upstream).
    pub fn poll_command(
        &mut self,
    ) -> Result<Option<detguest_wire::Command>, detguest_wire::DecodeError> {
        loop {
            match self.cons_c.pop_into(&mut self.scratch[..]) {
                Ok(None) => return Ok(None),
                Ok(Some(n)) => {
                    let rec = &self.scratch[..n];
                    if rec[2] == EventKind::Pad as u8 {
                        continue;
                    }
                    match detguest_wire::events::decode_command(rec) {
                        Ok((_, cmd)) => return Ok(Some(cmd)),
                        Err(detguest_wire::DecodeError::UnknownKind(_)) => continue,
                        Err(e) => return Err(e),
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Relay a workload-control record onto ring I (the COOP quiesce relay,
    /// ARCHITECTURE.md §6). Ring I's nominal producer is the host, but host
    /// pushes happen only while the vCPU is paused and this relay runs only
    /// in response to a ring-C command — temporally exclusive, so the agent
    /// may append continuing the same producer index.
    pub fn relay_workload_ctrl(
        &mut self,
        vnanos: u64,
        rec: &detguest_wire::WorkloadCtrl,
    ) -> Result<(), RingFull> {
        let desc = RingId::I.canonical_desc();
        // SAFETY: in-bounds index cells + ring area of the mapped page;
        // see the temporal-exclusivity argument above.
        unsafe {
            let prod_p = self.base.add(RingId::I.prod_offset()) as *mut u32;
            let cons_p = self.base.add(RingId::I.cons_offset()) as *const u32;
            let prod = prod_p.read_volatile();
            let cons = cons_p.read_volatile();
            let total = detguest_wire::record::record_len(8);
            let needed = ring::bytes_needed(prod, desc.size, total as u32);
            if ring::free(prod, cons, desc.size) < needed {
                return Err(RingFull);
            }
            let mask = desc.size - 1;
            let mut pos = prod;
            // Ring I has two temporally-exclusive producers (host + this
            // relay) and no shared seq counter — a spec gap (ARCHITECTURE.md
            // §6 table vs §7 rule 3). Derive seq from the byte position
            // (pos >> 3): deterministic, strictly increasing with prod, and
            // distinct for a tail pad vs the record after it. The v1 SDK
            // consumer matches quiesce tokens, not seq continuity.
            if needed > total as u32 {
                let tail = ring::contiguous_tail(prod, desc.size) as usize;
                let mut pad = vec![0u8; tail];
                detguest_wire::record::encode_pad(&mut pad, tail, pos >> 3).unwrap();
                std::ptr::copy_nonoverlapping(
                    pad.as_ptr(),
                    self.base.add(desc.offset as usize + (pos & mask) as usize),
                    tail,
                );
                pos = pos.wrapping_add(tail as u32);
            }
            let mut buf = [0u8; 32];
            let n = detguest_wire::events::encode_workload_ctrl(&mut buf, pos >> 3, vnanos, rec)
                .expect("fixed-size record");
            std::ptr::copy_nonoverlapping(
                buf.as_ptr(),
                self.base.add(desc.offset as usize + (pos & mask) as usize),
                n,
            );
            // Release-publish (the SDK consumer Acquire-loads this).
            let atomic = core::sync::atomic::AtomicU32::from_ptr(prod_p);
            atomic.store(
                prod.wrapping_add(needed),
                core::sync::atomic::Ordering::Release,
            );
        }
        Ok(())
    }
}

fn event_kind(ev: &EventPayload<'_>) -> EventKind {
    match ev {
        EventPayload::Pad => EventKind::Pad,
        EventPayload::Hello { .. } => EventKind::Hello,
        EventPayload::NameIntern { .. } => EventKind::NameIntern,
        EventPayload::AssertViolation { .. } => EventKind::AssertViolation,
        EventPayload::Reachable { .. } => EventKind::Reachable,
        EventPayload::Beacon { .. } => EventKind::Beacon,
        EventPayload::InjectQuery { .. } => EventKind::InjectQuery,
        EventPayload::RegionRegister(_) => EventKind::RegionRegister,
        EventPayload::RegionUpdate(_) => EventKind::RegionUpdate,
        EventPayload::WorkloadStarted { .. } => EventKind::WorkloadStarted,
        EventPayload::WorkloadExited { .. } => EventKind::WorkloadExited,
        EventPayload::LogLine { .. } => EventKind::LogLine,
        EventPayload::QuiesceReady { .. } => EventKind::QuiesceReady,
        EventPayload::FrameMark { .. } => EventKind::FrameMark,
        EventPayload::Ready { .. } => EventKind::Ready,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use detguest_wire::events::{decode_event, Command};
    use std::sync::atomic::{AtomicU32, Ordering};

    static DOORBELLS: AtomicU32 = AtomicU32::new(0);
    fn test_doorbell(_mask: u32) {
        DOORBELLS.fetch_add(1, Ordering::Relaxed);
    }

    #[test]
    fn header_and_manifest_initialized() {
        let ch = test_channel(test_doorbell);
        let hdr_bytes =
            // SAFETY: reading our own initialized buffer.
            unsafe { std::slice::from_raw_parts(ch.base_ptr(), OFF_RESERVED) };
        let hdr = ChannelHeader::read_from(hdr_bytes).unwrap();
        hdr.validate().unwrap();
        assert_eq!(hdr.header_flags, 0);
    }

    #[test]
    fn agent_ready_flag_sets() {
        let mut ch = test_channel(test_doorbell);
        ch.set_agent_ready();
        let hdr_bytes = unsafe { std::slice::from_raw_parts(ch.base_ptr(), OFF_RESERVED) };
        let hdr = ChannelHeader::read_from(hdr_bytes).unwrap();
        assert_eq!(hdr.header_flags & FLAG_AGENT_READY, FLAG_AGENT_READY);
    }

    #[test]
    fn emit_writes_ring_a_and_hello_doorbells() {
        let mut ch = test_channel(test_doorbell);
        let before = DOORBELLS.load(Ordering::Relaxed);
        assert!(ch.emit_with_doorbell(
            7,
            0,
            &EventPayload::Hello {
                proto_version: 1,
                agent_version: 0x100,
                capabilities: 3
            },
        ));
        assert!(DOORBELLS.load(Ordering::Relaxed) > before);
        // The record is on ring A at offset 0.
        let a = RingId::A.canonical_desc();
        let rec = unsafe { std::slice::from_raw_parts(ch.base_ptr().add(a.offset as usize), 32) };
        let (hdr, ev) = decode_event(rec).unwrap();
        assert_eq!(hdr.seq, 0);
        assert!(matches!(
            ev,
            EventPayload::Hello {
                proto_version: 1,
                ..
            }
        ));
    }

    #[test]
    fn droppable_overflow_bumps_counters_not_doorbell() {
        let mut ch = test_channel(test_doorbell);
        let msg = vec![b'x'; 1000];
        let mut dropped = 0u64;
        // Ring A is 64 KiB; ~64 of these fit. Push until drops happen.
        for _ in 0..200 {
            if !ch.emit(
                1,
                0,
                &EventPayload::LogLine {
                    stream: 3,
                    level: 4,
                    msg: &msg,
                },
            ) {
                dropped += 1;
            }
        }
        assert!(dropped > 0);
        let recs = unsafe {
            (ch.base_ptr().add(header::OFF_RING_A_DROPPED_RECORDS) as *const u64).read_volatile()
        };
        assert_eq!(recs, dropped);
    }

    #[test]
    fn poll_command_reads_host_pushes() {
        let mut ch = test_channel(test_doorbell);
        // Simulate a host push on ring C: encode at offset 0, bump prod.
        let c = RingId::C.canonical_desc();
        let mut buf = [0u8; 32];
        let n =
            detguest_wire::events::encode_command(&mut buf, 0, &Command::SetLogMask { mask: 0x3 })
                .unwrap();
        unsafe {
            std::ptr::copy_nonoverlapping(buf.as_ptr(), ch.base.add(c.offset as usize), n);
            let prod = ch.base.add(RingId::C.prod_offset()) as *mut u32;
            core::sync::atomic::AtomicU32::from_ptr(prod)
                .store(n as u32, core::sync::atomic::Ordering::Release);
        }
        assert_eq!(
            ch.poll_command().unwrap(),
            Some(Command::SetLogMask { mask: 0x3 })
        );
        assert_eq!(ch.poll_command().unwrap(), None);
    }

    #[test]
    fn relay_quiesce_req_lands_on_ring_i() {
        let mut ch = test_channel(test_doorbell);
        ch.relay_workload_ctrl(5, &detguest_wire::WorkloadCtrl::QuiesceReq { token: 9 })
            .unwrap();
        let i = RingId::I.canonical_desc();
        let rec = unsafe { std::slice::from_raw_parts(ch.base_ptr().add(i.offset as usize), 24) };
        let (_, back) = detguest_wire::events::decode_workload_ctrl(rec).unwrap();
        assert_eq!(back, detguest_wire::WorkloadCtrl::QuiesceReq { token: 9 });
        let prod =
            unsafe { (ch.base_ptr().add(RingId::I.prod_offset()) as *const u32).read_volatile() };
        assert_eq!(prod, 24);
    }
}
