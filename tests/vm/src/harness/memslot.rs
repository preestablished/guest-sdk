//! `GuestMem` over the harness's KVM memslot mapping (bead o3i).
//!
//! `detguest-host`'s `Channel` reads/writes guest physical memory through
//! this — the same role the hypervisor's memslot view plays in production.
//! Raw-pointer copies (no long-lived slices) keep the aliasing story simple
//! while the guest concurrently runs: the host only touches channel memory
//! while the vCPU is paused in a VM exit (the load-bearing invariant from
//! ARCHITECTURE.md §2), which is exactly when these methods get called.

use detguest_host::{GuestMem, MemError};

/// A borrowed view of one guest RAM slot (host virtual base + length).
#[derive(Clone, Copy)]
pub struct MemSlot {
    base: *mut u8,
    len: usize,
}

// The harness is single-threaded (vCPU runs on the same thread that
// services exits); Send keeps composition options open.
unsafe impl Send for MemSlot {}

impl MemSlot {
    /// Wrap the slot mapping. `base` must point to `len` bytes of live
    /// guest RAM (owned by the harness's `GuestMemoryMmap`).
    pub(crate) fn new(base: *mut u8, len: usize) -> MemSlot {
        MemSlot { base, len }
    }

    /// Length of the slot in bytes (the guest RAM size).
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    fn range(&self, gpa: u64, n: usize) -> Result<usize, MemError> {
        let off = usize::try_from(gpa).map_err(|_| MemError::Overflow)?;
        if off.checked_add(n).is_none() || off + n > self.len {
            return Err(MemError::Unmapped { gpa, len: n });
        }
        Ok(off)
    }
}

impl GuestMem for MemSlot {
    fn read(&self, gpa: u64, buf: &mut [u8]) -> Result<(), MemError> {
        let off = self.range(gpa, buf.len())?;
        // SAFETY: in-bounds of the live mapping; called only while the vCPU
        // is paused in an exit (see module docs).
        unsafe {
            std::ptr::copy_nonoverlapping(self.base.add(off), buf.as_mut_ptr(), buf.len());
        }
        Ok(())
    }

    fn write(&mut self, gpa: u64, buf: &[u8]) -> Result<(), MemError> {
        let off = self.range(gpa, buf.len())?;
        // SAFETY: as above.
        unsafe {
            std::ptr::copy_nonoverlapping(buf.as_ptr(), self.base.add(off), buf.len());
        }
        Ok(())
    }
}
