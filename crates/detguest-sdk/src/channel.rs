use std::{
    ffi::OsStr,
    fmt, io,
    os::unix::{ffi::OsStrExt, io::RawFd},
    ptr::NonNull,
    sync::atomic::{AtomicU32, Ordering},
};

use detguest_wire::{
    events::{encode_event, encoded_event_len, EventPayload},
    header::{
        ChannelHeader, RingId, CHANNEL_MAGIC, CHANNEL_SIZE, FLAG_WORKLOAD_ATTACHED,
        OFF_HEADER_FLAGS, OFF_RING_W_DROPPED_BYTES, OFF_RING_W_DROPPED_BY_KIND,
        OFF_RING_W_DROPPED_RECORDS, PROTO_VERSION,
    },
    record::{EventKind, MAX_RECORD_LEN},
    ring::{Consumer, Producer, RingFull},
    DecodeError,
};

use crate::{pio, InitError};

pub(crate) const DETGUEST_CHANNEL_FD_ENV: &str = "DETGUEST_CHANNEL_FD";
const DETGUEST_STANDALONE_PANIC_ENV: &str = "DETGUEST_STANDALONE_PANIC";

pub(crate) fn parse_channel_fd(raw: &OsStr) -> io::Result<RawFd> {
    let raw = std::str::from_utf8(raw.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "channel fd is not UTF-8"))?;
    raw.parse::<RawFd>()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "channel fd is not an integer"))
}

pub(crate) fn standalone_panic_enabled() -> bool {
    std::env::var_os(DETGUEST_STANDALONE_PANIC_ENV)
        .as_deref()
        .is_some_and(|v| v == OsStr::new("1"))
}

#[derive(Debug)]
struct MappedPage {
    ptr: NonNull<u8>,
    len: usize,
}

impl MappedPage {
    fn map(fd: RawFd) -> Result<MappedPage, InitError> {
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                CHANNEL_SIZE,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            return Err(InitError::AgentSocket(io::Error::last_os_error()));
        }
        Ok(MappedPage {
            ptr: NonNull::new(ptr.cast::<u8>()).expect("mmap never returns null on success"),
            len: CHANNEL_SIZE,
        })
    }

    fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}

impl Drop for MappedPage {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.ptr.as_ptr().cast(), self.len);
        }
    }
}

unsafe impl Send for MappedPage {}

/// Mapped detchannel plus the workload-owned ring halves.
pub(crate) struct MappedChannel {
    page: MappedPage,
    ring_w: RingW,
    _ring_i: Consumer<'static>,
}

unsafe impl Send for MappedChannel {}

impl fmt::Debug for MappedChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MappedChannel")
            .field("ptr", &self.page.ptr)
            .field("len", &self.page.len)
            .finish_non_exhaustive()
    }
}

impl MappedChannel {
    pub(crate) fn map(fd: RawFd) -> Result<MappedChannel, InitError> {
        let page = MappedPage::map(fd)?;
        let header = read_and_validate_header(page.bytes())?;
        let ptr = page.ptr.as_ptr();
        let ring_w = header.ring_desc[RingId::W as usize];
        let ring_i = header.ring_desc[RingId::I as usize];

        let producer_w = unsafe { RingW::from_raw(ptr, ring_w, 0) };
        let consumer_i = unsafe {
            Consumer::from_raw(
                ptr.add(ring_i.offset as usize),
                ring_i.size,
                ptr.add(RingId::I.prod_offset()).cast::<u32>(),
                ptr.add(RingId::I.cons_offset()).cast::<u32>(),
            )
        };
        Ok(MappedChannel {
            page,
            ring_w: producer_w,
            _ring_i: consumer_i,
        })
    }

    pub(crate) fn mark_workload_attached(&self) {
        set_workload_attached(self.page.ptr.as_ptr());
    }

    pub(crate) fn emit_w_event(
        &mut self,
        vnanos: u64,
        extra_flags: u8,
        ev: &EventPayload<'_>,
        class: EventClass,
    ) -> Result<(), InitError> {
        self.ring_w
            .emit(vnanos, extra_flags, ev, class, pio::doorbell_w)
    }
}

/// Ring-W event criticality.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EventClass {
    /// Doorbell and retry until the event is published.
    #[allow(dead_code)]
    Critical,
    /// Do not block; account the dropped record in channel header counters.
    Droppable,
}

