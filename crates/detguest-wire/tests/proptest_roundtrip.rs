//! Round-trip property tests (IMPLEMENTATION-PLAN M0 acceptance):
//! `decode(encode(x)) == x` for every record kind in every ring namespace,
//! plus decoder totality over arbitrary bytes (complementing the fuzz target).

use detguest_wire::events::{
    self, decode_command, decode_event, decode_workload_ctrl, encode_command, encode_event,
    encode_workload_ctrl, encoded_event_len, Command, EventPayload, QuiesceMode, RegionEvent,
    ShutdownMode, WorkloadCtrl,
};
use detguest_wire::header::ChannelHeader;
use detguest_wire::manifest::{Extent, ManifestHeader, RegionEntry};
use detguest_wire::record::{FLAG_REACHABLE_DECL, MAX_RECORD_LEN};
use detguest_wire::FaultDecision;
use proptest::prelude::*;

fn arb_name() -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(any::<u8>(), 0..=events::MAX_NAME)
}

fn arb_details() -> impl Strategy<Value = Vec<u8>> {
    // Over-cap inputs are legal for encode (clip + TRUNCATED); cap here so the
    // round-trip compares equal, and test clipping separately below.
    proptest::collection::vec(any::<u8>(), 0..=events::MAX_DETAILS)
}

fn arb_msg() -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(any::<u8>(), 0..=events::MAX_LOG_MSG)
}

#[derive(Debug, Clone)]
enum OwnedEvent {
    Pad,
    Hello(u32, u32, u64),
    NameIntern(u32, Vec<u8>, bool),
    AssertViolation(u32, u32, Vec<u8>),
    Reachable(u32),
    Beacon(u32),
    InjectQuery(u32, u32),
    RegionRegister(RegionEvent),
    RegionUpdate(RegionEvent),
    WorkloadStarted(u32, u32),
    WorkloadExited(u32, i32, i32),
    LogLine(u8, u8, Vec<u8>),
    QuiesceReady(u64),
    FrameMark(u32),
    Ready(u32, u32, u64),
}

impl OwnedEvent {
    fn as_payload(&self) -> EventPayload<'_> {
        match self {
            OwnedEvent::Pad => EventPayload::Pad,
            OwnedEvent::Hello(p, a, c) => EventPayload::Hello {
                proto_version: *p,
                agent_version: *a,
                capabilities: *c,
            },
            OwnedEvent::NameIntern(id, name, _) => EventPayload::NameIntern { name_id: *id, name },
            OwnedEvent::AssertViolation(id, n, details) => EventPayload::AssertViolation {
                name_id: *id,
                violation_count: *n,
                details,
            },
            OwnedEvent::Reachable(id) => EventPayload::Reachable { name_id: *id },
            OwnedEvent::Beacon(id) => EventPayload::Beacon { beacon_id: *id },
            OwnedEvent::InjectQuery(iseq, id) => EventPayload::InjectQuery {
                iseq: *iseq,
                name_id: *id,
            },
            OwnedEvent::RegionRegister(r) => EventPayload::RegionRegister(*r),
            OwnedEvent::RegionUpdate(r) => EventPayload::RegionUpdate(*r),
            OwnedEvent::WorkloadStarted(pid, unit) => EventPayload::WorkloadStarted {
                guest_pid: *pid,
                unit: *unit,
            },
            OwnedEvent::WorkloadExited(pid, code, sig) => EventPayload::WorkloadExited {
                guest_pid: *pid,
                exit_code: *code,
                term_signal: *sig,
            },
            OwnedEvent::LogLine(stream, level, msg) => EventPayload::LogLine {
                stream: *stream,
                level: *level,
                msg,
            },
            OwnedEvent::QuiesceReady(t) => EventPayload::QuiesceReady { token: *t },
            OwnedEvent::FrameMark(f) => EventPayload::FrameMark { frame_index: *f },
            OwnedEvent::Ready(u, rc, g) => EventPayload::Ready {
                unit: *u,
                region_count: *rc,
                manifest_generation: *g,
            },
        }
    }

    fn extra_flags(&self) -> u8 {
        match self {
            OwnedEvent::NameIntern(_, _, true) => FLAG_REACHABLE_DECL,
            _ => 0,
        }
    }
}

fn arb_region_event() -> impl Strategy<Value = RegionEvent> {
    (any::<u32>(), any::<u32>(), any::<u32>(), any::<u32>()).prop_map(
        |(region_id, name_id, layout_version, manifest_generation)| RegionEvent {
            region_id,
            name_id,
            layout_version,
            manifest_generation,
        },
    )
}

