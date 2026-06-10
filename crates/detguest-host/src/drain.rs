//! `drain_events`: pull complete records off rings A and W (API.md §2).

use detguest_wire::events::{decode_event, EventPayload, RegionEvent};
use detguest_wire::record::{
    FLAG_REACHABLE_DECL, FLAG_TRUNCATED, MAX_RECORD_LEN, PAD_MIN_LEN, RECORD_ALIGN,
};
use detguest_wire::{DecodeError, RingId};

use crate::channel::{Channel, InternEntry};
use crate::guestmem::{GuestMem, GuestMemExt};
use crate::{ChannelWriteSink, WireError};

/// A drained, typed guest event plus its framing metadata. The hypervisor
/// stamps it with the drain icount and its slot/lease identity on its side —
/// that stamp never enters guest memory (ARCHITECTURE.md §8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestEvent {
    /// Source ring (A or W).
    pub ring: RingId,
    /// Per-ring producer record counter.
    pub seq: u32,
    /// Guest virtual time (deterministic).
    pub vnanos: u64,
    /// Header TRUNCATED flag (details/msg clipped at its cap).
    pub truncated: bool,
    /// The decoded payload (owned).
    pub payload: OwnedPayload,
}

/// Owned mirror of [`detguest_wire::events::EventPayload`] for drained
/// events (`Pad` is consumed by the drain loop and never surfaces). String
/// fields stay raw bytes: UTF-8 is the producer's contract, not enforced on
/// the wire (consumers use lossy conversion).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum OwnedPayload {
    /// Agent announce (kind 1).
    Hello {
        /// Channel proto version (must be 1).
        proto_version: u32,
        /// Packed agent crate version.
        agent_version: u32,
        /// Capability bits.
        capabilities: u64,
    },
    /// name → id binding (kind 2).
    NameIntern {
        /// Intern id.
        name_id: u32,
        /// Raw name bytes.
        name: Vec<u8>,
        /// Emitted by `declare_reachable()` (header flag bit 1).
        reachable_decl: bool,
    },
    /// `assert_always` violation (kind 3).
    AssertViolation {
        /// Interned assert name.
        name_id: u32,
        /// Per-name count incl. this one.
        violation_count: u32,
        /// Raw details bytes.
        details: Vec<u8>,
    },
    /// First hit of a reachability name (kind 4).
    Reachable {
        /// Interned name.
        name_id: u32,
    },
    /// First hit of a coverage beacon (kind 5).
    Beacon {
        /// Beacon id.
        beacon_id: u32,
    },
    /// `inject_point` query (kind 6).
    InjectQuery {
        /// Guest-local inject counter.
        iseq: u32,
        /// Interned point name.
        name_id: u32,
    },
    /// Region published (kind 7).
    RegionRegister(RegionEvent),
    /// Region re-verified / unregistered (kind 8).
    RegionUpdate(RegionEvent),
    /// Workload exec'd (kind 9).
    WorkloadStarted {
        /// Guest PID.
        guest_pid: u32,
        /// Launched unit id.
        unit: u32,
    },
    /// Workload reaped (kind 10).
    WorkloadExited {
        /// Guest PID.
        guest_pid: u32,
        /// Exit code (-1 if signalled).
        exit_code: i32,
        /// Terminating signal (0 if normal).
        term_signal: i32,
    },
    /// Structured log line (kind 11).
    LogLine {
        /// Stream id.
        stream: u8,
        /// Level.
        level: u8,
        /// Raw message bytes.
        msg: Vec<u8>,
    },
    /// Quiesce point reached (kind 12).
    QuiesceReady {
        /// Echoed token.
        token: u64,
    },
    /// Frame boundary (kind 13).
    FrameMark {
        /// Completed frame index.
        frame_index: u32,
    },
    /// The deterministic READY point (kind 14).
    Ready {
        /// Autostart unit (0xFFFF_FFFF if none).
        unit: u32,
        /// Live regions at emit time.
        region_count: u32,
        /// Manifest generation (even).
        manifest_generation: u64,
    },
}

fn to_owned(flags: u8, p: &EventPayload<'_>) -> Option<OwnedPayload> {
    Some(match *p {
        EventPayload::Pad => return None,
        EventPayload::Hello {
            proto_version,
            agent_version,
            capabilities,
        } => OwnedPayload::Hello {
            proto_version,
            agent_version,
            capabilities,
        },
        EventPayload::NameIntern { name_id, name } => OwnedPayload::NameIntern {
            name_id,
            name: name.to_vec(),
            reachable_decl: flags & FLAG_REACHABLE_DECL != 0,
        },
        EventPayload::AssertViolation {
            name_id,
            violation_count,
            details,
        } => OwnedPayload::AssertViolation {
            name_id,
            violation_count,
            details: details.to_vec(),
        },
        EventPayload::Reachable { name_id } => OwnedPayload::Reachable { name_id },
        EventPayload::Beacon { beacon_id } => OwnedPayload::Beacon { beacon_id },
        EventPayload::InjectQuery { iseq, name_id } => OwnedPayload::InjectQuery { iseq, name_id },
        EventPayload::RegionRegister(r) => OwnedPayload::RegionRegister(r),
        EventPayload::RegionUpdate(r) => OwnedPayload::RegionUpdate(r),
        EventPayload::WorkloadStarted { guest_pid, unit } => {
            OwnedPayload::WorkloadStarted { guest_pid, unit }
        }
        EventPayload::WorkloadExited {
            guest_pid,
            exit_code,
            term_signal,
        } => OwnedPayload::WorkloadExited {
            guest_pid,
            exit_code,
            term_signal,
        },
        EventPayload::LogLine { stream, level, msg } => OwnedPayload::LogLine {
            stream,
            level,
            msg: msg.to_vec(),
        },
        EventPayload::QuiesceReady { token } => OwnedPayload::QuiesceReady { token },
        EventPayload::FrameMark { frame_index } => OwnedPayload::FrameMark { frame_index },
        EventPayload::Ready {
            unit,
            region_count,
            manifest_generation,
        } => OwnedPayload::Ready {
            unit,
            region_count,
            manifest_generation,
        },
    })
}

