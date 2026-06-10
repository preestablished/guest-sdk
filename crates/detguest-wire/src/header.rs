//! Channel header: magic, version, ring descriptors, drop counters, index cells.
//!
//! Byte layout per ARCHITECTURE.md §2 "Channel memory layout". The channel is a
//! single 2 MiB hugetlb page; every offset below is relative to its base.

use crate::{DecodeError, EncodeError};

/// Channel magic: `"DETGUEST"` read as a little-endian u64.
pub const CHANNEL_MAGIC: u64 = 0x5453_4555_4754_4544;

/// Wire protocol version this crate implements (API.md §3.5).
pub const PROTO_VERSION: u32 = 1;

/// Total channel size: one 2 MiB hugetlb page.
pub const CHANNEL_SIZE: usize = 0x20_0000;

/// Channel size expressed in 4 KiB pages (the INIT_GO commit value, API.md §5).
pub const CHANNEL_SIZE_PAGES: u32 = (CHANNEL_SIZE / 4096) as u32;

/// `header_flags` bit 0: agent finished channel init and emitted `Hello`.
pub const FLAG_AGENT_READY: u32 = 1 << 0;
/// `header_flags` bit 1: a workload has mapped the channel and owns ring W/I halves.
pub const FLAG_WORKLOAD_ATTACHED: u32 = 1 << 1;

// ---- header field offsets (ARCHITECTURE.md §2) ----

/// Offset of `magic` (u64).
pub const OFF_MAGIC: usize = 0x000;
/// Offset of `proto_version` (u32).
pub const OFF_PROTO_VERSION: usize = 0x008;
/// Offset of `header_flags` (u32).
pub const OFF_HEADER_FLAGS: usize = 0x00C;
/// Offset of `ring_desc[4]` ({offset: u32, size: u32} for C, I, A, W).
pub const OFF_RING_DESC: usize = 0x010;
/// Offset of the reserved 16-byte area after the ring descriptors.
pub const OFF_RESERVED: usize = 0x030;

// Drop counters (all u64, written only by the owning ring's producer).

/// Offset of `ringA_dropped_records` (u64).
pub const OFF_RING_A_DROPPED_RECORDS: usize = 0x040;
/// Offset of `ringA_dropped_bytes` (u64).
pub const OFF_RING_A_DROPPED_BYTES: usize = 0x048;
/// Offset of `ringW_dropped_records` (u64).
pub const OFF_RING_W_DROPPED_RECORDS: usize = 0x050;
/// Offset of `ringW_dropped_bytes` (u64).
pub const OFF_RING_W_DROPPED_BYTES: usize = 0x058;
/// Offset of `ringW_dropped_by_kind[16]` (u64 each, index = EventKind 0..15).
pub const OFF_RING_W_DROPPED_BY_KIND: usize = 0x060;
/// Number of per-kind drop counter slots.
pub const DROPPED_BY_KIND_SLOTS: usize = 16;

// Ring index cells. Each free-running u32 index lives alone in a 64-byte
// cache line (ARCHITECTURE.md §2).

/// Offset of the ring C producer index (host-owned).
pub const OFF_RING_C_PROD: usize = 0x100;
/// Offset of the ring C consumer index (agent-owned).
pub const OFF_RING_C_CONS: usize = 0x140;
/// Offset of the ring I producer index (host-owned).
pub const OFF_RING_I_PROD: usize = 0x180;
/// Offset of the ring I consumer index (SDK-owned).
pub const OFF_RING_I_CONS: usize = 0x1C0;
/// Offset of the ring A producer index (agent-owned).
pub const OFF_RING_A_PROD: usize = 0x200;
/// Offset of the ring A consumer index (host-owned).
pub const OFF_RING_A_CONS: usize = 0x240;
/// Offset of the ring W producer index (SDK-owned).
pub const OFF_RING_W_PROD: usize = 0x280;
/// Offset of the ring W consumer index (host-owned).
pub const OFF_RING_W_CONS: usize = 0x2C0;

/// Offset of the region manifest area (28 KiB; format in API.md §4).
pub const OFF_MANIFEST: usize = 0x1000;

// Canonical ring data placement (ARCHITECTURE.md §2).