fn arb_event() -> impl Strategy<Value = OwnedEvent> {
    prop_oneof![
        Just(OwnedEvent::Pad),
        (any::<u32>(), any::<u32>(), any::<u64>()).prop_map(|(p, a, c)| OwnedEvent::Hello(p, a, c)),
        (any::<u32>(), arb_name(), any::<bool>())
            .prop_map(|(id, n, d)| OwnedEvent::NameIntern(id, n, d)),
        (any::<u32>(), any::<u32>(), arb_details())
            .prop_map(|(id, n, d)| OwnedEvent::AssertViolation(id, n, d)),
        any::<u32>().prop_map(OwnedEvent::Reachable),
        any::<u32>().prop_map(OwnedEvent::Beacon),
        (any::<u32>(), any::<u32>()).prop_map(|(a, b)| OwnedEvent::InjectQuery(a, b)),
        arb_region_event().prop_map(OwnedEvent::RegionRegister),
        arb_region_event().prop_map(OwnedEvent::RegionUpdate),
        (any::<u32>(), any::<u32>()).prop_map(|(a, b)| OwnedEvent::WorkloadStarted(a, b)),
        (any::<u32>(), any::<i32>(), any::<i32>())
            .prop_map(|(a, b, c)| OwnedEvent::WorkloadExited(a, b, c)),
        (any::<u8>(), any::<u8>(), arb_msg()).prop_map(|(s, l, m)| OwnedEvent::LogLine(s, l, m)),
        any::<u64>().prop_map(OwnedEvent::QuiesceReady),
        any::<u32>().prop_map(OwnedEvent::FrameMark),
        (any::<u32>(), any::<u32>(), any::<u64>()).prop_map(|(u, r, g)| OwnedEvent::Ready(u, r, g)),
    ]
}

