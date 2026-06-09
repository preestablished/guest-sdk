//! detcall PIO register ABI: port numbers and `FaultDecision` packing (API.md §5, §1.4).
//!
//! The detcall ABI is x86-only by design; the constants live behind this module
//! so a future aarch64 guest ABI slot exists without touching the rest of the
//! crate (IMPLEMENTATION-PLAN "CI tiering").

/// First port of the detcall range.
pub const PORT_RANGE_START: u16 = 0xD370;
/// Last port of the detcall range (inclusive). Unknown ports inside are RAZ/WI.
pub const PORT_RANGE_END: u16 = 0xD39F;

/// IN: identify — returns [`IDENT_VALUE`].
pub const PORT_IDENT: u16 = 0xD370;
/// OUT: channel init, GPA bits 0–31 (latched).
pub const PORT_INIT_LO: u16 = 0xD374;
/// OUT: channel init, GPA bits 32–63 (latched).
pub const PORT_INIT_HI: u16 = 0xD378;
/// OUT: commit init (eax = channel size in 4 KiB pages); IN: init status.
pub const PORT_INIT_GO: u16 = 0xD37C;
/// OUT: doorbell (eax = ring mask) — host drains those guest→host rings now.
pub const PORT_DOORBELL: u16 = 0xD380;
/// OUT: inject query (eax = iseq); IN: packed `FaultDecision` for that iseq.
pub const PORT_INJECT: u16 = 0xD384;
/// OUT: quiesce ack (eax = low 32 bits of token) — agent FORCED path only.
pub const PORT_QUIESCE_ACK: u16 = 0xD388;

/// `IN 0xD370` return value: magic `0xD37E` in the high half, proto version 1 low.
pub const IDENT_VALUE: u32 = 0xD37E_0001;

/// Doorbell mask bit 0: drain ring A.
pub const DOORBELL_RING_A: u32 = 1 << 0;
/// Doorbell mask bit 1: drain ring W.
pub const DOORBELL_RING_W: u32 = 1 << 1;

/// `IN 0xD37C` status codes after an INIT_GO commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum InitStatus {
    /// Channel validated and attached.
    Ok = 0,
    /// Latched GPA invalid (unmapped / misaligned / out of guest RAM).
    BadGpa = 1,
    /// Channel header magic or proto_version mismatch at the latched GPA.
    BadMagicVersion = 2,
    /// A channel is already attached for this VM.
    AlreadyAttached = 3,
}

impl InitStatus {
    /// Decode a status word; unknown values are an error (host bug).
    pub const fn from_u32(v: u32) -> Option<InitStatus> {
        match v {
            0 => Some(InitStatus::Ok),
            1 => Some(InitStatus::BadGpa),
            2 => Some(InitStatus::BadMagicVersion),
            3 => Some(InitStatus::AlreadyAttached),
            _ => None,
        }
    }
}

/// Platform-defined fault kind 1: fail the operation (arg = suggested errno).
pub const PLATFORM_FAIL_GENERIC: u8 = 1;
/// Platform-defined fault kind 2: short read/write (arg = max bytes/items).
pub const PLATFORM_SHORT_COUNT: u8 = 2;
/// Platform-defined fault kind 3: sleep arg milliseconds of *virtual* time.
pub const PLATFORM_DELAY_VIRTUAL: u8 = 3;

/// Maximum value of the packed 24-bit `arg` field.
pub const FAULT_ARG_MAX: u32 = 0x00FF_FFFF;

/// The host's answer to an `inject_point` query (API.md §1.4).
///
/// Packed into 32 bits on the wire: bits 0..8 = kind, bits 8..32 = arg (u24).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultDecision {
    /// kind 0 — no fault; take the normal path.
    Proceed,
    /// kinds 1–63: platform-defined (see the `PLATFORM_*` kind constants).
    Platform {
        /// Fault kind, 1..=63.
        kind: u8,
        /// 24-bit argument; semantics per kind.
        arg: u32,
    },
    /// kinds 64–255: workload-defined; semantics owned by the workload's
    /// fault-plan schema (the input-synthesizer treats them opaquely).
    Workload {
        /// Fault kind, 64..=255.
        kind: u8,
        /// 24-bit argument; semantics per the workload's schema.
        arg: u32,
    },
}

