//! Typed payloads and whole-record encode/decode for all three ring namespaces
//! (API.md §3.2 events, §3.3 commands, §3.4 workload-control).
//!
//! Decoders are total over arbitrary bytes: they return `Err`, never panic
//! (locked in by the `decode_record` fuzz target). Variable-length payloads
//! borrow from the input record — no allocation anywhere in this module.

use crate::record::{
    record_len, CommandKind, EventKind, RecordHeader, WorkloadCtrlKind, FLAG_TRUNCATED,
    MAX_RECORD_LEN, RECORD_HEADER_LEN,
};
use crate::{DecodeError, EncodeError};

/// Maximum `AssertViolation.details` length in bytes (clipped + TRUNCATED beyond).
pub const MAX_DETAILS: usize = 512;
/// Maximum `LogLine.msg` length in bytes (clipped + TRUNCATED beyond).
pub const MAX_LOG_MSG: usize = 1024;
/// Maximum `NameIntern.name` length in bytes (hard error beyond — names are
/// `'static` literals, not data).
pub const MAX_NAME: usize = 256;

/// `Hello.capabilities` bit 0: agent supports FORCED quiesce.
pub const CAP_FORCED_QUIESCE: u64 = 1 << 0;
/// `Hello.capabilities` bit 1: agent supports ReverifyRegions.
pub const CAP_REVERIFY_REGIONS: u64 = 1 << 1;

/// `LogLine.stream` values (API.md §3.2).
pub mod log_stream {
    /// Workload stdout, relayed by the agent.
    pub const STDOUT: u8 = 1;
    /// Workload stderr, relayed by the agent.
    pub const STDERR: u8 = 2;
    /// Agent-internal messages.
    pub const AGENT: u8 = 3;
    /// SDK `log_line` user messages.
    pub const SDK_USER: u8 = 4;
}

/// Pack an agent crate version for `Hello.agent_version`:
/// `major << 16 | minor << 8 | patch`.
pub const fn pack_agent_version(major: u8, minor: u8, patch: u8) -> u32 {
    ((major as u32) << 16) | ((minor as u32) << 8) | patch as u32
}

/// `Quiesce.mode` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QuiesceMode {
    /// Relay onto ring I; the SDK parks at its next `quiesce_check()`.
    Coop = 0,
    /// SIGSTOP the workload.
    Forced = 1,
}

/// `Shutdown.mode` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ShutdownMode {
    /// SIGTERM, 2 s virtual-time grace, SIGKILL, power off.
    Graceful = 0,
    /// Skip the grace period.
    Immediate = 1,
}

