//! Region manifest: layout, entry/extent codecs, seqlock helpers (API.md §4).
//!
//! The manifest lives in the channel page at [`crate::header::OFF_MANIFEST`]
//! (0x1000). All functions here take the **manifest area slice** (starting at
//! that offset) and use offsets relative to it, so the same code serves the
//! agent (writing into the mapped page) and the host (reading a copied
//! snapshot).
//!
//! Seqlock discipline (API.md §4.2): the agent is the only writer ever —
//! `generation += 1` (odd), full fence, mutate, full fence, `generation += 1`
//! (even). Readers copy while `generation` is even and unchanged across the
//! copy, else retry. The helpers below do the byte-level arithmetic only;
//! fencing is the caller's responsibility because it depends on how the bytes
//! are mapped (in-guest mapping vs. host `GuestMem` reads).

use crate::{DecodeError, EncodeError};

/// Manifest magic `"DTDF"` as a little-endian u32.
pub const MANIFEST_MAGIC: u32 = 0x4644_5444;
/// Manifest format version this crate implements.
pub const MANIFEST_VERSION: u16 = 1;
/// Region slots.
pub const REGION_CAPACITY: usize = 64;
/// Extent pool slots.
pub const EXTENT_CAPACITY: usize = 1024;
/// Hard cap on a region name, bytes (NUL-padded in the entry).
pub const MAX_REGION_NAME: usize = 56;

/// `RegionEntry.flags` bit 31: entry unregistered; slot retained.
pub const REGION_FLAG_DEAD: u32 = 1 << 31;
/// `RegionFlags::FRAMEBUFFER` (API.md §1.5).
pub const REGION_FLAG_FRAMEBUFFER: u32 = 1 << 0;
/// `RegionFlags::HOT`.
pub const REGION_FLAG_HOT: u32 = 1 << 1;
/// `RegionFlags::HOST_WRITABLE` (reserved; v1 host never writes regions).
pub const REGION_FLAG_HOST_WRITABLE: u32 = 1 << 2;

// Offsets relative to the manifest area (absolute = 0x1000 + relative).

/// Header offset.
pub const OFF_HEADER: usize = 0x0;
/// `generation` offset (u64) — the seqlock word.
pub const OFF_GENERATION: usize = 0x8;
/// First region entry offset (absolute 0x1020).
pub const OFF_ENTRIES: usize = 0x20;
/// Region entry stride.
pub const REGION_ENTRY_SIZE: usize = 96;
/// Extent pool offset (absolute 0x2820).
pub const OFF_EXTENTS: usize = OFF_ENTRIES + REGION_CAPACITY * REGION_ENTRY_SIZE;
/// Extent stride.
pub const EXTENT_SIZE: usize = 16;
/// Total manifest bytes (relative; must fit the 28 KiB area).
pub const MANIFEST_TOTAL_SIZE: usize = OFF_EXTENTS + EXTENT_CAPACITY * EXTENT_SIZE;

// API.md §4.1 gives the absolute offsets; pin them here so drift fails compilation.
const _: () = {
    assert!(0x1000 + OFF_ENTRIES == 0x1020);
    assert!(0x1000 + OFF_EXTENTS == 0x2820);
    assert!(0x1000 + MANIFEST_TOTAL_SIZE == 0x6820);
    assert!(MANIFEST_TOTAL_SIZE <= 0x7000); // fits 0x1000..0x8000 with v2 headroom
};

/// Decoded manifest header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManifestHeader {
    /// Must equal [`MANIFEST_MAGIC`].
    pub magic: u32,
    /// Must equal [`MANIFEST_VERSION`].
    pub manifest_version: u16,
    /// Must equal [`REGION_CAPACITY`].
    pub region_capacity: u16,
    /// Seqlock generation: odd while the agent is writing.
    pub generation: u64,
    /// Live entries (dead entries keep their slots).
    pub region_count: u32,
    /// Used slots in the extent pool.
    pub extent_count: u32,
}

impl ManifestHeader {
    /// Parse from the manifest area.
    pub fn read_from(m: &[u8]) -> Result<ManifestHeader, DecodeError> {
        if m.len() < OFF_ENTRIES {
            return Err(DecodeError::Truncated);
        }
        Ok(ManifestHeader {
            magic: u32::from_le_bytes(m[0..4].try_into().unwrap()),
            manifest_version: u16::from_le_bytes(m[4..6].try_into().unwrap()),
            region_capacity: u16::from_le_bytes(m[6..8].try_into().unwrap()),
            generation: u64::from_le_bytes(m[8..16].try_into().unwrap()),
            region_count: u32::from_le_bytes(m[16..20].try_into().unwrap()),
            extent_count: u32::from_le_bytes(m[20..24].try_into().unwrap()),
        })
    }

