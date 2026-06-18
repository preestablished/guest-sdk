use std::{fs::OpenOptions, io, os::unix::fs::OpenOptionsExt, ptr::NonNull};

use crate::InitError;

const PV_PAD_BASE: libc::off_t = 0xD000_1000;
const PV_PAD_SIZE: usize = 0x1000;

/// Process-wide detcall and pv-pad setup.
#[derive(Debug)]
pub(crate) struct PioState {
    _pv_pad: Option<MappedMmio>,
}

impl PioState {
    #[cfg(test)]
    pub(crate) fn for_test() -> PioState {
        PioState { _pv_pad: None }
    }
}

pub(crate) fn init() -> Result<PioState, InitError> {
    raise_iopl()?;
    Ok(PioState {
        _pv_pad: Some(map_pv_pad()?),
    })
}

pub(crate) fn poll_input(port: u8) -> u32 {
    let _ = port;
    0
}

pub(crate) fn frame_mark() {}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn raise_iopl() -> Result<(), InitError> {
    let rc = unsafe { libc::iopl(3) };
    if rc == 0 {
        Ok(())
    } else {
        Err(InitError::PioPermissionDenied)
    }
}

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
fn raise_iopl() -> Result<(), InitError> {
    Err(InitError::PioPermissionDenied)
}

fn map_pv_pad() -> Result<MappedMmio, InitError> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_SYNC)
        .open("/dev/mem")
        .map_err(InitError::AgentSocket)?;
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            PV_PAD_SIZE,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            std::os::fd::AsRawFd::as_raw_fd(&file),
            PV_PAD_BASE,
        )
    };
    if ptr == libc::MAP_FAILED {
        return Err(InitError::AgentSocket(io::Error::last_os_error()));
    }
    Ok(MappedMmio {
        ptr: NonNull::new(ptr.cast::<u8>()).expect("mmap never returns null on success"),
        len: PV_PAD_SIZE,
    })
}

#[derive(Debug)]
struct MappedMmio {
    ptr: NonNull<u8>,
    len: usize,
}

impl Drop for MappedMmio {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.ptr.as_ptr().cast(), self.len);
        }
    }
}

unsafe impl Send for MappedMmio {}
