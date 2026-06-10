//! `Channel::attach` and the per-channel host state (API.md §2).

use std::collections::BTreeMap;

use detguest_wire::header::{ChannelHeader, RingDesc, OFF_RESERVED};
use detguest_wire::ports::InitStatus;
use detguest_wire::RingId;

use crate::guestmem::{GuestMem, GuestMemExt, MemError};

/// Why `Channel::attach` refused the guest's CHANNEL_INIT commit. The PIO
/// handler turns this into the nonzero status the guest reads via
/// `IN 0xD37C` (see [`AttachError::init_status`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AttachError {
    /// Header magic was not `"DETGUEST"`.
    BadMagic {
        /// The magic found at the latched GPA.
        found: u64,
    },
    /// Header proto_version was not 1.
    BadVersion {
        /// The version found.
        found: u32,
    },
    /// A ring descriptor points outside the channel page / overlaps the
    /// header+manifest area, or is misaligned.
    RingOutOfBounds {
        /// Which ring.
        ring: RingId,
    },
    /// A ring size is zero or not a power of two.
    BadRingSize {
        /// Which ring.
        ring: RingId,
    },
    /// Two ring data areas overlap (breaks SPSC single-ownership).
    RingsOverlap,
    /// The channel page (or part of it) is not mapped guest memory.
    Mem(MemError),
    /// Host policy: a channel is already attached for this VM. `attach`
    /// itself never returns this — the PIO handler owns per-VM attach state
    /// and uses it for the status mapping.
    AlreadyAttached,
}

impl From<MemError> for AttachError {
    fn from(e: MemError) -> AttachError {
        AttachError::Mem(e)
    }
}

impl AttachError {
    /// The `IN 0xD37C` status code for this error (API.md §5):
    /// 1 bad GPA, 2 bad magic/version (or malformed header), 3 already
    /// attached. Ring-descriptor problems are class 2 — the header at the
    /// GPA is readable but not a valid v1 channel header.
    pub fn init_status(&self) -> InitStatus {
        match self {
            AttachError::Mem(_) => InitStatus::BadGpa,
            AttachError::AlreadyAttached => InitStatus::AlreadyAttached,
            _ => InitStatus::BadMagicVersion,
        }
    }
}

/// One interned name (folded from `NameIntern` events — API.md §2).
#[derive(Debug, Clone)]
pub(crate) struct InternEntry {
    pub name: String,
    pub reachable_decl: bool,
}

/// Snapshot of the channel-header drop counters (guest-written; read-only
/// here — API.md §2 `drop_counters`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DropCounters {
    /// `ringA_dropped_records`.
    pub ring_a_records: u64,
    /// `ringA_dropped_bytes`.
    pub ring_a_bytes: u64,
    /// `ringW_dropped_records`.
    pub ring_w_records: u64,
    /// `ringW_dropped_bytes`.
    pub ring_w_bytes: u64,
    /// `ringW_dropped_by_kind[16]`, index = EventKind.
    pub ring_w_by_kind: [u64; 16],
}

/// An attached detchannel (API.md §2).
///
/// Host-side state lives outside guest RAM and must be checkpointed
/// alongside the hypervisor's per-branch state. Two classes:
/// - the intern table and pending-inject table are *reconstructible* from
///   the drained event stream (caching them avoids re-scans);
/// - the ring C/I producer seqs are **not** reconstructible — the host is
///   the producer there and never drains those rings. They MUST be saved
///   via [`Channel::producer_seqs`] and restored via
///   [`Channel::restore_producer_seqs`] after re-attaching on a snapshot
///   restore, or the next push would re-emit an already-used seq.
pub struct Channel<M: GuestMem> {
    pub(crate) gm: M,
    pub(crate) base: u64,
    pub(crate) header: ChannelHeader,
    pub(crate) interns: BTreeMap<u32, InternEntry>,
    /// iseq → name_id for InjectQuery events drained but not yet answered.
    pub(crate) pending_injects: BTreeMap<u32, u32>,
    /// Host-side producer record seqs for rings C and I (pads consume one
    /// too, matching `wire::ring`).
    pub(crate) next_seq_c: u32,
    pub(crate) next_seq_i: u32,
    /// Records skipped because their kind is unknown to this proto version
    /// (API.md §3.5: skip by len, count in a host metric).
    pub unknown_kind_records: u64,
    /// INJECT answers for an iseq with no matching drained query
    /// (API.md §5: answer Proceed + warning metric).
    pub unmatched_injects: u64,
    /// `NameIntern` re-binding an existing id to a *different* name
    /// (first-wins is kept; a nonzero count indicates a guest bug).
    pub intern_redefinitions: u64,
}

