use std::{
    ffi::OsStr,
    fmt, io,
    os::unix::{ffi::OsStrExt, io::RawFd},
    ptr::NonNull,
    sync::atomic::{AtomicU32, Ordering},
};

use detguest_wire::{
    header::{
        ChannelHeader, RingId, CHANNEL_MAGIC, CHANNEL_SIZE, FLAG_WORKLOAD_ATTACHED,
        OFF_HEADER_FLAGS, PROTO_VERSION,
    },
    ring::{Consumer, Producer},
    DecodeError,
};

use crate::InitError;

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
    _ring_w: Producer<'static>,
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

        let producer_w = unsafe {
            Producer::from_raw(
                ptr.add(ring_w.offset as usize),
                ring_w.size,
                ptr.add(RingId::W.prod_offset()).cast::<u32>(),
                ptr.add(RingId::W.cons_offset()).cast::<u32>(),
                0,
            )
        };
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
            _ring_w: producer_w,
            _ring_i: consumer_i,
        })
    }

    pub(crate) fn mark_workload_attached(&self) {
        set_workload_attached(self.page.ptr.as_ptr());
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