/// Ring C data offset.
pub const OFF_RING_C_DATA: usize = 0x8000;
/// Ring C size (16 KiB).
pub const RING_C_SIZE: u32 = 0x4000;
/// Ring I data offset.
pub const OFF_RING_I_DATA: usize = 0xC000;
/// Ring I size (16 KiB).
pub const RING_I_SIZE: u32 = 0x4000;
/// Ring A data offset.
pub const OFF_RING_A_DATA: usize = 0x1_0000;
/// Ring A size (64 KiB).
pub const RING_A_SIZE: u32 = 0x1_0000;
/// Ring W data offset.
pub const OFF_RING_W_DATA: usize = 0x2_0000;
/// Ring W size (1 MiB).
///
/// ARCHITECTURE.md §2's layout table gives ring W 0x1E0000 bytes, but that
/// contradicts the same section's normative index discipline ("free-running
/// u32, masked by `size - 1` — sizes are powers of two") and API.md §2's
/// attach validation, which rejects non-power-of-two descriptors. 0x1E0000 is
/// not a power of two; free-running u32 indices break at u32 wraparound for
/// any size that does not divide 2^32. We take the power-of-two requirement as
/// normative (both validation paths depend on it) and size W at the largest
/// power of two that fits the documented area: 1 MiB at 0x2_0000, with
/// 0x12_0000..0x20_0000 reserved. Tracked as a spec documentation issue.
pub const RING_W_SIZE: u32 = 0x10_0000;

/// Reserved area after ring W data (headroom from the ring-W sizing decision
/// documented on [`RING_W_SIZE`]).
pub const OFF_RESERVED_TAIL: usize = OFF_RING_W_DATA + RING_W_SIZE as usize;

// Layout invariants — drift fails compilation (IMPLEMENTATION-PLAN M0).
const _: () = {
    assert!(OFF_RING_DESC + 4 * 8 == OFF_RESERVED);
    assert!(OFF_RING_W_DROPPED_BY_KIND + DROPPED_BY_KIND_SLOTS * 8 <= OFF_RING_C_PROD);
    assert!(OFF_RING_W_CONS + 64 <= OFF_MANIFEST);
    // Rings are power-of-two sized (free-running index discipline requires it).
    assert!(RING_C_SIZE.is_power_of_two());
    assert!(RING_I_SIZE.is_power_of_two());
    assert!(RING_A_SIZE.is_power_of_two());
    assert!(RING_W_SIZE.is_power_of_two());
    // Ring areas pack back-to-back: C | I | A | W | reserved tail | end.
    assert!(OFF_RING_C_DATA + RING_C_SIZE as usize == OFF_RING_I_DATA);
    assert!(OFF_RING_I_DATA + RING_I_SIZE as usize == OFF_RING_A_DATA);
    assert!(OFF_RING_A_DATA + RING_A_SIZE as usize == OFF_RING_W_DATA);
    assert!(OFF_RING_W_DATA + RING_W_SIZE as usize == OFF_RESERVED_TAIL);
    assert!(OFF_RESERVED_TAIL <= CHANNEL_SIZE);
    // The manifest area (0x1000..0x8000) holds the API.md §4 layout with room to grow.
    assert!(crate::manifest::MANIFEST_TOTAL_SIZE <= OFF_RING_C_DATA - OFF_MANIFEST);
};

/// The four SPSC rings (ARCHITECTURE.md §2 table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum RingId {
    /// host → agent control commands.
    C = 0,
    /// host → workload control records (quiesce relay; never pad input).
    I = 1,
    /// agent → host events.
    A = 2,
    /// SDK (workload) → host events.
    W = 3,
}

impl RingId {
    /// All rings in descriptor order.
    pub const ALL: [RingId; 4] = [RingId::C, RingId::I, RingId::A, RingId::W];

    /// Offset of this ring's producer index cell.
    pub const fn prod_offset(self) -> usize {
        match self {
            RingId::C => OFF_RING_C_PROD,
            RingId::I => OFF_RING_I_PROD,
            RingId::A => OFF_RING_A_PROD,
            RingId::W => OFF_RING_W_PROD,
        }
    }

    /// Offset of this ring's consumer index cell.
    pub const fn cons_offset(self) -> usize {
        match self {
            RingId::C => OFF_RING_C_CONS,
            RingId::I => OFF_RING_I_CONS,
            RingId::A => OFF_RING_A_CONS,
            RingId::W => OFF_RING_W_CONS,
        }
    }