/// The host's ring C/I producer seqs — the non-reconstructible part of the
/// per-channel host state (see [`Channel`] docs). Checkpoint with the
/// hypervisor's per-branch state; restore after re-attach.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProducerSeqs {
    /// Next ring C record seq (pads consume one too).
    pub ring_c: u32,
    /// Next ring I record seq.
    pub ring_i: u32,
}

impl<M: GuestMem> std::fmt::Debug for Channel<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Channel")
            .field("base", &format_args!("{:#x}", self.base))
            .field("header", &self.header)
            .field("interns", &self.interns.len())
            .field("pending_injects", &self.pending_injects.len())
            .field("unknown_kind_records", &self.unknown_kind_records)
            .field("unmatched_injects", &self.unmatched_injects)
            .finish_non_exhaustive()
    }
}

impl<M: GuestMem> Channel<M> {
    /// Attach after the guest's CHANNEL_INIT detcall (API.md §2). Validates
    /// magic, proto_version (== 1), and ring descriptors (within the 2 MiB
    /// page, power-of-two sizes, mutually disjoint).
    pub fn attach(gm: M, base_gpa: u64) -> Result<Channel<M>, AttachError> {
        let mut hdr_bytes = [0u8; OFF_RESERVED];
        gm.read(base_gpa, &mut hdr_bytes)?;
        let header =
            ChannelHeader::read_from(&hdr_bytes).expect("fixed-size header buffer always parses");
        if header.magic != detguest_wire::CHANNEL_MAGIC {
            return Err(AttachError::BadMagic {
                found: header.magic,
            });
        }
        if header.proto_version != detguest_wire::PROTO_VERSION {
            return Err(AttachError::BadVersion {
                found: header.proto_version,
            });
        }
        for ring in RingId::ALL {
            let d = header.ring_desc[ring as usize];
            if d.size == 0 || !d.size.is_power_of_two() {
                return Err(AttachError::BadRingSize { ring });
            }
            if d.validate().is_err() {
                return Err(AttachError::RingOutOfBounds { ring });
            }
        }
        // Pairwise disjointness (same rule ChannelHeader::validate enforces,
        // surfaced as a distinct error for the PIO handler's logs).
        for i in 0..4 {
            for j in (i + 1)..4 {
                let (a, b) = (header.ring_desc[i], header.ring_desc[j]);
                let a_end = a.offset as u64 + a.size as u64;
                let b_end = b.offset as u64 + b.size as u64;
                if (a.offset as u64) < b_end && (b.offset as u64) < a_end {
                    return Err(AttachError::RingsOverlap);
                }
            }
        }
        Ok(Channel {
            gm,
            base: base_gpa,
            header,
            interns: BTreeMap::new(),
            pending_injects: BTreeMap::new(),
            next_seq_c: 0,
            next_seq_i: 0,
            unknown_kind_records: 0,
            unmatched_injects: 0,
            intern_redefinitions: 0,
        })
    }

    /// Export the ring C/I producer seqs for checkpointing. A fresh boot
    /// starts at (0, 0); after a snapshot restore the hypervisor must feed
    /// the checkpointed value back via [`Channel::restore_producer_seqs`] —
    /// `attach` alone cannot derive these (the host never drains C/I).
    pub fn producer_seqs(&self) -> ProducerSeqs {
        ProducerSeqs {
            ring_c: self.next_seq_c,
            ring_i: self.next_seq_i,
        }
    }

    /// Restore checkpointed ring C/I producer seqs after re-attaching.
    pub fn restore_producer_seqs(&mut self, seqs: ProducerSeqs) {
        self.next_seq_c = seqs.ring_c;
        self.next_seq_i = seqs.ring_i;
    }

    /// The validated channel header.
    pub fn header(&self) -> &ChannelHeader {
        &self.header
    }

    /// The channel base GPA.
    pub fn base_gpa(&self) -> u64 {
        self.base
    }

    /// Borrow the underlying guest memory.
    pub fn guest_mem(&self) -> &M {
        &self.gm
    }

    pub(crate) fn ring_desc(&self, ring: RingId) -> RingDesc {
        self.header.ring_desc[ring as usize]
    }

    pub(crate) fn prod_gpa(&self, ring: RingId) -> u64 {
        self.base + ring.prod_offset() as u64
    }

    pub(crate) fn cons_gpa(&self, ring: RingId) -> u64 {
        self.base + ring.cons_offset() as u64
    }

    pub(crate) fn data_gpa(&self, ring: RingId) -> u64 {
        self.base + self.ring_desc(ring).offset as u64
    }

    /// Resolve an interned `name_id` (folded from drained `NameIntern`
    /// events — API.md §2).
    ///
    /// The cached name is a **lossy** UTF-8 conversion; the raw bytes (which
    /// may differ for a non-UTF-8 producer) are on the original drained
    /// `NameIntern` event.
    pub fn intern_name(&self, id: u32) -> Option<&str> {
        self.interns.get(&id).map(|e| e.name.as_str())
    }

