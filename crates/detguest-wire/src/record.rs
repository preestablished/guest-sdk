//! Record framing (API.md §3.0) and the per-ring kind namespaces (§3.1, §3.3, §3.4).
//!
//! Every record on every ring:
//!
//! ```text
//! offset  size  field    notes
//! 0       2     len      u16, total record bytes incl. this header; multiple of 8;
//!                        16 ≤ len ≤ 4096 (Pad: 8 ≤ len, see below)
//! 2       1     kind     namespace depends on ring (EventKind for A/W,
//!                        CommandKind for C, WorkloadCtrlKind for I)
//! 3       1     flags    bit0 TRUNCATED  bit1 REACHABLE_DECL (NameIntern only)
//! 4       4     seq      u32 per-ring producer counter, starts at 0
//! 8       8     vnanos   u64 guest CLOCK_MONOTONIC_RAW ns; 0 for host-produced
//! 16      ...   payload  kind-specific, zero-padded to 8-byte multiple
//! ```
//!
//! Records start 8-byte aligned and never wrap: when the bytes remaining before
//! the ring end cannot hold the record, the producer writes a `Pad` record
//! (kind 0) whose `len` covers the whole tail and starts the real record at
//! offset 0. Because record positions are always 8-byte aligned, the smallest
//! possible tail is 8 bytes — a `Pad` there is the 8-byte header prefix only
//! (len, kind, flags, seq; no `vnanos`). `Pad` records consume a `seq` number
//! like any other record, so per-ring seq stays strictly monotonic.

use crate::{DecodeError, EncodeError};

/// Full record header size in bytes.
pub const RECORD_HEADER_LEN: usize = 16;
/// Minimum length of a non-`Pad` record (a bare header).
pub const MIN_RECORD_LEN: usize = 16;
/// Maximum record length, header included.
pub const MAX_RECORD_LEN: usize = 4096;
/// Record alignment; `len` is always a multiple of this.
pub const RECORD_ALIGN: usize = 8;
/// Minimum length of a `Pad` record (header prefix without `vnanos`).
pub const PAD_MIN_LEN: usize = 8;

/// `flags` bit 0: variable-length payload was clipped at its documented cap.
pub const FLAG_TRUNCATED: u8 = 1 << 0;
/// `flags` bit 1: `NameIntern` emitted by `declare_reachable()` (API.md §3.2).
pub const FLAG_REACHABLE_DECL: u8 = 1 << 1;

/// Round a payload length up to the record alignment.
pub const fn pad8(n: usize) -> usize {
    (n + (RECORD_ALIGN - 1)) & !(RECORD_ALIGN - 1)
}

/// Total record length for a payload of `payload_len` bytes.
pub const fn record_len(payload_len: usize) -> usize {
    RECORD_HEADER_LEN + pad8(payload_len)
}

/// Event kinds for rings A and W (API.md §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum EventKind {
    /// Tail filler; carries nothing.
    Pad = 0,
    /// Agent announce after CHANNEL_INIT (critical).
    Hello = 1,
    /// name → name_id binding (critical).
    NameIntern = 2,
    /// `assert_always` violation (critical).
    AssertViolation = 3,
    /// First hit of an `expect_reachable` name (critical).
    Reachable = 4,
    /// First hit of a coverage beacon id (droppable).
    Beacon = 5,
    /// `inject_point` query, paired with the INJECT detcall (critical).
    InjectQuery = 6,
    /// Region published in the manifest (critical).
    RegionRegister = 7,
    /// Region re-verified/unregistered (critical).
    RegionUpdate = 8,
    /// Workload exec'd (critical).
    WorkloadStarted = 9,
    /// Workload reaped (critical).
    WorkloadExited = 10,
    /// Structured log line (droppable).
    LogLine = 11,
    /// Quiesce point reached (critical).
    QuiesceReady = 12,
    /// Frame boundary, paired with the FRAME_COUNTER MMIO write (critical).
    FrameMark = 13,
    /// The deterministic READY point (critical; ARCHITECTURE.md §4.1).
    Ready = 14,
}

impl EventKind {
    /// Decode a kind byte; `None` for kinds this proto version does not define
    /// (consumers skip unknown kinds by `len` — API.md §3.5).
    pub const fn from_u8(v: u8) -> Option<EventKind> {
        Some(match v {
            0 => EventKind::Pad,
            1 => EventKind::Hello,
            2 => EventKind::NameIntern,
            3 => EventKind::AssertViolation,
            4 => EventKind::Reachable,
            5 => EventKind::Beacon,
            6 => EventKind::InjectQuery,
            7 => EventKind::RegionRegister,
            8 => EventKind::RegionUpdate,
            9 => EventKind::WorkloadStarted,
            10 => EventKind::WorkloadExited,
            11 => EventKind::LogLine,
            12 => EventKind::QuiesceReady,
            13 => EventKind::FrameMark,
            14 => EventKind::Ready,
            _ => return None,
        })
    }

    /// Critical events doorbell-and-retry on a full ring; droppable events bump
    /// the drop counters and are skipped (ARCHITECTURE.md §3).
    pub const fn is_critical(self) -> bool {
        !matches!(
            self,
            EventKind::Pad | EventKind::Beacon | EventKind::LogLine
        )
    }
}