proptest! {
    #[test]
    fn event_round_trip(ev in arb_event(), seq in any::<u32>(), vnanos in any::<u64>()) {
        let payload = ev.as_payload();
        let mut buf = [0u8; MAX_RECORD_LEN];
        let n = encode_event(&mut buf, seq, vnanos, ev.extra_flags(), &payload).unwrap();
        prop_assert_eq!(n, encoded_event_len(&payload));
        prop_assert_eq!(n % 8, 0);
        let (hdr, back) = decode_event(&buf[..n]).unwrap();
        prop_assert_eq!(hdr.len as usize, n);
        prop_assert_eq!(hdr.seq, seq);
        // Pads are framing filler: encode_event routes them through
        // encode_pad, which always stamps vnanos = 0.
        let expect_vnanos = if matches!(ev, OwnedEvent::Pad) { 0 } else { vnanos };
        prop_assert_eq!(hdr.vnanos, expect_vnanos);
        prop_assert_eq!(back, payload);
        prop_assert_eq!(hdr.flags & FLAG_REACHABLE_DECL != 0,
                        matches!(ev, OwnedEvent::NameIntern(_, _, true)));
    }

    #[test]
    fn command_round_trip(
        pick in 0u8..6,
        unit in any::<u32>(),
        mask in any::<u32>(),
        token in any::<u64>(),
        forced in any::<bool>(),
        immediate in any::<bool>(),
        seq in any::<u32>(),
    ) {
        let cmd = match pick {
            0 => Command::StartWorkload { unit, log_mask: mask },
            1 => Command::Quiesce {
                token,
                mode: if forced { QuiesceMode::Forced } else { QuiesceMode::Coop },
            },
            2 => Command::Resume { token },
            3 => Command::Shutdown {
                mode: if immediate { ShutdownMode::Immediate } else { ShutdownMode::Graceful },
            },
            4 => Command::SetLogMask { mask },
            _ => Command::ReverifyRegions,
        };
        let mut buf = [0u8; 64];
        let n = encode_command(&mut buf, seq, &cmd).unwrap();
        let (hdr, back) = decode_command(&buf[..n]).unwrap();
        prop_assert_eq!(hdr.vnanos, 0);
        prop_assert_eq!(back, cmd);
    }

    #[test]
    fn workload_ctrl_round_trip(token in any::<u64>(), req in any::<bool>(), seq in any::<u32>(), vn in any::<u64>()) {
        let rec = if req {
            WorkloadCtrl::QuiesceReq { token }
        } else {
            WorkloadCtrl::Resume { token }
        };
        let mut buf = [0u8; 32];
        let n = encode_workload_ctrl(&mut buf, seq, vn, &rec).unwrap();
        let (_, back) = decode_workload_ctrl(&buf[..n]).unwrap();
        prop_assert_eq!(back, rec);
    }

    #[test]
    fn fault_decision_round_trip(v in any::<u32>()) {
        let d = FaultDecision::unpack(v);
        // Kind 0 erases the arg bits by spec; everything else round-trips exactly.
        if v & 0xFF == 0 {
            prop_assert_eq!(d, FaultDecision::Proceed);
            prop_assert_eq!(d.pack(), 0);
        } else {
            prop_assert_eq!(d.pack(), v);
        }
    }

    #[test]
    fn manifest_entry_round_trip(
        region_id in any::<u32>(),
        name_id in any::<u32>(),
        layout_version in any::<u32>(),
        flags in any::<u32>(),
        gva in any::<u64>(),
        len in any::<u64>(),
        extent_off in any::<u32>(),
        extent_n in any::<u32>(),
        name in proptest::collection::vec(1u8..=255, 0..=56),
        slot in 0usize..64,
    ) {
        let mut area = vec![0u8; detguest_wire::manifest::MANIFEST_TOTAL_SIZE];
        let e = RegionEntry {
            region_id, name_id, layout_version, flags, gva, len, extent_off, extent_n,
            name: RegionEntry::pack_name(&name).unwrap(),
        };
        e.write_to(&mut area, slot).unwrap();
        prop_assert_eq!(RegionEntry::read_from(&area, slot).unwrap(), e);
    }

    #[test]
    fn extent_round_trip(gpa in any::<u64>(), len in any::<u64>(), slot in 0usize..1024) {
        let mut area = vec![0u8; detguest_wire::manifest::MANIFEST_TOTAL_SIZE];
        let x = Extent { gpa, len };
        x.write_to(&mut area, slot).unwrap();
        prop_assert_eq!(Extent::read_from(&area, slot).unwrap(), x);
    }

    /// Decoder totality: arbitrary bytes never panic any decoder.
    /// (The cargo-fuzz target hammers this harder; this keeps the property in
    /// the default test suite.)
    #[test]
    fn decoders_are_total(bytes in proptest::collection::vec(any::<u8>(), 0..5000)) {
        let _ = decode_event(&bytes);
        let _ = decode_command(&bytes);
        let _ = decode_workload_ctrl(&bytes);
        let _ = detguest_wire::record::RecordHeader::read_from(&bytes);
        if let Ok(h) = ChannelHeader::read_from(&bytes) { let _ = h.validate(); }
        if let Ok(m) = ManifestHeader::read_from(&bytes) { let _ = m.validate(); }
    }

    /// Over-cap details/msg clip and set TRUNCATED (clipping path round-trips
    /// to the clipped value, never panics).
    #[test]
    fn clipping_sets_truncated(
        extra in 1usize..200,
        which in any::<bool>(),
        seq in any::<u32>(),
    ) {
        let mut buf = [0u8; MAX_RECORD_LEN];
        if which {
            let details = vec![0xAB; events::MAX_DETAILS + extra];
            let n = encode_event(&mut buf, seq, 0, 0, &EventPayload::AssertViolation {
                name_id: 1, violation_count: 1, details: &details,
            }).unwrap();
            let (hdr, ev) = decode_event(&buf[..n]).unwrap();
            prop_assert!(hdr.flags & detguest_wire::record::FLAG_TRUNCATED != 0);
            match ev {
                EventPayload::AssertViolation { details, .. } =>
                    prop_assert_eq!(details.len(), events::MAX_DETAILS),
                _ => prop_assert!(false),
            }
        } else {
            let msg = vec![0xCD; events::MAX_LOG_MSG + extra];
            let n = encode_event(&mut buf, seq, 0, 0, &EventPayload::LogLine {
                stream: 1, level: 0, msg: &msg,
            }).unwrap();
            let (hdr, ev) = decode_event(&buf[..n]).unwrap();
            prop_assert!(hdr.flags & detguest_wire::record::FLAG_TRUNCATED != 0);
            match ev {
                EventPayload::LogLine { msg, .. } =>
                    prop_assert_eq!(msg.len(), events::MAX_LOG_MSG),
                _ => prop_assert!(false),
            }
        }
    }
}