/// A decoded ring A/W event payload, borrowing variable-length fields from the
/// record bytes. Flag bits (TRUNCATED, REACHABLE_DECL) stay on the
/// [`RecordHeader`] — they are framing, not payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventPayload<'a> {
    /// Tail filler (kind 0); consumers skip it.
    Pad,
    /// Agent announce (kind 1).
    Hello {
        /// Must equal the channel header's proto_version (=1).
        proto_version: u32,
        /// Packed agent crate version (see [`pack_agent_version`]).
        agent_version: u32,
        /// [`CAP_FORCED_QUIESCE`] | [`CAP_REVERIFY_REGIONS`].
        capabilities: u64,
    },
    /// name → id binding (kind 2).
    NameIntern {
        /// Guest-local intern counter value, starts at 1.
        name_id: u32,
        /// UTF-8 name bytes (≤ [`MAX_NAME`], no NUL).
        name: &'a [u8],
    },
    /// `assert_always` violation (kind 3).
    AssertViolation {
        /// Interned assert name.
        name_id: u32,
        /// Per-name violation count including this one (1-based).
        violation_count: u32,
        /// UTF-8 details, ≤ [`MAX_DETAILS`] (TRUNCATED flag when clipped).
        details: &'a [u8],
    },
    /// First hit of a reachability name (kind 4).
    Reachable {
        /// Interned name.
        name_id: u32,
    },
    /// First hit of a coverage beacon (kind 5).
    Beacon {
        /// Beacon id (< 65536).
        beacon_id: u32,
    },
    /// `inject_point` query (kind 6).
    InjectQuery {
        /// Guest-local inject counter, starts at 0.
        iseq: u32,
        /// Interned inject-point name.
        name_id: u32,
    },
    /// Region published (kind 7).
    RegionRegister(RegionEvent),
    /// Region re-verified / unregistered (kind 8).
    RegionUpdate(RegionEvent),
    /// Workload exec'd (kind 9).
    WorkloadStarted {
        /// PID inside the guest.
        guest_pid: u32,
        /// Boot-manifest unit id that was launched.
        unit: u32,
    },
    /// Workload reaped (kind 10).
    WorkloadExited {
        /// PID inside the guest.
        guest_pid: u32,
        /// Exit code; -1 if killed by signal.
        exit_code: i32,
        /// Terminating signal; 0 on normal exit.
        term_signal: i32,
    },
    /// Structured log line (kind 11).
    LogLine {
        /// Stream id (see [`log_stream`]).
        stream: u8,
        /// 0 error … 4 trace.
        level: u8,
        /// UTF-8 message, ≤ [`MAX_LOG_MSG`] (TRUNCATED flag when clipped);
        /// invalid sequences lossily replaced by the producer.
        msg: &'a [u8],
    },
    /// Quiesce point reached (kind 12).
    QuiesceReady {
        /// Echo of the host's quiesce token.
        token: u64,
    },
    /// Frame boundary (kind 13).
    FrameMark {
        /// Emulated frame just completed; equals the FRAME_COUNTER MMIO value
        /// written immediately after this record (API.md §1.6 ordering rule).
        frame_index: u32,
    },
    /// The deterministic READY point (kind 14; ARCHITECTURE.md §4.1).
    Ready {
        /// Autostart unit started (`0xFFFF_FFFF` if none).
        unit: u32,
        /// Live regions in the manifest at emit time.
        region_count: u32,
        /// Manifest seqlock generation (even).
        manifest_generation: u64,
    },
}

/// Common payload of `RegionRegister` / `RegionUpdate` (kinds 7/8).
///
/// Full extents live in the manifest; the event is a notification + pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionEvent {
    /// Manifest slot index.
    pub region_id: u32,
    /// Interned region name.
    pub name_id: u32,
    /// Workload-declared layout version.
    pub layout_version: u32,
    /// Manifest generation after the update (even).
    pub manifest_generation: u32,
}

/// A decoded ring C command (API.md §3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Fork+exec preconfigured unit (kind 1).
    StartWorkload {
        /// Boot-manifest unit id.
        unit: u32,
        /// Initial LogLine mask.
        log_mask: u32,
    },
    /// Request a quiesce point (kind 2).
    Quiesce {
        /// Host-chosen token; `QuiesceReady` must echo it.
        token: u64,
        /// COOP or FORCED.
        mode: QuiesceMode,
    },
    /// Resume a FORCED-quiesced workload (kind 3).
    Resume {
        /// Token from the matching `Quiesce`.
        token: u64,
    },
    /// Kill workload, power off VM (kind 4).
    Shutdown {
        /// Graceful or immediate.
        mode: ShutdownMode,
    },
    /// Adjust LogLine production (kind 5).
    SetLogMask {
        /// New mask.
        mask: u32,
    },
    /// Re-walk pagemap for all live regions (kind 6).
    ReverifyRegions,
}

/// A decoded ring I workload-control record (API.md §3.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkloadCtrl {
    /// Quiesce relay (kind 2).
    QuiesceReq {
        /// Host-chosen token.
        token: u64,
    },
    /// Unpark from `quiesce_check` (kind 3).
    Resume {
        /// Token from the matching `QuiesceReq`.
        token: u64,
    },
}

// ---- little-endian field helpers ----

fn get_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn get_u64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

