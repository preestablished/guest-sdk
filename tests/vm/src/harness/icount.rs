//! Guest retired-instruction counting via perf (bead 9bs) — the measurement
//! behind the M2 bit-identical-icount gate (IMPLEMENTATION-PLAN M2
//! acceptance: "measured by the harness's retired-instruction counter").
//!
//! `perf_event_open` on the vCPU thread with `exclude_host` set counts only
//! instructions retired while the CPU is in VMX non-root mode (the guest),
//! using the PMU virtualization KVM provides. Requires
//! `perf_event_paranoid <= 1` (the preflight gate).

use std::io;
use std::os::fd::{FromRawFd, OwnedFd};

const PERF_TYPE_HARDWARE: u32 = 0;
const PERF_COUNT_HW_INSTRUCTIONS: u64 = 1;
/// `perf_event_attr.flags` bit positions (linux/perf_event.h).
const FLAG_DISABLED: u64 = 1; // bit 0
const FLAG_EXCLUDE_HOST: u64 = 1 << 19;
/// PERF_ATTR_SIZE_VER5 — large enough for every field we set.
const ATTR_SIZE: u32 = 112;
/// PERF_FLAG_FD_CLOEXEC (not in the libc crate).
const PERF_FLAG_FD_CLOEXEC: libc::c_ulong = 8;

#[repr(C)]
#[derive(Default)]
struct PerfEventAttr {
    type_: u32,
    size: u32,
    config: u64,
    sample_period_or_freq: u64,
    sample_type: u64,
    read_format: u64,
    flags: u64,
    wakeup: u32,
    bp_type: u32,
    bp_addr_or_config1: u64,
    bp_len_or_config2: u64,
    branch_sample_type: u64,
    sample_regs_user: u64,
    sample_stack_user: u32,
    clockid: i32,
    sample_regs_intr: u64,
    aux_watermark: u32,
    sample_max_stack: u16,
    reserved_2: u16,
}

// The kernel copies `size` bytes from userspace; the struct must really be
// that large or the syscall reads out of bounds (review finding).
const _: () = assert!(std::mem::size_of::<PerfEventAttr>() == ATTR_SIZE as usize);

/// An open guest-only retired-instruction counter on this thread.
pub struct GuestIcount {
    fd: OwnedFd,
}

impl GuestIcount {
    /// Open the counter for the calling thread (the harness runs the vCPU on
    /// the thread that built it). Starts disabled; enable around the region
    /// of interest or just read deltas.
    pub fn open() -> io::Result<GuestIcount> {
        let mut attr = PerfEventAttr {
            type_: PERF_TYPE_HARDWARE,
            size: ATTR_SIZE,
            config: PERF_COUNT_HW_INSTRUCTIONS,
            flags: FLAG_DISABLED | FLAG_EXCLUDE_HOST,
            ..Default::default()
        };
        // SAFETY: syscall with a properly-sized attr struct; pid=0 (this
        // thread), cpu=-1 (any), no group, no flags.
        let fd = unsafe {
            libc::syscall(
                libc::SYS_perf_event_open,
                &mut attr as *mut PerfEventAttr,
                0i32,
                -1i32,
                -1i32,
                PERF_FLAG_FD_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: fresh fd from the kernel.
        let fd = unsafe { OwnedFd::from_raw_fd(fd as i32) };
        Ok(GuestIcount { fd })
    }

    /// Enable counting.
    pub fn enable(&self) -> io::Result<()> {
        self.ioctl(0x2400 /* PERF_EVENT_IOC_ENABLE */)
    }

    /// Disable counting.
    pub fn disable(&self) -> io::Result<()> {
        self.ioctl(0x2401 /* PERF_EVENT_IOC_DISABLE */)
    }

    /// Read the current count.
    pub fn read(&self) -> io::Result<u64> {
        use std::os::fd::AsRawFd;
        let mut v: u64 = 0;
        // SAFETY: reading 8 bytes from a counter fd into a local.
        let n = unsafe {
            libc::read(
                self.fd.as_raw_fd(),
                &mut v as *mut u64 as *mut libc::c_void,
                8,
            )
        };
        if n != 8 {
            return Err(io::Error::last_os_error());
        }
        Ok(v)
    }

    fn ioctl(&self, req: libc::c_ulong) -> io::Result<()> {
        use std::os::fd::AsRawFd;
        // SAFETY: documented perf ioctls with no argument.
        let rc = unsafe { libc::ioctl(self.fd.as_raw_fd(), req, 0) };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}