    /// Serialize into the manifest area.
    pub fn write_to(&self, m: &mut [u8]) -> Result<(), EncodeError> {
        if m.len() < OFF_ENTRIES {
            return Err(EncodeError::BufferTooSmall);
        }
        m[0..4].copy_from_slice(&self.magic.to_le_bytes());
        m[4..6].copy_from_slice(&self.manifest_version.to_le_bytes());
        m[6..8].copy_from_slice(&self.region_capacity.to_le_bytes());
        m[8..16].copy_from_slice(&self.generation.to_le_bytes());
        m[16..20].copy_from_slice(&self.region_count.to_le_bytes());
        m[20..24].copy_from_slice(&self.extent_count.to_le_bytes());
        m[24..32].fill(0);
        Ok(())
    }

    /// Host-side validation: magic, version, capacity, counts within bounds.
    pub fn validate(&self) -> Result<(), DecodeError> {
        if self.magic != MANIFEST_MAGIC {
            return Err(DecodeError::BadMagic);
        }
        if self.manifest_version != MANIFEST_VERSION {
            return Err(DecodeError::BadVersion);
        }
        if self.region_capacity as usize != REGION_CAPACITY
            || self.region_count as usize > REGION_CAPACITY
            || self.extent_count as usize > EXTENT_CAPACITY
        {
            return Err(DecodeError::BadField);
        }
        Ok(())
    }
}

/// One region entry (96 bytes on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionEntry {
    /// Slot index; stable for the life of the channel.
    pub region_id: u32,
    /// Intern id (name also inlined below — self-contained after restore).
    pub name_id: u32,
    /// Workload-declared layout version.
    pub layout_version: u32,
    /// `RegionFlags` bits; bit 31 = DEAD.
    pub flags: u32,
    /// Guest-virtual base (informational/debug).
    pub gva: u64,
    /// Region length in bytes.
    pub len: u64,
    /// Index of the first extent in the pool.
    pub extent_off: u32,
    /// Number of extents.
    pub extent_n: u32,
    /// UTF-8 name, NUL-padded to 56 bytes.
    pub name: [u8; MAX_REGION_NAME],
}

impl RegionEntry {
    /// Byte offset of entry `i` within the manifest area.
    pub const fn offset(i: usize) -> usize {
        OFF_ENTRIES + i * REGION_ENTRY_SIZE
    }

    /// Build the NUL-padded name field; `Err` if over [`MAX_REGION_NAME`].
    pub fn pack_name(name: &[u8]) -> Result<[u8; MAX_REGION_NAME], EncodeError> {
        if name.len() > MAX_REGION_NAME {
            return Err(EncodeError::FieldTooLong);
        }
        let mut out = [0u8; MAX_REGION_NAME];
        out[..name.len()].copy_from_slice(name);
        Ok(out)
    }

    /// The live name bytes (up to the first NUL).
    pub fn name_bytes(&self) -> &[u8] {
        let end = self
            .name
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(MAX_REGION_NAME);
        &self.name[..end]
    }

    /// Entry is live (not DEAD).
    pub const fn is_live(&self) -> bool {
        self.flags & REGION_FLAG_DEAD == 0
    }

    /// Parse entry `i` from the manifest area.
    pub fn read_from(m: &[u8], i: usize) -> Result<RegionEntry, DecodeError> {
        if i >= REGION_CAPACITY {
            return Err(DecodeError::BadField);
        }
        let at = Self::offset(i);
        if m.len() < at + REGION_ENTRY_SIZE {
            return Err(DecodeError::Truncated);
        }
        let e = &m[at..at + REGION_ENTRY_SIZE];
        let mut name = [0u8; MAX_REGION_NAME];
        name.copy_from_slice(&e[40..96]);
        Ok(RegionEntry {
            region_id: u32::from_le_bytes(e[0..4].try_into().unwrap()),
            name_id: u32::from_le_bytes(e[4..8].try_into().unwrap()),
            layout_version: u32::from_le_bytes(e[8..12].try_into().unwrap()),
            flags: u32::from_le_bytes(e[12..16].try_into().unwrap()),
            gva: u64::from_le_bytes(e[16..24].try_into().unwrap()),
            len: u64::from_le_bytes(e[24..32].try_into().unwrap()),
            extent_off: u32::from_le_bytes(e[32..36].try_into().unwrap()),
            extent_n: u32::from_le_bytes(e[36..40].try_into().unwrap()),
            name,
        })
    }