fn put_u32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn put_u64(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

/// Write a record header + fixed-size payload writer into `buf`.
fn encode_with(
    buf: &mut [u8],
    kind: u8,
    flags: u8,
    seq: u32,
    vnanos: u64,
    payload_len: usize,
    fill: impl FnOnce(&mut [u8]),
) -> Result<usize, EncodeError> {
    let total = record_len(payload_len);
    debug_assert!(total <= MAX_RECORD_LEN);
    if buf.len() < total {
        return Err(EncodeError::BufferTooSmall);
    }
    let hdr = RecordHeader {
        len: total as u16,
        kind,
        flags,
        seq,
        vnanos,
    };
    buf[..total].fill(0);
    hdr.write_to(buf)?;
    fill(&mut buf[RECORD_HEADER_LEN..RECORD_HEADER_LEN + payload_len]);
    Ok(total)
}

/// Encode an event record (rings A/W) into `buf`; returns total record length.
///
/// `extra_flags` is OR'd into the header flags (used for REACHABLE_DECL on
/// `NameIntern`). Over-cap `details`/`msg` are clipped and TRUNCATED is set;
/// an over-cap `name` is a hard [`EncodeError::FieldTooLong`].
pub fn encode_event(
    buf: &mut [u8],
    seq: u32,
    vnanos: u64,
    extra_flags: u8,
    ev: &EventPayload<'_>,
) -> Result<usize, EncodeError> {
    match *ev {
        EventPayload::Pad => {
            crate::record::encode_pad(buf, RECORD_HEADER_LEN, seq).map(|_| RECORD_HEADER_LEN)
        }
        EventPayload::Hello {
            proto_version,
            agent_version,
            capabilities,
        } => encode_with(
            buf,
            EventKind::Hello as u8,
            extra_flags,
            seq,
            vnanos,
            16,
            |p| {
                put_u32(p, 0, proto_version);
                put_u32(p, 4, agent_version);
                put_u64(p, 8, capabilities);
            },
        ),
        EventPayload::NameIntern { name_id, name } => {
            if name.len() > MAX_NAME {
                return Err(EncodeError::FieldTooLong);
            }
            encode_with(
                buf,
                EventKind::NameIntern as u8,
                extra_flags,
                seq,
                vnanos,
                8 + name.len(),
                |p| {
                    put_u32(p, 0, name_id);
                    p[4..6].copy_from_slice(&(name.len() as u16).to_le_bytes());
                    p[8..8 + name.len()].copy_from_slice(name);
                },
            )
        }
        EventPayload::AssertViolation {
            name_id,
            violation_count,
            details,
        } => {
            let clipped = &details[..core::cmp::min(details.len(), MAX_DETAILS)];
            let flags = extra_flags
                | if clipped.len() < details.len() {
                    FLAG_TRUNCATED
                } else {
                    0
                };
            encode_with(
                buf,
                EventKind::AssertViolation as u8,
                flags,
                seq,
                vnanos,
                16 + clipped.len(),
                |p| {
                    put_u32(p, 0, name_id);
                    put_u32(p, 4, violation_count);
                    p[8..10].copy_from_slice(&(clipped.len() as u16).to_le_bytes());
                    p[16..16 + clipped.len()].copy_from_slice(clipped);
                },
            )
        }
        EventPayload::Reachable { name_id } => encode_with(
            buf,
            EventKind::Reachable as u8,
            extra_flags,
            seq,
            vnanos,
            8,
            |p| {
                put_u32(p, 0, name_id);
            },
        ),
        EventPayload::Beacon { beacon_id } => encode_with(
            buf,
            EventKind::Beacon as u8,
            extra_flags,
            seq,
            vnanos,
            8,
            |p| {
                put_u32(p, 0, beacon_id);
            },
        ),
        EventPayload::InjectQuery { iseq, name_id } => encode_with(
            buf,
            EventKind::InjectQuery as u8,
            extra_flags,
            seq,
            vnanos,
            8,
            |p| {
                put_u32(p, 0, iseq);
                put_u32(p, 4, name_id);
            },
        ),
        EventPayload::RegionRegister(r) | EventPayload::RegionUpdate(r) => {
            let kind = if matches!(ev, EventPayload::RegionRegister(_)) {
                EventKind::RegionRegister
            } else {
                EventKind::RegionUpdate
            };
            encode_with(buf, kind as u8, extra_flags, seq, vnanos, 16, |p| {
                put_u32(p, 0, r.region_id);
                put_u32(p, 4, r.name_id);
                put_u32(p, 8, r.layout_version);
                put_u32(p, 12, r.manifest_generation);
            })
        }
        EventPayload::WorkloadStarted { guest_pid, unit } => encode_with(
            buf,
            EventKind::WorkloadStarted as u8,
            extra_flags,
            seq,
            vnanos,
            8,
            |p| {
                put_u32(p, 0, guest_pid);
                put_u32(p, 4, unit);
            },
        ),
        EventPayload::WorkloadExited {
            guest_pid,
            exit_code,
            term_signal,
        } => encode_with(
            buf,
            EventKind::WorkloadExited as u8,
            extra_flags,
            seq,
            vnanos,
            16,
            |p| {
                put_u32(p, 0, guest_pid);
                put_u32(p, 4, exit_code as u32);
                put_u32(p, 8, term_signal as u32);
            },
        ),
        EventPayload::LogLine { stream, level, msg } => {
            let clipped = &msg[..core::cmp::min(msg.len(), MAX_LOG_MSG)];
            let flags = extra_flags
                | if clipped.len() < msg.len() {
                    FLAG_TRUNCATED
                } else {
                    0
                };
            encode_with(
                buf,
                EventKind::LogLine as u8,
                flags,
                seq,
                vnanos,
                8 + clipped.len(),
                |p| {
                    p[0] = stream;
                    p[1] = level;
                    p[2..4].copy_from_slice(&(clipped.len() as u16).to_le_bytes());
                    p[8..8 + clipped.len()].copy_from_slice(clipped);
                },
            )
        }
        EventPayload::QuiesceReady { token } => encode_with(
            buf,
            EventKind::QuiesceReady as u8,
            extra_flags,
            seq,
            vnanos,
            8,
            |p| {
                put_u64(p, 0, token);
            },
        ),
        EventPayload::FrameMark { frame_index } => encode_with(
            buf,
            EventKind::FrameMark as u8,
            extra_flags,
            seq,
            vnanos,
            8,
            |p| {
                put_u32(p, 0, frame_index);
            },
        ),
        EventPayload::Ready {
            unit,
            region_count,
            manifest_generation,
        } => encode_with(
            buf,
            EventKind::Ready as u8,
            extra_flags,
            seq,
            vnanos,
            16,
            |p| {
                put_u32(p, 0, unit);
                put_u32(p, 4, region_count);
                put_u64(p, 8, manifest_generation);
            },
        ),
    }
}

/// Decode one event record (rings A/W) from `bytes` (which must start at a
/// record boundary). Returns the header and the typed payload.
///
/// Unknown kinds return [`DecodeError::UnknownKind`]; the caller still advances
/// by the framed `len` (forward compatibility, API.md §3.5).
pub fn decode_event(bytes: &[u8]) -> Result<(RecordHeader, EventPayload<'_>), DecodeError> {
    let hdr = RecordHeader::read_from(bytes)?;
    let payload = &bytes[hdr.payload_range()];
    let kind = EventKind::from_u8(hdr.kind).ok_or(DecodeError::UnknownKind(hdr.kind))?;
    let ev = match kind {
        EventKind::Pad => EventPayload::Pad,
        EventKind::Hello => {
            if payload.len() < 16 {
                return Err(DecodeError::BadLen);
            }
            EventPayload::Hello {
                proto_version: get_u32(payload, 0),
                agent_version: get_u32(payload, 4),
                capabilities: get_u64(payload, 8),
            }
        }
        EventKind::NameIntern => {
            if payload.len() < 8 {
                return Err(DecodeError::BadLen);
            }
            let name_len = u16::from_le_bytes(payload[4..6].try_into().unwrap()) as usize;
            if name_len > MAX_NAME || 8 + name_len > payload.len() {
                return Err(DecodeError::BadField);
            }
            EventPayload::NameIntern {
                name_id: get_u32(payload, 0),
                name: &payload[8..8 + name_len],
            }
        }
        EventKind::AssertViolation => {
            if payload.len() < 16 {
                return Err(DecodeError::BadLen);
            }
            let details_len = u16::from_le_bytes(payload[8..10].try_into().unwrap()) as usize;
            if details_len > MAX_DETAILS || 16 + details_len > payload.len() {
                return Err(DecodeError::BadField);
            }
            EventPayload::AssertViolation {
                name_id: get_u32(payload, 0),
                violation_count: get_u32(payload, 4),
                details: &payload[16..16 + details_len],
            }
        }
        EventKind::Reachable => {
            if payload.len() < 8 {
                return Err(DecodeError::BadLen);
            }
            EventPayload::Reachable {
                name_id: get_u32(payload, 0),
            }
        }
        EventKind::Beacon => {
            if payload.len() < 8 {
                return Err(DecodeError::BadLen);
            }
            EventPayload::Beacon {
                beacon_id: get_u32(payload, 0),
            }
        }
        EventKind::InjectQuery => {
            if payload.len() < 8 {
                return Err(DecodeError::BadLen);
            }
            EventPayload::InjectQuery {
                iseq: get_u32(payload, 0),
                name_id: get_u32(payload, 4),
            }
        }
        EventKind::RegionRegister | EventKind::RegionUpdate => {
            if payload.len() < 16 {
                return Err(DecodeError::BadLen);
            }
            let r = RegionEvent {
                region_id: get_u32(payload, 0),
                name_id: get_u32(payload, 4),
                layout_version: get_u32(payload, 8),
                manifest_generation: get_u32(payload, 12),
            };
            if kind == EventKind::RegionRegister {
                EventPayload::RegionRegister(r)
            } else {
                EventPayload::RegionUpdate(r)
            }
        }
        EventKind::WorkloadStarted => {
            if payload.len() < 8 {
                return Err(DecodeError::BadLen);
            }
            EventPayload::WorkloadStarted {
                guest_pid: get_u32(payload, 0),
                unit: get_u32(payload, 4),
            }
        }
        EventKind::WorkloadExited => {
            if payload.len() < 16 {
                return Err(DecodeError::BadLen);
            }
            EventPayload::WorkloadExited {
                guest_pid: get_u32(payload, 0),
                exit_code: get_u32(payload, 4) as i32,
                term_signal: get_u32(payload, 8) as i32,
            }
        }
        EventKind::LogLine => {
            if payload.len() < 8 {
                return Err(DecodeError::BadLen);
            }
            let msg_len = u16::from_le_bytes(payload[2..4].try_into().unwrap()) as usize;
            if msg_len > MAX_LOG_MSG || 8 + msg_len > payload.len() {
                return Err(DecodeError::BadField);
            }
            EventPayload::LogLine {
                stream: payload[0],
                level: payload[1],
                msg: &payload[8..8 + msg_len],
            }
        }
        EventKind::QuiesceReady => {
            if payload.len() < 8 {
                return Err(DecodeError::BadLen);
            }
            EventPayload::QuiesceReady {
                token: get_u64(payload, 0),
            }
        }
        EventKind::FrameMark => {
            if payload.len() < 8 {
                return Err(DecodeError::BadLen);
            }
            EventPayload::FrameMark {
                frame_index: get_u32(payload, 0),
            }
        }
        EventKind::Ready => {
            if payload.len() < 16 {
                return Err(DecodeError::BadLen);
            }
            EventPayload::Ready {
                unit: get_u32(payload, 0),
                region_count: get_u32(payload, 4),
                manifest_generation: get_u64(payload, 8),
            }
        }
    };
    Ok((hdr, ev))
}

/// Encode a ring C command record. Host-produced: `vnanos` is always 0
/// (the input log carries the icount — API.md §3.3).
pub fn encode_command(buf: &mut [u8], seq: u32, cmd: &Command) -> Result<usize, EncodeError> {
    match *cmd {
        Command::StartWorkload { unit, log_mask } => {
            encode_with(buf, CommandKind::StartWorkload as u8, 0, seq, 0, 8, |p| {
                put_u32(p, 0, unit);
                put_u32(p, 4, log_mask);
            })
        }
        Command::Quiesce { token, mode } => {
            encode_with(buf, CommandKind::Quiesce as u8, 0, seq, 0, 16, |p| {
                put_u64(p, 0, token);
                put_u32(p, 8, mode as u32);
            })
        }
        Command::Resume { token } => {
            encode_with(buf, CommandKind::Resume as u8, 0, seq, 0, 8, |p| {
                put_u64(p, 0, token);
            })
        }
        Command::Shutdown { mode } => {
            encode_with(buf, CommandKind::Shutdown as u8, 0, seq, 0, 8, |p| {
                put_u32(p, 0, mode as u32);
            })
        }
        Command::SetLogMask { mask } => {
            encode_with(buf, CommandKind::SetLogMask as u8, 0, seq, 0, 8, |p| {
                put_u32(p, 0, mask);
            })
        }
        Command::ReverifyRegions => encode_with(
            buf,
            CommandKind::ReverifyRegions as u8,
            0,
            seq,
            0,
            0,
            |_| {},
        ),
    }
}

/// Decode one ring C command record.
pub fn decode_command(bytes: &[u8]) -> Result<(RecordHeader, Command), DecodeError> {
    let hdr = RecordHeader::read_from(bytes)?;
    let payload = &bytes[hdr.payload_range()];
    let kind = CommandKind::from_u8(hdr.kind).ok_or(DecodeError::UnknownKind(hdr.kind))?;
    let cmd = match kind {
        CommandKind::StartWorkload => {
            if payload.len() < 8 {
                return Err(DecodeError::BadLen);
            }
            Command::StartWorkload {
                unit: get_u32(payload, 0),
                log_mask: get_u32(payload, 4),
            }
        }
        CommandKind::Quiesce => {
            if payload.len() < 16 {
                return Err(DecodeError::BadLen);
            }
            let mode = match get_u32(payload, 8) {
                0 => QuiesceMode::Coop,
                1 => QuiesceMode::Forced,
                _ => return Err(DecodeError::BadField),
            };
            Command::Quiesce {
                token: get_u64(payload, 0),
                mode,
            }
        }
        CommandKind::Resume => {
            if payload.len() < 8 {
                return Err(DecodeError::BadLen);
            }
            Command::Resume {
                token: get_u64(payload, 0),
            }
        }
        CommandKind::Shutdown => {
            if payload.len() < 8 {
                return Err(DecodeError::BadLen);
            }
            let mode = match get_u32(payload, 0) {
                0 => ShutdownMode::Graceful,
                1 => ShutdownMode::Immediate,
                _ => return Err(DecodeError::BadField),
            };
            Command::Shutdown { mode }
        }
        CommandKind::SetLogMask => {
            if payload.len() < 8 {
                return Err(DecodeError::BadLen);
            }
            Command::SetLogMask {
                mask: get_u32(payload, 0),
            }
        }
        CommandKind::ReverifyRegions => Command::ReverifyRegions,
    };
    Ok((hdr, cmd))
}

/// Encode a ring I workload-control record (host- or agent-produced; `vnanos`
/// is 0 for host pushes, the agent's virtual time for the quiesce relay).
pub fn encode_workload_ctrl(
    buf: &mut [u8],
    seq: u32,
    vnanos: u64,
    rec: &WorkloadCtrl,
) -> Result<usize, EncodeError> {
    match *rec {
        WorkloadCtrl::QuiesceReq { token } => encode_with(
            buf,
            WorkloadCtrlKind::QuiesceReq as u8,
            0,
            seq,
            vnanos,
            8,
            |p| {
                put_u64(p, 0, token);
            },
        ),
        WorkloadCtrl::Resume { token } => encode_with(
            buf,
            WorkloadCtrlKind::Resume as u8,
            0,
            seq,
            vnanos,
            8,
            |p| {
                put_u64(p, 0, token);
            },
        ),
    }
}

/// Decode one ring I workload-control record.
pub fn decode_workload_ctrl(bytes: &[u8]) -> Result<(RecordHeader, WorkloadCtrl), DecodeError> {
    let hdr = RecordHeader::read_from(bytes)?;
    let payload = &bytes[hdr.payload_range()];
    let kind = WorkloadCtrlKind::from_u8(hdr.kind).ok_or(DecodeError::UnknownKind(hdr.kind))?;
    let rec = match kind {
        WorkloadCtrlKind::QuiesceReq => {
            if payload.len() < 8 {
                return Err(DecodeError::BadLen);
            }
            WorkloadCtrl::QuiesceReq {
                token: get_u64(payload, 0),
            }
        }
        WorkloadCtrlKind::Resume => {
            if payload.len() < 8 {
                return Err(DecodeError::BadLen);
            }
            WorkloadCtrl::Resume {
                token: get_u64(payload, 0),
            }
        }
    };
    Ok((hdr, rec))
}

/// Convenience: total encoded length a payload will occupy on the wire,
/// after clipping (used by producers to check ring space before encoding).
pub fn encoded_event_len(ev: &EventPayload<'_>) -> usize {
    match *ev {
        EventPayload::Pad => RECORD_HEADER_LEN,
        EventPayload::Hello { .. }
        | EventPayload::RegionRegister(_)
        | EventPayload::RegionUpdate(_)
        | EventPayload::WorkloadExited { .. }
        | EventPayload::Ready { .. } => record_len(16),
        EventPayload::NameIntern { name, .. } => record_len(8 + name.len()),
        EventPayload::AssertViolation { details, .. } => {
            record_len(16 + core::cmp::min(details.len(), MAX_DETAILS))
        }
        EventPayload::LogLine { msg, .. } => record_len(8 + core::cmp::min(msg.len(), MAX_LOG_MSG)),
        EventPayload::Reachable { .. }
        | EventPayload::Beacon { .. }
        | EventPayload::InjectQuery { .. }
        | EventPayload::WorkloadStarted { .. }
        | EventPayload::QuiesceReady { .. }
        | EventPayload::FrameMark { .. } => record_len(8),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::FLAG_REACHABLE_DECL;

    fn round_trip(ev: EventPayload<'_>) {
        let mut buf = [0u8; MAX_RECORD_LEN];
        let n = encode_event(&mut buf, 5, 1234, 0, &ev).unwrap();
        assert_eq!(n, encoded_event_len(&ev));
        assert_eq!(n % 8, 0);
        let (hdr, back) = decode_event(&buf[..n]).unwrap();
        assert_eq!(hdr.len as usize, n);
        assert_eq!(hdr.seq, 5);
        assert_eq!(back, ev);
    }

    #[test]
    fn all_event_kinds_round_trip() {
        round_trip(EventPayload::Hello {
            proto_version: 1,
            agent_version: pack_agent_version(0, 1, 0),
            capabilities: CAP_FORCED_QUIESCE | CAP_REVERIFY_REGIONS,
        });
        round_trip(EventPayload::NameIntern {
            name_id: 1,
            name: b"hp_within_max",
        });
        round_trip(EventPayload::AssertViolation {
            name_id: 1,
            violation_count: 3,
            details: b"hp=120 max=100",
        });
        round_trip(EventPayload::Reachable { name_id: 9 });
        round_trip(EventPayload::Beacon { beacon_id: 0xFFFF });
        round_trip(EventPayload::InjectQuery {
            iseq: 0,
            name_id: 2,
        });
        round_trip(EventPayload::RegionRegister(RegionEvent {
            region_id: 0,
            name_id: 4,
            layout_version: 1,
            manifest_generation: 2,
        }));
        round_trip(EventPayload::RegionUpdate(RegionEvent {
            region_id: 1,
            name_id: 5,
            layout_version: 2,
            manifest_generation: 4,
        }));
        round_trip(EventPayload::WorkloadStarted {
            guest_pid: 2,
            unit: 0,
        });
        round_trip(EventPayload::WorkloadExited {
            guest_pid: 2,
            exit_code: -1,
            term_signal: 9,
        });
        round_trip(EventPayload::LogLine {
            stream: log_stream::STDOUT,
            level: 2,
            msg: b"hi",
        });
        round_trip(EventPayload::QuiesceReady { token: u64::MAX });
        round_trip(EventPayload::FrameMark { frame_index: 60 });
        round_trip(EventPayload::Ready {
            unit: 0xFFFF_FFFF,
            region_count: 0,
            manifest_generation: 2,
        });
    }

    #[test]
    fn commands_round_trip() {
        let cases = [
            Command::StartWorkload {
                unit: 0,
                log_mask: 0x1F,
            },
            Command::Quiesce {
                token: 7,
                mode: QuiesceMode::Coop,
            },
            Command::Quiesce {
                token: 7,
                mode: QuiesceMode::Forced,
            },
            Command::Resume { token: 7 },
            Command::Shutdown {
                mode: ShutdownMode::Graceful,
            },
            Command::Shutdown {
                mode: ShutdownMode::Immediate,
            },
            Command::SetLogMask { mask: 3 },
            Command::ReverifyRegions,
        ];
        let mut buf = [0u8; 64];
        for c in cases {
            let n = encode_command(&mut buf, 1, &c).unwrap();
            let (hdr, back) = decode_command(&buf[..n]).unwrap();
            assert_eq!(hdr.vnanos, 0, "host records carry vnanos 0");
            assert_eq!(back, c);
        }
    }

    #[test]
    fn workload_ctrl_round_trips() {
        let mut buf = [0u8; 32];
        for rec in [
            WorkloadCtrl::QuiesceReq { token: 1 },
            WorkloadCtrl::Resume { token: 1 },
        ] {
            let n = encode_workload_ctrl(&mut buf, 0, 50, &rec).unwrap();
            let (_, back) = decode_workload_ctrl(&buf[..n]).unwrap();
            assert_eq!(back, rec);
        }
    }

    #[test]
    fn long_details_clip_and_set_truncated() {
        let details = [b'x'; MAX_DETAILS + 100];
        let mut buf = [0u8; MAX_RECORD_LEN];
        let n = encode_event(
            &mut buf,
            0,
            0,
            0,
            &EventPayload::AssertViolation {
                name_id: 1,
                violation_count: 1,
                details: &details,
            },
        )
        .unwrap();
        let (hdr, ev) = decode_event(&buf[..n]).unwrap();
        assert!(hdr.flags & FLAG_TRUNCATED != 0);
        match ev {
            EventPayload::AssertViolation { details, .. } => assert_eq!(details.len(), MAX_DETAILS),
            _ => panic!("wrong kind"),
        }
    }

    #[test]
    fn long_log_msg_clips_and_max_size_fits() {
        let msg = [b'm'; MAX_LOG_MSG + 1];
        let mut buf = [0u8; MAX_RECORD_LEN];
        let n = encode_event(
            &mut buf,
            0,
            0,
            0,
            &EventPayload::LogLine {
                stream: 4,
                level: 0,
                msg: &msg,
            },
        )
        .unwrap();
        assert!(n <= MAX_RECORD_LEN);
        let (hdr, ev) = decode_event(&buf[..n]).unwrap();
        assert!(hdr.flags & FLAG_TRUNCATED != 0);
        match ev {
            EventPayload::LogLine { msg, .. } => assert_eq!(msg.len(), MAX_LOG_MSG),
            _ => panic!("wrong kind"),
        }
    }

    #[test]
    fn over_long_name_is_a_hard_error() {
        let name = [b'n'; MAX_NAME + 1];
        let mut buf = [0u8; MAX_RECORD_LEN];
        assert_eq!(
            encode_event(
                &mut buf,
                0,
                0,
                0,
                &EventPayload::NameIntern {
                    name_id: 1,
                    name: &name
                }
            ),
            Err(EncodeError::FieldTooLong)
        );
    }

    #[test]
    fn reachable_decl_flag_passes_through() {
        let mut buf = [0u8; 64];
        let n = encode_event(
            &mut buf,
            0,
            0,
            FLAG_REACHABLE_DECL,
            &EventPayload::NameIntern {
                name_id: 2,
                name: b"goal",
            },
        )
        .unwrap();
        let (hdr, _) = decode_event(&buf[..n]).unwrap();
        assert!(hdr.flags & FLAG_REACHABLE_DECL != 0);
    }

    #[test]
    fn unknown_kind_reports_but_frames() {
        let mut buf = [0u8; 24];
        RecordHeader {
            len: 24,
            kind: 200,
            flags: 0,
            seq: 0,
            vnanos: 0,
        }
        .write_to(&mut buf)
        .unwrap();
        assert_eq!(decode_event(&buf), Err(DecodeError::UnknownKind(200)));
        // The header itself still frames, so consumers can skip by len.
        assert_eq!(RecordHeader::read_from(&buf).unwrap().len, 24);
    }

    #[test]
    fn decoder_never_reads_past_declared_lengths() {
        // name_len pointing past the record is rejected, not read.
        let mut buf = [0u8; 32];
        let n = encode_event(
            &mut buf,
            0,
            0,
            0,
            &EventPayload::NameIntern {
                name_id: 1,
                name: b"abc",
            },
        )
        .unwrap();
        buf[RECORD_HEADER_LEN + 4] = 255; // forge name_len
        assert_eq!(decode_event(&buf[..n]), Err(DecodeError::BadField));
    }
}
