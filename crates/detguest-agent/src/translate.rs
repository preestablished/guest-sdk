//! GVA→GPA self-translation via `/proc/self/pagemap` (ARCHITECTURE.md §4
//! step 4, §5 step 2).
//!
//! No unsafe needed: pagemap is plain file I/O. Each 8-byte entry: bit 63 =
//! present, bit 62 = swapped, bits 0–54 = PFN. Guest PFN ⇒ GPA is
//! `pfn << 12` — the guest kernel's physical address space IS the GPA space
//! the hypervisor exposes (identity by construction of the VM memory map).

use std::fs::File;
use std::io;
use std::os::unix::fs::FileExt;

/// Pagemap entry flags/fields.
const PM_PRESENT: u64 = 1 << 63;
const PM_SWAPPED: u64 = 1 << 62;
const PM_PFN_MASK: u64 = (1 << 55) - 1;

/// Why a translation failed.
#[derive(Debug, PartialEq, Eq)]
pub enum TranslateError {
    /// Page not present (not faulted in / reclaimed) — registration must
    /// fail loudly per API.md §1.5 (`NotPinned`).
    NotPresent {
        /// The failing virtual address.
        vaddr: u64,
    },
    /// Page is in swap — must be impossible in the swapless image.
    Swapped {
        /// The failing virtual address.
        vaddr: u64,
    },
    /// Kernel hid the PFN (no CAP_SYS_ADMIN) — fail loud at startup.
    PfnHidden {
        /// The failing virtual address.
        vaddr: u64,
    },
    /// pagemap I/O error.
    Io(io::ErrorKind),
}

impl From<io::Error> for TranslateError {
    fn from(e: io::Error) -> TranslateError {
        TranslateError::Io(e.kind())
    }
}

/// Open `/proc/self/pagemap` once; reuse for all translations.
pub fn open_pagemap() -> io::Result<File> {
    File::open("/proc/self/pagemap")
}

/// Decode one raw pagemap entry for `vaddr` (pure; unit-tested).
pub fn decode_entry(vaddr: u64, entry: u64) -> Result<u64, TranslateError> {
    if entry & PM_SWAPPED != 0 {
        return Err(TranslateError::Swapped { vaddr });
    }
    if entry & PM_PRESENT == 0 {
        return Err(TranslateError::NotPresent { vaddr });
    }
    let pfn = entry & PM_PFN_MASK;
    if pfn == 0 {
        // Present but PFN masked: pagemap hides PFNs without CAP_SYS_ADMIN.
        return Err(TranslateError::PfnHidden { vaddr });
    }
    Ok(pfn << 12)
}

/// Translate one 4 KiB-aligned GVA to its GPA.
pub fn gva_to_gpa(pagemap: &File, vaddr: u64) -> Result<u64, TranslateError> {
    let mut buf = [0u8; 8];
    pagemap.read_exact_at(&mut buf, (vaddr / 4096) * 8)?;
    let entry = u64::from_le_bytes(buf);
    let base = decode_entry(vaddr & !0xFFF, entry)?;
    Ok(base + (vaddr & 0xFFF))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_entry_cases() {
        // present, pfn 0x1234
        assert_eq!(
            decode_entry(0x1000, PM_PRESENT | 0x1234).unwrap(),
            0x1234 << 12
        );
        // not present
        assert_eq!(
            decode_entry(0x1000, 0),
            Err(TranslateError::NotPresent { vaddr: 0x1000 })
        );
        // swapped wins over present bit checks
        assert_eq!(
            decode_entry(0x1000, PM_SWAPPED | 1),
            Err(TranslateError::Swapped { vaddr: 0x1000 })
        );
        // PFN hidden (present, pfn 0)
        assert_eq!(
            decode_entry(0x1000, PM_PRESENT),
            Err(TranslateError::PfnHidden { vaddr: 0x1000 })
        );
    }

    #[test]
    fn live_translation_smoke() {
        // On a host with CAP_SYS_ADMIN this resolves; without it, pagemap
        // hides PFNs — accept PfnHidden as the documented behavior. Either
        // way the plumbing (offset math, read_exact_at) is exercised.
        let pagemap = match open_pagemap() {
            Ok(f) => f,
            Err(_) => return, // /proc not available (containers)
        };
        let page = vec![0xAAu8; 4096];
        let vaddr = page.as_ptr() as u64;
        match gva_to_gpa(&pagemap, vaddr) {
            Ok(gpa) => assert_eq!(gpa & 0xFFF, vaddr & 0xFFF),
            Err(TranslateError::PfnHidden { .. }) => {}
            Err(TranslateError::NotPresent { .. }) => {} // page may be lazy
            Err(e) => panic!("unexpected: {e:?}"),
        }
    }
}