/// Command kinds for ring C, host → agent (API.md §3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum CommandKind {
    /// Fork+exec a preconfigured workload unit.
    StartWorkload = 1,
    /// Request a quiesce point (COOP relays onto ring I; FORCED stops the workload).
    Quiesce = 2,
    /// Resume a FORCED-quiesced workload (COOP Resume rides ring I).
    Resume = 3,
    /// Kill the workload and power off the VM.
    Shutdown = 4,
    /// Adjust LogLine production.
    SetLogMask = 5,
    /// Re-walk pagemap for all live regions.
    ReverifyRegions = 6,
}

impl CommandKind {
    /// Decode a kind byte.
    pub const fn from_u8(v: u8) -> Option<CommandKind> {
        Some(match v {
            1 => CommandKind::StartWorkload,
            2 => CommandKind::Quiesce,
            3 => CommandKind::Resume,
            4 => CommandKind::Shutdown,
            5 => CommandKind::SetLogMask,
            6 => CommandKind::ReverifyRegions,
            _ => return None,
        })
    }
}

/// Workload-control kinds for ring I, host → workload/SDK (API.md §3.4).
///
/// Kind 1 was the removed generic Input record; pad input never rides ring I
/// and kind 1 is never reassigned in v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum WorkloadCtrlKind {
    /// Quiesce relay from the agent.
    QuiesceReq = 2,
    /// Unpark a cooperatively-quiesced workload.
    Resume = 3,
}

impl WorkloadCtrlKind {
    /// Decode a kind byte.
    pub const fn from_u8(v: u8) -> Option<WorkloadCtrlKind> {
        Some(match v {
            2 => WorkloadCtrlKind::QuiesceReq,
            3 => WorkloadCtrlKind::Resume,
            _ => return None,
        })
    }
}

/// A decoded record header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordHeader {
    /// Total record bytes including this header; multiple of 8.
    pub len: u16,
    /// Kind byte in the owning ring's namespace.
    pub kind: u8,
    /// [`FLAG_TRUNCATED`] | [`FLAG_REACHABLE_DECL`].
    pub flags: u8,
    /// Per-ring producer record counter, starts at 0. `Pad` consumes one too.
    pub seq: u32,
    /// Producer's guest `CLOCK_MONOTONIC_RAW` ns; 0 on host-produced records
    /// and on 8-byte tail `Pad`s (which have no `vnanos` field on the wire).
    pub vnanos: u64,
}

impl RecordHeader {
    /// Serialize into `buf` (which must hold `self.len` bytes). Writes only the
    /// header prefix that exists at this `len` (8 bytes for a minimal `Pad`).
    pub fn write_to(&self, buf: &mut [u8]) -> Result<(), EncodeError> {
        let hdr = core::cmp::min(self.len as usize, RECORD_HEADER_LEN);
        if buf.len() < hdr {
            return Err(EncodeError::BufferTooSmall);
        }
        buf[0..2].copy_from_slice(&self.len.to_le_bytes());
        buf[2] = self.kind;
        buf[3] = self.flags;
        buf[4..8].copy_from_slice(&self.seq.to_le_bytes());
        if hdr == RECORD_HEADER_LEN {
            buf[8..16].copy_from_slice(&self.vnanos.to_le_bytes());
        }
        Ok(())
    }

    /// Decode and frame-validate a record header from the start of `buf`.
    ///
    /// Enforces the framing rules: `len` a multiple of 8, within
    /// [`MIN_RECORD_LEN`]`..=`[`MAX_RECORD_LEN`] (`Pad`: [`PAD_MIN_LEN`] minimum),
    /// and `len <= buf.len()`. Does not interpret the kind beyond the `Pad`
    /// special case — unknown kinds are the caller's concern.
    pub fn read_from(buf: &[u8]) -> Result<RecordHeader, DecodeError> {
        if buf.len() < PAD_MIN_LEN {
            return Err(DecodeError::Truncated);
        }
        let len = u16::from_le_bytes(buf[0..2].try_into().unwrap());
        let kind = buf[2];
        let flags = buf[3];
        let seq = u32::from_le_bytes(buf[4..8].try_into().unwrap());
        let l = len as usize;
        if l % RECORD_ALIGN != 0 || l > MAX_RECORD_LEN {
            return Err(DecodeError::BadLen);
        }
        let min = if kind == EventKind::Pad as u8 {
            PAD_MIN_LEN
        } else {
            MIN_RECORD_LEN
        };
        if l < min {
            return Err(DecodeError::BadLen);
        }
        if l > buf.len() {
            return Err(DecodeError::Truncated);
        }
        let vnanos = if l >= RECORD_HEADER_LEN {
            u64::from_le_bytes(buf[8..16].try_into().unwrap())
        } else {
            0
        };
        Ok(RecordHeader {
            len,
            kind,
            flags,
            seq,
            vnanos,
        })
    }

