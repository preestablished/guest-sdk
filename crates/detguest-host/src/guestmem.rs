//! `GuestMem`: hypervisor-provided access to guest physical memory, plus the
//! `Vec`-backed mock used by host-side tests (API.md §2).

/// Guest memory access failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MemError {
    /// `[gpa, gpa+len)` is not (fully) mapped.
    Unmapped {
        /// Failing guest-physical address.
        gpa: u64,
        /// Access length.
        len: usize,
    },
    /// Arithmetic overflow computing the access range.
    Overflow,
}

/// Hypervisor-provided access to guest physical memory. Implemented by the
/// VMM over its memslot mappings. All offsets are GPAs.
pub trait GuestMem {
    /// Read `buf.len()` bytes at `gpa`.
    fn read(&self, gpa: u64, buf: &mut [u8]) -> Result<(), MemError>;
    /// Write `buf` at `gpa`.
    fn write(&mut self, gpa: u64, buf: &[u8]) -> Result<(), MemError>;
}

/// Convenience reads used throughout the crate.
pub(crate) trait GuestMemExt: GuestMem {
    fn read_u32(&self, gpa: u64) -> Result<u32, MemError> {
        let mut b = [0u8; 4];
        self.read(gpa, &mut b)?;
        Ok(u32::from_le_bytes(b))
    }
    fn read_u64(&self, gpa: u64) -> Result<u64, MemError> {
        let mut b = [0u8; 8];
        self.read(gpa, &mut b)?;
        Ok(u64::from_le_bytes(b))
    }
    fn write_u32(&mut self, gpa: u64, v: u32) -> Result<(), MemError> {
        self.write(gpa, &v.to_le_bytes())
    }
}

impl<M: GuestMem + ?Sized> GuestMemExt for M {}

/// A segmented, `Vec<u8>`-backed mock: a set of non-overlapping GPA ranges.
/// Multiple segments make discontiguous layouts (the 3-extent `read_region`
/// acceptance case) easy to model; a single segment is the common case.
#[derive(Debug, Default)]
pub struct MockGuestMem {
    segments: Vec<Segment>,
}

#[derive(Debug)]
struct Segment {
    base: u64,
    data: Vec<u8>,
}

impl MockGuestMem {
    /// An empty mock (no mapped ranges).
    pub fn new() -> MockGuestMem {
        MockGuestMem::default()
    }

    /// Map `data` at `[base, base + data.len())`. Panics on overlap with an
    /// existing segment (test-construction bug, not a runtime condition).
    pub fn add_segment(&mut self, base: u64, data: Vec<u8>) {
        let end = base
            .checked_add(data.len() as u64)
            .expect("segment end overflows");
        for s in &self.segments {
            let s_end = s
                .base
                .checked_add(s.data.len() as u64)
                .expect("segment end overflows");
            assert!(end <= s.base || base >= s_end, "overlapping mock segments");
        }
        self.segments.push(Segment { base, data });
    }

    /// Single zeroed segment of `len` bytes at `base`.
    pub fn with_zeroed(base: u64, len: usize) -> MockGuestMem {
        let mut m = MockGuestMem::new();
        m.add_segment(base, vec![0u8; len]);
        m
    }

    /// Borrow a mapped range (test assertions).
    pub fn slice(&self, gpa: u64, len: usize) -> Option<&[u8]> {
        let (seg, off) = self.locate(gpa, len)?;
        Some(&self.segments[seg].data[off..off + len])
    }

    fn locate(&self, gpa: u64, len: usize) -> Option<(usize, usize)> {
        let end = gpa.checked_add(len as u64)?;
        for (i, s) in self.segments.iter().enumerate() {
            let s_end = s.base + s.data.len() as u64;
            if gpa >= s.base && end <= s_end {
                return Some((i, (gpa - s.base) as usize));
            }
        }
        None
    }
}

impl GuestMem for MockGuestMem {
    fn read(&self, gpa: u64, buf: &mut [u8]) -> Result<(), MemError> {
        match self.locate(gpa, buf.len()) {
            Some((seg, off)) => {
                buf.copy_from_slice(&self.segments[seg].data[off..off + buf.len()]);
                Ok(())
            }
            None => Err(MemError::Unmapped {
                gpa,
                len: buf.len(),
            }),
        }
    }

    fn write(&mut self, gpa: u64, buf: &[u8]) -> Result<(), MemError> {
        match self.locate(gpa, buf.len()) {
            Some((seg, off)) => {
                self.segments[seg].data[off..off + buf.len()].copy_from_slice(buf);
                Ok(())
            }
            None => Err(MemError::Unmapped {
                gpa,
                len: buf.len(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_reads_and_writes_within_segments() {
        let mut m = MockGuestMem::with_zeroed(0x1000, 0x100);
        m.write(0x1010, &[1, 2, 3]).unwrap();
        let mut b = [0u8; 3];
        m.read(0x1010, &mut b).unwrap();
        assert_eq!(b, [1, 2, 3]);
    }

    #[test]
    fn unmapped_and_straddling_accesses_fail() {
        let mut m = MockGuestMem::with_zeroed(0x1000, 0x100);
        m.add_segment(0x3000, vec![0u8; 0x100]);
        let mut b = [0u8; 4];
        assert!(m.read(0x0, &mut b).is_err());
        // straddles the end of the first segment into a hole
        assert!(m.read(0x10FE, &mut b).is_err());
        // straddling two segments across a hole also fails
        assert!(m.read(0x10FC, &mut [0u8; 64]).is_err());
        assert!(m.write(0x2000, &b).is_err());
    }

    #[test]
    fn overlapping_segments_panic() {
        let r = std::panic::catch_unwind(|| {
            let mut m = MockGuestMem::with_zeroed(0x1000, 0x100);
            m.add_segment(0x1080, vec![0u8; 0x100]);
        });
        assert!(r.is_err());
    }
}
