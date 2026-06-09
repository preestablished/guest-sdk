//! detguest-wire — byte-level wire formats for the detchannel.
//!
//! Shared by the in-guest SDK/agent producers and the host-side consumer so the
//! encode/decode paths are bit-for-bit identical on both sides (ARCHITECTURE.md §1).
//!
//! Normative references: `prompts/docs/guest-sdk/API.md` (record kinds, payloads,
//! manifest format, detcall ABI) and `prompts/docs/guest-sdk/ARCHITECTURE.md` §2
//! (channel memory layout, ring discipline, memory-ordering rules).
//!
//! All on-wire integers are little-endian; all structures are fixed-layout —
//! no serde on the hot path, every encoder/decoder here is hand-written.
//! No floating point anywhere in this crate (ARCHITECTURE.md §7 rule 8).
#![no_std]
// Module-scoped unsafe policy (IMPLEMENTATION-PLAN M6): unsafe is permitted only
// in `ring` (SPSC ring-pointer arithmetic over shared channel memory). Every other
// module inherits this crate-level deny.
#![deny(unsafe_code)]
#![deny(missing_docs)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(test)]
extern crate std;

pub mod events;
pub mod header;
pub mod manifest;
pub mod ports;
pub mod record;
pub mod ring;

pub use events::{Command, EventPayload, WorkloadCtrl};
pub use header::{ChannelHeader, RingDesc, RingId, CHANNEL_MAGIC, CHANNEL_SIZE, PROTO_VERSION};
pub use ports::FaultDecision;
pub use record::{EventKind, RecordHeader, MAX_RECORD_LEN, MIN_RECORD_LEN, RECORD_HEADER_LEN};

/// Errors returned by wire decoders.
///
/// Decoders never panic: arbitrary bytes decode to `Err`, never to UB or abort
/// (this is the property the `decode_record` fuzz target locks in).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DecodeError {
    /// Input shorter than the structure's fixed prefix.
    Truncated,
    /// `len` field violates framing rules (alignment, bounds, or payload fit).
    BadLen,
    /// Unknown record kind for the ring's namespace.
    UnknownKind(u8),
    /// A payload field violates its documented range.
    BadField,
    /// Structure magic mismatch.
    BadMagic,
    /// Structure version mismatch.
    BadVersion,
}

/// Errors returned by wire encoders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EncodeError {
    /// Destination buffer too small for the encoded record.
    BufferTooSmall,
    /// A field exceeds its documented limit (e.g. name > 256 bytes).
    FieldTooLong,
}