    /// Canonical data area for this ring.
    pub const fn canonical_desc(self) -> RingDesc {
        match self {
            RingId::C => RingDesc {
                offset: OFF_RING_C_DATA as u32,
                size: RING_C_SIZE,
            },
            RingId::I => RingDesc {
                offset: OFF_RING_I_DATA as u32,
                size: RING_I_SIZE,
            },
            RingId::A => RingDesc {
                offset: OFF_RING_A_DATA as u32,
                size: RING_A_SIZE,
            },
            RingId::W => RingDesc {
                offset: OFF_RING_W_DATA as u32,
                size: RING_W_SIZE,
            },
        }
    }
}

/// One ring descriptor in the channel header: `{offset: u32, size: u32}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RingDesc {
    /// Byte offset of the ring data area from the channel base.
    pub offset: u32,
    /// Ring data size in bytes; must be a power of two.
    pub size: u32,
}

impl RingDesc {
    /// Validate this descriptor against the channel bounds (host attach path).
    ///
    /// Checks: power-of-two size, nonzero, data area within the 2 MiB page and
    /// not overlapping the header/manifest area.
    pub fn validate(&self) -> Result<(), DecodeError> {
        if self.size == 0 || !self.size.is_power_of_two() {
            return Err(DecodeError::BadField);
        }
        let off = self.offset as usize;
        let size = self.size as usize;
        if off < OFF_RING_C_DATA || off.checked_add(size).is_none() || off + size > CHANNEL_SIZE {
            return Err(DecodeError::BadField);
        }
        if off % 8 != 0 {
            return Err(DecodeError::BadField);
        }
        Ok(())
    }
}

/// Decoded channel header (fixed fields only; counters/indices live in place).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelHeader {
    /// Must equal [`CHANNEL_MAGIC`].
    pub magic: u64,
    /// Must equal [`PROTO_VERSION`].
    pub proto_version: u32,
    /// [`FLAG_AGENT_READY`] | [`FLAG_WORKLOAD_ATTACHED`].
    pub header_flags: u32,
    /// Descriptors for rings C, I, A, W in that order.
    pub ring_desc: [RingDesc; 4],
}

impl ChannelHeader {
    /// A spec-canonical header: proto v1, canonical ring placement, no flags set.
    pub fn canonical() -> Self {
        ChannelHeader {
            magic: CHANNEL_MAGIC,
            proto_version: PROTO_VERSION,
            header_flags: 0,
            ring_desc: [
                RingId::C.canonical_desc(),
                RingId::I.canonical_desc(),
                RingId::A.canonical_desc(),
                RingId::W.canonical_desc(),
            ],
        }
    }

    /// Serialize the fixed header fields into the start of a channel page.
    ///
    /// Writes bytes `0x000..0x030` only; counters and index cells are left
    /// untouched (the agent zeroes the whole page before this).
    pub fn write_to(&self, buf: &mut [u8]) -> Result<(), EncodeError> {
        if buf.len() < OFF_RESERVED {
            return Err(EncodeError::BufferTooSmall);
        }
        buf[OFF_MAGIC..OFF_MAGIC + 8].copy_from_slice(&self.magic.to_le_bytes());
        buf[OFF_PROTO_VERSION..OFF_PROTO_VERSION + 4]
            .copy_from_slice(&self.proto_version.to_le_bytes());
        buf[OFF_HEADER_FLAGS..OFF_HEADER_FLAGS + 4]
            .copy_from_slice(&self.header_flags.to_le_bytes());
        for (i, d) in self.ring_desc.iter().enumerate() {
            let at = OFF_RING_DESC + i * 8;
            buf[at..at + 4].copy_from_slice(&d.offset.to_le_bytes());
            buf[at + 4..at + 8].copy_from_slice(&d.size.to_le_bytes());
        }
        Ok(())
    }