struct RingW {
    producer: Producer<'static>,
    drops: RingWDropCounters,
}

unsafe impl Send for RingW {}

impl RingW {
    unsafe fn from_raw(
        channel_ptr: *mut u8,
        desc: detguest_wire::header::RingDesc,
        next_seq: u32,
    ) -> RingW {
        RingW {
            producer: Producer::from_raw(
                channel_ptr.add(desc.offset as usize),
                desc.size,
                channel_ptr.add(RingId::W.prod_offset()).cast::<u32>(),
                channel_ptr.add(RingId::W.cons_offset()).cast::<u32>(),
                next_seq,
            ),
            drops: RingWDropCounters {
                channel: NonNull::new(channel_ptr).expect("channel pointer is non-null"),
            },
        }
    }

    fn emit(
        &mut self,
        vnanos: u64,
        extra_flags: u8,
        ev: &EventPayload<'_>,
        class: EventClass,
        mut doorbell_w: impl FnMut() -> Result<(), InitError>,
    ) -> Result<(), InitError> {
        let len = encoded_event_len(ev);
        if len > MAX_RECORD_LEN {
            return Err(InitError::AgentSocket(io::Error::new(
                io::ErrorKind::InvalidInput,
                "event exceeds max record size",
            )));
        }
        match class {
            EventClass::Droppable => {
                match push_event(&mut self.producer, len, vnanos, extra_flags, ev) {
                    Ok(()) => Ok(()),
                    Err(RingFull) => {
                        self.drops.bump(event_kind(ev), len);
                        Ok(())
                    }
                }
            }
            EventClass::Critical => loop {
                match push_event(&mut self.producer, len, vnanos, extra_flags, ev) {
                    Ok(()) => return Ok(()),
                    Err(RingFull) => doorbell_w()?,
                }
            },
        }
    }
}

fn push_event(
    producer: &mut Producer<'_>,
    len: usize,
    vnanos: u64,
    extra_flags: u8,
    ev: &EventPayload<'_>,
) -> Result<(), RingFull> {
    producer
        .try_push(len, |buf, seq| {
            encode_event(buf, seq, vnanos, extra_flags, ev)
        })
        .map(|_| ())
}

struct RingWDropCounters {
    channel: NonNull<u8>,
}

unsafe impl Send for RingWDropCounters {}

impl RingWDropCounters {
    fn bump(&self, kind: EventKind, bytes: usize) {
        fetch_add_u64(self.channel, OFF_RING_W_DROPPED_RECORDS, 1);
        fetch_add_u64(self.channel, OFF_RING_W_DROPPED_BYTES, bytes as u64);
        let by_kind = OFF_RING_W_DROPPED_BY_KIND + kind as usize * std::mem::size_of::<u64>();
        fetch_add_u64(self.channel, by_kind, 1);
    }
}

