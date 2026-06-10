//! detguest-host — host-side access to the detchannel (API.md §2).
//!
//! Linked by `determinism-hypervisor`. Everything here observes two invariants
//! from ARCHITECTURE.md §2:
//!
//! 1. **Host writes to channel memory happen only while the vCPU is paused**,
//!    and *every* such write is reported through [`ChannelWriteSink`] so the
//!    hypervisor can append it to the input log. No mutate-without-sink API
//!    exists in this crate, by design (IMPLEMENTATION-PLAN risk table).
//! 2. The host never spins the guest: a full ring C/I is a [`PushError`] the
//!    caller retries at the next pause, not a wait loop.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod channel;
pub mod commands;
pub mod drain;
pub mod guestmem;
pub mod inject;
pub mod manifest;

pub use channel::{AttachError, Channel};
pub use drain::{GuestEvent, OwnedPayload};
pub use guestmem::{GuestMem, MemError, MockGuestMem};
pub use inject::{FaultPlan, InjectResponder, LogFaultPlan, TableFaultPlan};
pub use manifest::{RegionManifest, ResolvedRegion};

use detguest_wire::RingId;

/// Hook through which EVERY host-side mutation of channel memory is reported,
/// so the hypervisor can append it to the input log (ARCHITECTURE.md §2).
/// The hypervisor stamps each entry with the icount at call time.
pub trait ChannelWriteSink {
    /// A record (preceded by its tail `Pad`, when one was needed) was written
    /// into `ring` and published by storing `new_prod`. `bytes` is the full
    /// span written in ring order: pad bytes (if any) then the record.
    fn ring_push(&mut self, ring: RingId, bytes: &[u8], new_prod: u32);
    /// A consumer index was bumped to `new_cons` after draining `ring`.
    fn cons_bump(&mut self, ring: RingId, new_cons: u32);
    /// An `IN` detcall was answered with `value` (API.md §5).
    fn pio_answer(&mut self, port: u16, value: u32);
}

/// One recorded [`ChannelWriteSink`] mutation (testing / audit).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SinkOp {
    /// See [`ChannelWriteSink::ring_push`].
    RingPush {
        /// Ring written.
        ring: RingId,
        /// Full byte span written (pad + record).
        bytes: Vec<u8>,
        /// Published producer index.
        new_prod: u32,
    },
    /// See [`ChannelWriteSink::cons_bump`].
    ConsBump {
        /// Ring drained.
        ring: RingId,
        /// Stored consumer index.
        new_cons: u32,
    },
    /// See [`ChannelWriteSink::pio_answer`].
    PioAnswer {
        /// Port answered.
        port: u16,
        /// Value returned in eax.
        value: u32,
    },
}

/// A [`ChannelWriteSink`] that records every mutation, in order. Used by the
/// loopback acceptance test ("every host mutation appeared exactly once in
/// the recorded trace") and usable by hypervisor tests.
#[derive(Debug, Default)]
pub struct RecordingSink {
    /// The ordered mutation trace.
    pub ops: Vec<SinkOp>,
}

impl ChannelWriteSink for RecordingSink {
    fn ring_push(&mut self, ring: RingId, bytes: &[u8], new_prod: u32) {
        self.ops.push(SinkOp::RingPush {
            ring,
            bytes: bytes.to_vec(),
            new_prod,
        });
    }
    fn cons_bump(&mut self, ring: RingId, new_cons: u32) {
        self.ops.push(SinkOp::ConsBump { ring, new_cons });
    }
    fn pio_answer(&mut self, port: u16, value: u32) {
        self.ops.push(SinkOp::PioAnswer { port, value });
    }
}

/// Errors from wire-level parsing/consistency while reading channel memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum WireError {
    /// Guest memory access failed.
    Mem(MemError),
    /// A structure failed to parse (framing or field validation).
    Decode(detguest_wire::DecodeError),
    /// Ring indices imply more used bytes than the ring holds (corruption).
    CorruptIndices {
        /// Which ring.
        ring: RingId,
    },
    /// The manifest seqlock stayed odd/changing beyond the retry bound.
    SeqlockLivelock,
}

impl From<MemError> for WireError {
    fn from(e: MemError) -> WireError {
        WireError::Mem(e)
    }
}

impl From<detguest_wire::DecodeError> for WireError {
    fn from(e: detguest_wire::DecodeError) -> WireError {
        WireError::Decode(e)
    }
}

/// Errors from pushing a command/workload-control record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PushError {
    /// Not enough free ring space (host retries at the next pause).
    RingFull,
    /// Guest memory access failed.
    Mem(MemError),
    /// Encoding failed (field over its documented limit).
    Encode(detguest_wire::EncodeError),
}

impl From<MemError> for PushError {
    fn from(e: MemError) -> PushError {
        PushError::Mem(e)
    }
}

/// Errors from [`Channel::read_region`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RegionReadError {
    /// No live region with that name in the manifest.
    NameNotFound,
    /// `offset + buf.len()` exceeds the region length, or the extent table is
    /// inconsistent with the region length.
    OutOfBounds,
    /// Manifest read failed.
    Wire(WireError),
    /// Guest memory access failed.
    Mem(MemError),
}

impl From<WireError> for RegionReadError {
    fn from(e: WireError) -> RegionReadError {
        RegionReadError::Wire(e)
    }
}

impl From<MemError> for RegionReadError {
    fn from(e: MemError) -> RegionReadError {
        RegionReadError::Mem(e)
    }
}