    /// Decode the fixed header fields from the start of a channel page.
    ///
    /// Pure parse — no validation beyond length. Use [`ChannelHeader::validate`]
    /// for the attach-path checks.
    pub fn read_from(buf: &[u8]) -> Result<Self, DecodeError> {
        if buf.len() < OFF_RESERVED {
            return Err(DecodeError::Truncated);
        }
        let magic = u64::from_le_bytes(buf[OFF_MAGIC..OFF_MAGIC + 8].try_into().unwrap());
        let proto_version = u32::from_le_bytes(
            buf[OFF_PROTO_VERSION..OFF_PROTO_VERSION + 4]
                .try_into()
                .unwrap(),
        );
        let header_flags = u32::from_le_bytes(
            buf[OFF_HEADER_FLAGS..OFF_HEADER_FLAGS + 4]
                .try_into()
                .unwrap(),
        );
        let mut ring_desc = [RingDesc { offset: 0, size: 0 }; 4];
        for (i, d) in ring_desc.iter_mut().enumerate() {
            let at = OFF_RING_DESC + i * 8;
            d.offset = u32::from_le_bytes(buf[at..at + 4].try_into().unwrap());
            d.size = u32::from_le_bytes(buf[at + 4..at + 8].try_into().unwrap());
        }
        Ok(ChannelHeader {
            magic,
            proto_version,
            header_flags,
            ring_desc,
        })
    }

    /// Attach-path validation (API.md §2 `Channel::attach`): magic, version,
    /// every ring descriptor within the page with power-of-two size, and no
    /// two ring data areas overlapping (two SPSC rings aliasing the same
    /// bytes would break the single-owner discipline `ring` relies on).
    pub fn validate(&self) -> Result<(), DecodeError> {
        if self.magic != CHANNEL_MAGIC {
            return Err(DecodeError::BadMagic);
        }
        if self.proto_version != PROTO_VERSION {
            return Err(DecodeError::BadVersion);
        }
        for d in &self.ring_desc {
            d.validate()?;
        }
        for i in 0..self.ring_desc.len() {
            for j in (i + 1)..self.ring_desc.len() {
                let (a, b) = (&self.ring_desc[i], &self.ring_desc[j]);
                let a_end = a.offset as u64 + a.size as u64;
                let b_end = b.offset as u64 + b.size as u64;
                if (a.offset as u64) < b_end && (b.offset as u64) < a_end {
                    return Err(DecodeError::BadField);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_header_round_trips_and_validates() {
        let h = ChannelHeader::canonical();
        let mut page = [0u8; 0x40];
        h.write_to(&mut page).unwrap();
        let back = ChannelHeader::read_from(&page).unwrap();
        assert_eq!(h, back);
        back.validate().unwrap();
    }

    #[test]
    fn magic_bytes_spell_detguest() {
        assert_eq!(&CHANNEL_MAGIC.to_le_bytes(), b"DETGUEST");
    }

    #[test]
    fn bad_magic_and_version_rejected() {
        let mut h = ChannelHeader::canonical();
        h.magic ^= 1;
        assert_eq!(h.validate(), Err(DecodeError::BadMagic));
        let mut h = ChannelHeader::canonical();
        h.proto_version = 2;
        assert_eq!(h.validate(), Err(DecodeError::BadVersion));
    }

    #[test]
    fn ring_desc_bounds_checked() {
        let mut h = ChannelHeader::canonical();
        h.ring_desc[3].size = 0x30_0000; // larger than the page
        assert!(h.validate().is_err());
        let mut h = ChannelHeader::canonical();
        h.ring_desc[0].size = 0x3000; // not a power of two
        assert!(h.validate().is_err());
        let mut h = ChannelHeader::canonical();
        h.ring_desc[0].offset = 0x100; // overlaps header area
        assert!(h.validate().is_err());
    }

    #[test]
    fn overlapping_ring_descs_rejected() {
        // Two rings aliasing the same bytes would break SPSC single-ownership.
        let mut h = ChannelHeader::canonical();
        h.ring_desc[1] = h.ring_desc[0];
        assert_eq!(h.validate(), Err(DecodeError::BadField));
        // Partial overlap is rejected too.
        let mut h = ChannelHeader::canonical();
        h.ring_desc[1].offset = h.ring_desc[0].offset + 8;
        h.ring_desc[1].size = h.ring_desc[0].size;
        assert_eq!(h.validate(), Err(DecodeError::BadField));
    }

    #[test]
    fn index_cells_are_cache_line_separated() {
        let offs = [
            OFF_RING_C_PROD,
            OFF_RING_C_CONS,
            OFF_RING_I_PROD,
            OFF_RING_I_CONS,
            OFF_RING_A_PROD,
            OFF_RING_A_CONS,
            OFF_RING_W_PROD,
            OFF_RING_W_CONS,
        ];
        for w in offs.windows(2) {
            assert_eq!(w[1] - w[0], 64);
        }
    }
}