    /// Serialize into slot `i` of the manifest area.
    pub fn write_to(&self, m: &mut [u8], i: usize) -> Result<(), EncodeError> {
        if i >= REGION_CAPACITY {
            return Err(EncodeError::FieldTooLong);
        }
        let at = Self::offset(i);
        if m.len() < at + REGION_ENTRY_SIZE {
            return Err(EncodeError::BufferTooSmall);
        }
        let e = &mut m[at..at + REGION_ENTRY_SIZE];
        e[0..4].copy_from_slice(&self.region_id.to_le_bytes());
        e[4..8].copy_from_slice(&self.name_id.to_le_bytes());
        e[8..12].copy_from_slice(&self.layout_version.to_le_bytes());
        e[12..16].copy_from_slice(&self.flags.to_le_bytes());
        e[16..24].copy_from_slice(&self.gva.to_le_bytes());
        e[24..32].copy_from_slice(&self.len.to_le_bytes());
        e[32..36].copy_from_slice(&self.extent_off.to_le_bytes());
        e[36..40].copy_from_slice(&self.extent_n.to_le_bytes());
        e[40..96].copy_from_slice(&self.name);
        Ok(())
    }

    /// Bounds-check this entry's extent range against the pool and the header.
    pub fn validate_extents(&self, hdr: &ManifestHeader) -> Result<(), DecodeError> {
        let off = self.extent_off as usize;
        let n = self.extent_n as usize;
        match off.checked_add(n) {
            Some(end) if end <= hdr.extent_count as usize => Ok(()),
            _ => Err(DecodeError::BadField),
        }
    }
}

/// One extent in the pool: a contiguous GPA range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extent {
    /// Guest-physical base.
    pub gpa: u64,
    /// Length in bytes. Extents of one region are logically concatenated in order.
    pub len: u64,
}

impl Extent {
    /// Byte offset of extent `i` within the manifest area.
    pub const fn offset(i: usize) -> usize {
        OFF_EXTENTS + i * EXTENT_SIZE
    }

    /// Parse extent `i` from the manifest area.
    pub fn read_from(m: &[u8], i: usize) -> Result<Extent, DecodeError> {
        if i >= EXTENT_CAPACITY {
            return Err(DecodeError::BadField);
        }
        let at = Self::offset(i);
        if m.len() < at + EXTENT_SIZE {
            return Err(DecodeError::Truncated);
        }
        Ok(Extent {
            gpa: u64::from_le_bytes(m[at..at + 8].try_into().unwrap()),
            len: u64::from_le_bytes(m[at + 8..at + 16].try_into().unwrap()),
        })
    }

    /// Serialize into pool slot `i`.
    pub fn write_to(&self, m: &mut [u8], i: usize) -> Result<(), EncodeError> {
        if i >= EXTENT_CAPACITY {
            return Err(EncodeError::FieldTooLong);
        }
        let at = Self::offset(i);
        if m.len() < at + EXTENT_SIZE {
            return Err(EncodeError::BufferTooSmall);
        }
        m[at..at + 8].copy_from_slice(&self.gpa.to_le_bytes());
        m[at + 8..at + 16].copy_from_slice(&self.len.to_le_bytes());
        Ok(())
    }
}

/// Initialize an empty manifest (agent, during channel setup): canonical
/// header, generation 0 (even), zero counts.
pub fn init_manifest(m: &mut [u8]) -> Result<(), EncodeError> {
    ManifestHeader {
        magic: MANIFEST_MAGIC,
        manifest_version: MANIFEST_VERSION,
        region_capacity: REGION_CAPACITY as u16,
        generation: 0,
        region_count: 0,
        extent_count: 0,
    }
    .write_to(m)
}

/// Read the seqlock generation word.
pub fn read_generation(m: &[u8]) -> Result<u64, DecodeError> {
    if m.len() < OFF_GENERATION + 8 {
        return Err(DecodeError::Truncated);
    }
    Ok(u64::from_le_bytes(
        m[OFF_GENERATION..OFF_GENERATION + 8].try_into().unwrap(),
    ))
}