    /// The payload byte range within a record of this header, if any.
    ///
    /// For records shorter than a full header (8-byte tail `Pad`s) the empty
    /// range sits at `len`, not at [`RECORD_HEADER_LEN`] — an empty range must
    /// still be in-bounds for the record's own bytes, or slicing the record
    /// with it panics (found by the `decode_record` fuzz target).
    pub fn payload_range(&self) -> core::ops::Range<usize> {
        let l = self.len as usize;
        if l <= RECORD_HEADER_LEN {
            l..l
        } else {
            RECORD_HEADER_LEN..l
        }
    }
}

/// Encode a `Pad` record covering exactly `tail_len` bytes at the ring tail.
///
/// `tail_len` must be a multiple of 8 and at least [`PAD_MIN_LEN`]. Tails of at
/// least 16 bytes get a full header with `vnanos = 0`; an 8-byte tail gets the
/// 8-byte header prefix only. Bytes past the written header up to `tail_len`
/// are zeroed.
pub fn encode_pad(buf: &mut [u8], tail_len: usize, seq: u32) -> Result<usize, EncodeError> {
    debug_assert!(tail_len % RECORD_ALIGN == 0 && tail_len >= PAD_MIN_LEN);
    if buf.len() < tail_len || tail_len > u16::MAX as usize {
        return Err(EncodeError::BufferTooSmall);
    }
    let hdr = RecordHeader {
        len: tail_len as u16,
        kind: EventKind::Pad as u8,
        flags: 0,
        seq,
        vnanos: 0,
    };
    buf[..tail_len].fill(0);
    hdr.write_to(buf)?;
    Ok(tail_len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_round_trip() {
        let h = RecordHeader {
            len: 32,
            kind: 3,
            flags: 1,
            seq: 7,
            vnanos: 99,
        };
        let mut buf = [0u8; 32];
        h.write_to(&mut buf).unwrap();
        assert_eq!(RecordHeader::read_from(&buf).unwrap(), h);
    }

    #[test]
    fn eight_byte_pad_round_trip() {
        let mut buf = [0xAAu8; 8];
        encode_pad(&mut buf, 8, 41).unwrap();
        let h = RecordHeader::read_from(&buf).unwrap();
        assert_eq!(h.len, 8);
        assert_eq!(h.kind, EventKind::Pad as u8);
        assert_eq!(h.seq, 41);
        assert_eq!(h.vnanos, 0);
    }

    #[test]
    fn framing_rules_enforced() {
        // not multiple of 8
        let mut buf = [0u8; 32];
        RecordHeader {
            len: 20,
            kind: 1,
            flags: 0,
            seq: 0,
            vnanos: 0,
        }
        .write_to(&mut buf)
        .unwrap();
        assert_eq!(RecordHeader::read_from(&buf), Err(DecodeError::BadLen));
        // non-pad below 16
        RecordHeader {
            len: 8,
            kind: 1,
            flags: 0,
            seq: 0,
            vnanos: 0,
        }
        .write_to(&mut buf)
        .unwrap();
        assert_eq!(RecordHeader::read_from(&buf), Err(DecodeError::BadLen));
        // longer than the buffer
        RecordHeader {
            len: 64,
            kind: 1,
            flags: 0,
            seq: 0,
            vnanos: 0,
        }
        .write_to(&mut buf)
        .unwrap();
        assert_eq!(
            RecordHeader::read_from(&buf[..32]),
            Err(DecodeError::Truncated)
        );
        // over MAX_RECORD_LEN
        let mut big = [0u8; 8192];
        RecordHeader {
            len: 4104,
            kind: 1,
            flags: 0,
            seq: 0,
            vnanos: 0,
        }
        .write_to(&mut big)
        .unwrap();
        assert_eq!(RecordHeader::read_from(&big), Err(DecodeError::BadLen));
    }

    #[test]
    fn criticality_classes_match_spec() {
        use EventKind::*;
        for k in [
            Hello,
            NameIntern,
            AssertViolation,
            Reachable,
            InjectQuery,
            RegionRegister,
            RegionUpdate,
            WorkloadStarted,
            WorkloadExited,
            QuiesceReady,
            FrameMark,
            Ready,
        ] {
            assert!(k.is_critical(), "{k:?} must be critical");
        }
        for k in [Pad, Beacon, LogLine] {
            assert!(!k.is_critical(), "{k:?} must be droppable");
        }
    }

    #[test]
    fn ring_i_kind_one_stays_reserved() {
        assert_eq!(WorkloadCtrlKind::from_u8(1), None);
    }

    #[test]
    fn eight_byte_pad_decodes_as_event_without_panicking() {
        // Regression: fuzz artifact crash-f3aa5f21 — an 8-byte tail Pad fed to
        // decode_event sliced bytes[16..16] out of an 8-byte input and
        // panicked. payload_range must stay within the record's own length.
        let bytes = [0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFC, 0x0A];
        let hdr = RecordHeader::read_from(&bytes).unwrap();
        assert_eq!(hdr.payload_range(), 8..8);
        let (hdr, ev) = crate::events::decode_event(&bytes).unwrap();
        assert_eq!(hdr.len, 8);
        assert_eq!(ev, crate::events::EventPayload::Pad);
    }
}