    /// name_ids interned with the REACHABLE_DECL flag (declared reachability
    /// targets — the orchestrator's "universe of targets", API.md §1.2).
    pub fn declared_reachables(&self) -> impl Iterator<Item = u32> + '_ {
        self.interns
            .iter()
            .filter(|(_, e)| e.reachable_decl)
            .map(|(id, _)| *id)
    }

    /// Current drop counters (guest-written; read-only here).
    ///
    /// Deliberate deviation from API.md §2's sketched infallible signature:
    /// the counters live in guest memory and `GuestMem` reads can fail, so
    /// the error is surfaced instead of swallowed (docs-as-built update
    /// tracked for the M6 documentation pass).
    pub fn drop_counters(&self) -> Result<DropCounters, MemError> {
        use detguest_wire::header as h;
        let mut c = DropCounters {
            ring_a_records: self
                .gm
                .read_u64(self.base + h::OFF_RING_A_DROPPED_RECORDS as u64)?,
            ring_a_bytes: self
                .gm
                .read_u64(self.base + h::OFF_RING_A_DROPPED_BYTES as u64)?,
            ring_w_records: self
                .gm
                .read_u64(self.base + h::OFF_RING_W_DROPPED_RECORDS as u64)?,
            ring_w_bytes: self
                .gm
                .read_u64(self.base + h::OFF_RING_W_DROPPED_BYTES as u64)?,
            ring_w_by_kind: [0; 16],
        };
        for (i, slot) in c.ring_w_by_kind.iter_mut().enumerate() {
            *slot = self
                .gm
                .read_u64(self.base + (h::OFF_RING_W_DROPPED_BY_KIND + i * 8) as u64)?;
        }
        Ok(c)
    }

    /// Take the pending `InjectQuery` for `iseq`, if drained (API.md §5).
    pub(crate) fn take_pending_inject(&mut self, iseq: u32) -> Option<u32> {
        self.pending_injects.remove(&iseq)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guestmem::MockGuestMem;
    use detguest_wire::header::CHANNEL_SIZE;

    pub(crate) const BASE: u64 = 0x1000_0000;

    pub(crate) fn fresh_channel_mem() -> MockGuestMem {
        let mut gm = MockGuestMem::with_zeroed(BASE, CHANNEL_SIZE);
        let mut hdr = [0u8; OFF_RESERVED];
        ChannelHeader::canonical().write_to(&mut hdr).unwrap();
        gm.write(BASE, &hdr).unwrap();
        gm
    }

    #[test]
    fn attach_accepts_canonical_header() {
        let ch = Channel::attach(fresh_channel_mem(), BASE).unwrap();
        assert_eq!(ch.header().proto_version, 1);
    }

    #[test]
    fn attach_maps_errors_to_init_status() {
        // Unmapped GPA → BadGpa(1)
        let gm = MockGuestMem::new();
        let e = Channel::attach(gm, BASE).unwrap_err();
        assert_eq!(e.init_status(), InitStatus::BadGpa);

        // Corrupt magic → BadMagicVersion(2)
        let mut gm = fresh_channel_mem();
        gm.write(BASE, &[0u8; 8]).unwrap();
        let e = Channel::attach(gm, BASE).unwrap_err();
        assert!(matches!(e, AttachError::BadMagic { .. }));
        assert_eq!(e.init_status(), InitStatus::BadMagicVersion);

        // Bad ring size → 2
        let mut gm = fresh_channel_mem();
        let mut hdr = ChannelHeader::canonical();
        hdr.ring_desc[3].size = 0x3000; // not a power of two
        let mut b = [0u8; OFF_RESERVED];
        hdr.write_to(&mut b).unwrap();
        gm.write(BASE, &b).unwrap();
        let e = Channel::attach(gm, BASE).unwrap_err();
        assert!(matches!(e, AttachError::BadRingSize { ring: RingId::W }));

        // Overlapping rings → RingsOverlap
        let mut gm = fresh_channel_mem();
        let mut hdr = ChannelHeader::canonical();
        hdr.ring_desc[1] = hdr.ring_desc[0];
        let mut b = [0u8; OFF_RESERVED];
        hdr.write_to(&mut b).unwrap();
        gm.write(BASE, &b).unwrap();
        assert!(matches!(
            Channel::attach(gm, BASE),
            Err(AttachError::RingsOverlap)
        ));

        // Policy variant maps to 3.
        assert_eq!(
            AttachError::AlreadyAttached.init_status(),
            InitStatus::AlreadyAttached
        );
    }
}
