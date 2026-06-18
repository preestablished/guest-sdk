use std::{fs::OpenOptions, io, os::unix::fs::OpenOptionsExt, ptr::NonNull};

use crate::InitError;
#[cfg(not(test))]
use detguest_wire::ports::{DOORBELL_RING_W, PORT_DOORBELL};

const PV_PAD_BASE: libc::off_t = 0xD000_1000;
const PV_PAD_SIZE: usize = 0x1000;
#[cfg(test)]
pub(crate) const PV_PAD_WORDS: usize = PV_PAD_SIZE / std::mem::size_of::<u32>();
#[cfg(test)]
pub(crate) const PVPAD_PAD0_WORD: usize = 0x08 / std::mem::size_of::<u32>();
#[cfg(test)]
pub(crate) const PVPAD_FRAME_COUNTER_WORD: usize = 0x18 / std::mem::size_of::<u32>();

const PVPAD_PAD0_OFF: usize = 0x08;
const PVPAD_FRAME_COUNTER_OFF: usize = 0x18;
const PVPAD_PAD_COUNT: u8 = 4;

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

    #[cfg(test)]
    pub(crate) fn for_test_with_pvpad(words: &'static mut [u32; PV_PAD_WORDS]) -> PioState {
        PioState {
            _pv_pad: Some(MappedMmio {
                ptr: NonNull::new(words.as_mut_ptr().cast::<u8>())
                    .expect("test pv-pad pointer is non-null"),
                len: PV_PAD_SIZE,
                unmap_on_drop: false,
            }),
        }
    }

    pub(crate) fn poll_input(&self, port: u8) -> u32 {
        if port >= PVPAD_PAD_COUNT {
            return 0;
        }
        let Some(pv_pad) = &self._pv_pad else {
            return 0;
        };
        pv_pad.read_u32(PVPAD_PAD0_OFF + port as usize * std::mem::size_of::<u32>())
    }

    pub(crate) fn write_frame_counter(&self, frame_index: u32) {
        if let Some(pv_pad) = &self._pv_pad {
            pv_pad.write_u32(PVPAD_FRAME_COUNTER_OFF, frame_index);
        }
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

#[cfg(test)]
pub(crate) fn doorbell_w() -> Result<(), InitError> {
    Ok(())
}

#[cfg(not(test))]
pub(crate) fn doorbell_w() -> Result<(), InitError> {
    detcall_out(PORT_DOORBELL, DOORBELL_RING_W)
}

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
        unmap_on_drop: true,
    })
}

#[cfg(all(not(test), target_arch = "x86_64"))]
fn detcall_out(port: u16, value: u32) -> Result<(), InitError> {
    unsafe {
        core::arch::asm!(
            "out dx, eax",
            in("dx") port,
            in("eax") value,
            options(nostack, preserves_flags)
        );
    }
    Ok(())
}

#[cfg(all(not(test), target_arch = "x86"))]
fn detcall_out(port: u16, value: u32) -> Result<(), InitError> {
    unsafe {
        core::arch::asm!(
            "out dx, eax",
            in("dx") port,
            in("eax") value,
            options(nostack, preserves_flags)
        );
    }
    Ok(())
}

#[cfg(all(not(test), not(any(target_arch = "x86", target_arch = "x86_64"))))]
fn detcall_out(_port: u16, _value: u32) -> Result<(), InitError> {
    Err(InitError::PioPermissionDenied)
}

#[derive(Debug)]
struct MappedMmio {
    ptr: NonNull<u8>,
    len: usize,
    unmap_on_drop: bool,
}

impl MappedMmio {
    fn read_u32(&self, offset: usize) -> u32 {
        if offset + std::mem::size_of::<u32>() > self.len {
            return 0;
        }
        unsafe { std::ptr::read_volatile(self.ptr.as_ptr().add(offset).cast::<u32>()) }
    }

    fn write_u32(&self, offset: usize, value: u32) {
        if offset + std::mem::size_of::<u32>() > self.len {
            return;
        }
        unsafe {
            std::ptr::write_volatile(self.ptr.as_ptr().add(offset).cast::<u32>(), value);
        }
    }
}

impl Drop for MappedMmio {
    fn drop(&mut self) {
        if self.unmap_on_drop {
            unsafe {
                libc::munmap(self.ptr.as_ptr().cast(), self.len);
            }
        }
    }
}

unsafe impl Send for MappedMmio {}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_words() -> &'static mut [u32; PV_PAD_WORDS] {
        Box::leak(Box::new([0; PV_PAD_WORDS]))
    }

    #[test]
    fn poll_input_reads_current_latch_value() {
        let words = test_words();
        let ptr = words.as_mut_ptr();
        words[PVPAD_PAD0_WORD + 2] = 0xAABB_CCDD;
        let pio = PioState::for_test_with_pvpad(words);

        assert_eq!(pio.poll_input(2), 0xAABB_CCDD);

        unsafe {
            ptr.add(PVPAD_PAD0_WORD + 2).write(0x1122_3344);
        }
        assert_eq!(pio.poll_input(2), 0x1122_3344);
        assert_eq!(pio.poll_input(4), 0);
    }

    #[test]
    fn write_frame_counter_updates_latch() {
        let words = test_words();
        let ptr = words.as_mut_ptr();
        let pio = PioState::for_test_with_pvpad(words);

        pio.write_frame_counter(17);

        let frame = unsafe { ptr.add(PVPAD_FRAME_COUNTER_WORD).read() };
        assert_eq!(frame, 17);
    }
}
