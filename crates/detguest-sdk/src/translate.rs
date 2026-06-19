//! GVA-to-GPA self-translation for workload-published regions.

use std::fs::File;
use std::io;
use std::os::unix::fs::FileExt;

const PM_PRESENT: u64 = 1 << 63;
const PM_SWAPPED: u64 = 1 << 62;
const PM_PFN_MASK: u64 = (1 << 55) - 1;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum TranslateError {
    NotPresent { vaddr: u64 },
    Swapped { vaddr: u64 },
    PfnHidden { vaddr: u64 },
    Io(io::ErrorKind),
}

impl From<io::Error> for TranslateError {
    fn from(e: io::Error) -> TranslateError {
        TranslateError::Io(e.kind())
    }
}

pub(crate) fn open_pagemap() -> io::Result<File> {
    File::open("/proc/self/pagemap")
}

fn decode_entry(vaddr: u64, entry: u64) -> Result<u64, TranslateError> {
    if entry & PM_SWAPPED != 0 {
        return Err(TranslateError::Swapped { vaddr });
    }
    if entry & PM_PRESENT == 0 {
        return Err(TranslateError::NotPresent { vaddr });
    }
    let pfn = entry & PM_PFN_MASK;
    if pfn == 0 {
        return Err(TranslateError::PfnHidden { vaddr });
    }
    Ok(pfn << 12)
}

pub(crate) fn gva_to_gpa(pagemap: &File, vaddr: u64) -> Result<u64, TranslateError> {
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
        assert_eq!(
            decode_entry(0x1000, PM_PRESENT | 0x1234).unwrap(),
            0x1234 << 12
        );
        assert_eq!(
            decode_entry(0x1000, 0),
            Err(TranslateError::NotPresent { vaddr: 0x1000 })
        );
        assert_eq!(
            decode_entry(0x1000, PM_SWAPPED | 1),
            Err(TranslateError::Swapped { vaddr: 0x1000 })
        );
        assert_eq!(
            decode_entry(0x1000, PM_PRESENT),
            Err(TranslateError::PfnHidden { vaddr: 0x1000 })
        );
    }
}