fn fetch_add_u64(channel: NonNull<u8>, offset: usize, value: u64) {
    let counter =
        unsafe { std::sync::atomic::AtomicU64::from_ptr(channel.as_ptr().add(offset).cast()) };
    counter.fetch_add(value, Ordering::Relaxed);
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

fn read_and_validate_header(bytes: &[u8]) -> Result<ChannelHeader, InitError> {
    let header = ChannelHeader::read_from(bytes).map_err(map_decode_error)?;
    if header.magic != CHANNEL_MAGIC {
        return Err(InitError::BadChannelHeader {
            found_magic: header.magic,
        });
    }
    if header.proto_version != PROTO_VERSION {
        return Err(InitError::ProtocolVersionMismatch {
            guest: PROTO_VERSION,
            channel: header.proto_version,
        });
    }
    header.validate().map_err(map_decode_error)?;
    Ok(header)
}

fn map_decode_error(err: DecodeError) -> InitError {
    InitError::AgentSocket(io::Error::new(
        io::ErrorKind::InvalidData,
        format!("invalid detguest channel header: {err:?}"),
    ))
}

fn set_workload_attached(ptr: *mut u8) {
    let flags = unsafe { AtomicU32::from_ptr(ptr.add(OFF_HEADER_FLAGS).cast::<u32>()) };
    flags.fetch_or(FLAG_WORKLOAD_ATTACHED, Ordering::AcqRel);
}

#[cfg(test)]
mod tests {
    use super::*;
    use detguest_wire::{
        events::decode_event,
        header::{OFF_RESERVED, OFF_RING_W_CONS, OFF_RING_W_DATA, OFF_RING_W_PROD},
    };

    fn test_page() -> &'static mut [u8] {
        let mut page = vec![0u8; CHANNEL_SIZE].into_boxed_slice();
        ChannelHeader::canonical()
            .write_to(&mut page[..OFF_RESERVED])
            .unwrap();
        Box::leak(page)
    }

    fn test_ring_w(ptr: *mut u8) -> RingW {
        unsafe { RingW::from_raw(ptr, RingId::W.canonical_desc(), 0) }
    }

    fn atomic_u32(ptr: *mut u8, offset: usize) -> &'static AtomicU32 {
        unsafe { AtomicU32::from_ptr(ptr.add(offset).cast::<u32>()) }
    }

    fn atomic_u64(ptr: *mut u8, offset: usize) -> &'static std::sync::atomic::AtomicU64 {
        unsafe { std::sync::atomic::AtomicU64::from_ptr(ptr.add(offset).cast::<u64>()) }
    }

    fn force_ring_w_full(ptr: *mut u8) {
        let size = RingId::W.canonical_desc().size;
        atomic_u32(ptr, OFF_RING_W_PROD).store(size, Ordering::Release);
        atomic_u32(ptr, OFF_RING_W_CONS).store(0, Ordering::Release);
    }

    fn drain_ring_w(ptr: *mut u8) {
        let prod = atomic_u32(ptr, OFF_RING_W_PROD).load(Ordering::Acquire);
        atomic_u32(ptr, OFF_RING_W_CONS).store(prod, Ordering::Release);
    }

    fn load_counter(ptr: *mut u8, offset: usize) -> u64 {
        atomic_u64(ptr, offset).load(Ordering::Relaxed)
    }

    #[test]
    fn droppable_full_ring_bumps_drop_counters_without_doorbell() {
        let page = test_page();
        let ptr = page.as_mut_ptr();
        let mut ring_w = test_ring_w(ptr);
        force_ring_w_full(ptr);

        let ev = EventPayload::Beacon { beacon_id: 7 };
        let len = encoded_event_len(&ev);
        ring_w
            .emit(0, 0, &ev, EventClass::Droppable, || {
                panic!("droppable event must not doorbell")
            })
            .unwrap();

        assert_eq!(load_counter(ptr, OFF_RING_W_DROPPED_RECORDS), 1);
        assert_eq!(load_counter(ptr, OFF_RING_W_DROPPED_BYTES), len as u64);
        assert_eq!(
            load_counter(
                ptr,
                OFF_RING_W_DROPPED_BY_KIND
                    + EventKind::Beacon as usize * std::mem::size_of::<u64>()
            ),
            1
        );
        assert_eq!(
            load_counter(
                ptr,
                OFF_RING_W_DROPPED_BY_KIND
                    + EventKind::LogLine as usize * std::mem::size_of::<u64>()
            ),
            0
        );
    }

    #[test]
    fn critical_full_ring_doorbells_retries_and_publishes() {
        let page = test_page();
        let ptr = page.as_mut_ptr();
        let mut ring_w = test_ring_w(ptr);
        force_ring_w_full(ptr);

        let mut doorbells = 0;
        let ev = EventPayload::FrameMark { frame_index: 42 };
        let len = encoded_event_len(&ev);
        ring_w
            .emit(123, 0, &ev, EventClass::Critical, || {
                doorbells += 1;
                drain_ring_w(ptr);
                Ok(())
            })
            .unwrap();

        assert_eq!(doorbells, 1);
        assert_eq!(load_counter(ptr, OFF_RING_W_DROPPED_RECORDS), 0);
        assert_eq!(load_counter(ptr, OFF_RING_W_DROPPED_BYTES), 0);

        let record = &page[OFF_RING_W_DATA..OFF_RING_W_DATA + len];
        let (hdr, payload) = decode_event(record).unwrap();
        assert_eq!(hdr.seq, 0);
        assert_eq!(hdr.vnanos, 123);
        assert_eq!(payload, ev);
    }
}