/// Seqlock writer entry: bump generation to odd. The caller must fence after
/// this and before mutating (in-guest: `core::sync::atomic::fence(SeqCst)`).
/// Returns the new (odd) generation. Errs if a write is already open.
pub fn writer_begin(m: &mut [u8]) -> Result<u64, EncodeError> {
    let g = read_generation(m).map_err(|_| EncodeError::BufferTooSmall)?;
    if g % 2 != 0 {
        return Err(EncodeError::SeqlockMisuse); // nested begin — agent bug
    }
    let new = g + 1;
    m[OFF_GENERATION..OFF_GENERATION + 8].copy_from_slice(&new.to_le_bytes());
    Ok(new)
}

/// Seqlock writer exit: bump generation back to even. The caller must fence
/// between the last mutation and this. Returns the new (even) generation.
pub fn writer_end(m: &mut [u8]) -> Result<u64, EncodeError> {
    let g = read_generation(m).map_err(|_| EncodeError::BufferTooSmall)?;
    if g % 2 != 1 {
        return Err(EncodeError::SeqlockMisuse); // end without begin — agent bug
    }
    let new = g + 1;
    m[OFF_GENERATION..OFF_GENERATION + 8].copy_from_slice(&new.to_le_bytes());
    Ok(new)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> std::vec::Vec<u8> {
        std::vec![0u8; MANIFEST_TOTAL_SIZE]
    }

    #[test]
    fn init_then_read_header() {
        let mut m = area();
        init_manifest(&mut m).unwrap();
        let h = ManifestHeader::read_from(&m).unwrap();
        h.validate().unwrap();
        assert_eq!(h.generation, 0);
        assert_eq!(h.region_count, 0);
    }

    #[test]
    fn entry_and_extent_round_trip() {
        let mut m = area();
        init_manifest(&mut m).unwrap();
        let e = RegionEntry {
            region_id: 3,
            name_id: 7,
            layout_version: 1,
            flags: REGION_FLAG_HOT,
            gva: 0x7f00_0000_0000,
            len: 0x20_0000,
            extent_off: 5,
            extent_n: 2,
            name: RegionEntry::pack_name(b"wram").unwrap(),
        };
        e.write_to(&mut m, 3).unwrap();
        let back = RegionEntry::read_from(&m, 3).unwrap();
        assert_eq!(back, e);
        assert_eq!(back.name_bytes(), b"wram");
        assert!(back.is_live());

        let x = Extent {
            gpa: 0x1000_0000,
            len: 0x20_0000,
        };
        x.write_to(&mut m, 5).unwrap();
        assert_eq!(Extent::read_from(&m, 5).unwrap(), x);
    }

    #[test]
    fn seqlock_generation_discipline() {
        let mut m = area();
        init_manifest(&mut m).unwrap();
        let odd = writer_begin(&mut m).unwrap();
        assert_eq!(odd, 1);
        assert!(writer_begin(&mut m).is_err(), "nested begin must fail");
        let even = writer_end(&mut m).unwrap();
        assert_eq!(even, 2);
        assert!(writer_end(&mut m).is_err(), "end without begin must fail");
        assert_eq!(read_generation(&m).unwrap(), 2);
    }

    #[test]
    fn dead_flag_and_extent_bounds() {
        let mut m = area();
        init_manifest(&mut m).unwrap();
        let mut e = RegionEntry {
            region_id: 0,
            name_id: 1,
            layout_version: 1,
            flags: REGION_FLAG_DEAD,
            gva: 0,
            len: 0,
            extent_off: 1020,
            extent_n: 8,
            name: RegionEntry::pack_name(b"dead").unwrap(),
        };
        assert!(!e.is_live());
        let hdr = ManifestHeader::read_from(&m).unwrap();
        // extent range past extent_count is rejected
        assert!(e.validate_extents(&hdr).is_err());
        e.extent_n = 0;
        e.extent_off = 0;
        assert!(e.validate_extents(&hdr).is_ok());
    }

    #[test]
    fn over_long_name_rejected() {
        assert!(RegionEntry::pack_name(&[b'a'; 57]).is_err());
    }

    #[test]
    fn layout_matches_api_md() {
        assert_eq!(RegionEntry::offset(0), 0x20);
        assert_eq!(RegionEntry::offset(1) - RegionEntry::offset(0), 96);
        assert_eq!(Extent::offset(0), 0x1820);
        assert_eq!(MANIFEST_TOTAL_SIZE, 0x5820);
    }
}