impl FaultDecision {
    /// Pack for the `IN 0xD384` answer. `arg` is masked to 24 bits; arguments
    /// above [`FAULT_ARG_MAX`] cannot be represented and are truncated, so
    /// callers must range-check first (the host crate does).
    ///
    /// Kind invariants (debug-asserted): `Platform.kind` is `1..=63` and
    /// `Workload.kind` is `64..=255`. A `Platform { kind: 0, .. }` would
    /// silently round-trip back to `Proceed`, dropping `arg` — construct
    /// decisions via [`FaultDecision::unpack`] or respect the ranges.
    pub const fn pack(self) -> u32 {
        match self {
            FaultDecision::Proceed => 0,
            FaultDecision::Platform { kind, arg } => {
                debug_assert!(kind >= 1 && kind <= 63);
                debug_assert!(arg <= FAULT_ARG_MAX);
                (kind as u32) | ((arg & FAULT_ARG_MAX) << 8)
            }
            FaultDecision::Workload { kind, arg } => {
                debug_assert!(kind >= 64);
                debug_assert!(arg <= FAULT_ARG_MAX);
                (kind as u32) | ((arg & FAULT_ARG_MAX) << 8)
            }
        }
    }

    /// Unpack an `IN 0xD384` answer. Total: every u32 decodes to a decision.
    /// kind 0 is `Proceed` regardless of the arg bits (the spec defines `0` =
    /// Proceed; nonzero arg bits with kind 0 are RAZ noise and ignored).
    pub const fn unpack(v: u32) -> FaultDecision {
        let kind = (v & 0xFF) as u8;
        let arg = v >> 8;
        match kind {
            0 => FaultDecision::Proceed,
            1..=63 => FaultDecision::Platform { kind, arg },
            _ => FaultDecision::Workload { kind, arg },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_golden_values() {
        // Golden packed values from IMPLEMENTATION-PLAN M0 acceptance.
        assert_eq!(FaultDecision::Proceed.pack(), 0);
        assert_eq!(
            FaultDecision::Platform { kind: 2, arg: 512 }.pack(),
            0x0002_0002
        );
        assert_eq!(
            FaultDecision::Workload {
                kind: 200,
                arg: 0xFF_FFFF
            }
            .pack(),
            0xFFFF_FFC8
        );
    }

    #[test]
    fn unpack_inverts_pack() {
        let cases = [
            FaultDecision::Proceed,
            FaultDecision::Platform { kind: 1, arg: 0 },
            FaultDecision::Platform { kind: 2, arg: 512 },
            FaultDecision::Platform { kind: 3, arg: 1000 },
            FaultDecision::Platform {
                kind: 63,
                arg: FAULT_ARG_MAX,
            },
            FaultDecision::Workload { kind: 64, arg: 1 },
            FaultDecision::Workload {
                kind: 200,
                arg: 0xFF_FFFF,
            },
            FaultDecision::Workload { kind: 255, arg: 0 },
        ];
        for c in cases {
            assert_eq!(FaultDecision::unpack(c.pack()), c);
        }
    }

    #[test]
    fn kind_zero_is_always_proceed() {
        assert_eq!(FaultDecision::unpack(0xFFFF_FF00), FaultDecision::Proceed);
    }

    #[test]
    fn ident_value_encodes_magic_and_proto() {
        assert_eq!(IDENT_VALUE >> 16, 0xD37E);
        assert_eq!(IDENT_VALUE & 0xFFFF, 1);
    }

    #[test]
    fn ports_sit_inside_the_claimed_range() {
        for p in [
            PORT_IDENT,
            PORT_INIT_LO,
            PORT_INIT_HI,
            PORT_INIT_GO,
            PORT_DOORBELL,
            PORT_INJECT,
            PORT_QUIESCE_ACK,
        ] {
            assert!((PORT_RANGE_START..=PORT_RANGE_END).contains(&p));
        }
    }
}