impl<M: GuestMem> Channel<M> {
    /// Drain all complete records from rings A and W (API.md §2). Bumps
    /// consumer indices through `sink`. Call ONLY while the vCPU is paused
    /// (pause boundary or inside a PIO exit). Returns events in (ring, seq)
    /// order — ring A fully, then ring W.
    ///
    /// Tolerances per spec: a record extending past the published producer
    /// index stops the drain at the last complete record (mid-write partial
    /// records are never partially decoded); unknown kinds are skipped by
    /// `len` and counted in [`Channel::unknown_kind_records`]; `Pad` records
    /// are consumed silently. `NameIntern` events are additionally folded
    /// into the channel's intern table, and `InjectQuery` events into the
    /// pending-inject table for [`crate::InjectResponder`].
    pub fn drain_events(
        &mut self,
        sink: &mut dyn ChannelWriteSink,
    ) -> Result<Vec<GuestEvent>, WireError> {
        let mut out = Vec::new();
        for ring in [RingId::A, RingId::W] {
            self.drain_ring(ring, sink, &mut out)?;
        }
        Ok(out)
    }

    fn drain_ring(
        &mut self,
        ring: RingId,
        sink: &mut dyn ChannelWriteSink,
        out: &mut Vec<GuestEvent>,
    ) -> Result<(), WireError> {
        let desc = self.ring_desc(ring);
        let size = desc.size;
        let mask = size - 1;
        let data = self.data_gpa(ring);
        let prod = self.gm.read_u32(self.prod_gpa(ring))?;
        let cons = self.gm.read_u32(self.cons_gpa(ring))?;
        let mut avail = prod.wrapping_sub(cons);
        if avail > size {
            return Err(WireError::CorruptIndices { ring });
        }
        let mut pos = cons;
        let mut rec = [0u8; MAX_RECORD_LEN];
        while avail >= PAD_MIN_LEN as u32 {
            let off = pos & mask;
            let tail = size - off;
            debug_assert!(tail >= PAD_MIN_LEN as u32, "positions are 8-aligned");
            // Peek the 8-byte prefix for the length.
            self.gm.read(data + off as u64, &mut rec[..PAD_MIN_LEN])?;
            let len = u16::from_le_bytes(rec[0..2].try_into().unwrap()) as u32;
            let kind = rec[2];
            let min = if kind == 0 {
                PAD_MIN_LEN
            } else {
                detguest_wire::MIN_RECORD_LEN
            } as u32;
            if len % RECORD_ALIGN as u32 != 0 || len < min || len as usize > MAX_RECORD_LEN {
                return Err(WireError::Decode(DecodeError::BadLen));
            }
            if len > avail {
                // Producer mid-write: stop at the last complete record.
                break;
            }
            if len > tail {
                // Records never wrap; a len crossing the ring end is corrupt.
                return Err(WireError::Decode(DecodeError::BadLen));
            }
            self.gm.read(data + off as u64, &mut rec[..len as usize])?;
            match decode_event(&rec[..len as usize]) {
                Ok((hdr, payload)) => {
                    if let Some(owned) = to_owned(hdr.flags, &payload) {
                        if let OwnedPayload::NameIntern {
                            name_id,
                            ref name,
                            reachable_decl,
                        } = owned
                        {
                            self.interns
                                .entry(name_id)
                                .and_modify(|e| e.reachable_decl |= reachable_decl)
                                .or_insert_with(|| InternEntry {
                                    name: String::from_utf8_lossy(name).into_owned(),
                                    reachable_decl,
                                });
                        }
                        if let OwnedPayload::InjectQuery { iseq, name_id } = owned {
                            self.pending_injects.insert(iseq, name_id);
                        }
                        out.push(GuestEvent {
                            ring,
                            seq: hdr.seq,
                            vnanos: hdr.vnanos,
                            truncated: hdr.flags & FLAG_TRUNCATED != 0,
                            payload: owned,
                        });
                    }
                }
                Err(DecodeError::UnknownKind(_)) => {
                    // Forward compatibility: skip by len, count (API.md §3.5).
                    self.unknown_kind_records += 1;
                }
                Err(e) => return Err(WireError::Decode(e)),
            }
            pos = pos.wrapping_add(len);
            avail -= len;
        }
        if pos != cons {
            self.gm.write_u32(self.cons_gpa(ring), pos)?;
            sink.cons_bump(ring, pos);
        }
        Ok(())
    }
}
