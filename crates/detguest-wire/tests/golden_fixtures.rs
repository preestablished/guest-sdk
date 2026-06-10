//! Golden binary fixtures (IMPLEMENTATION-PLAN M0 acceptance; API.md §3.5
//! "golden tests pin every byte of every v1 payload").
//!
//! One checked-in `tests/golden/*.bin` per record kind, plus the spec-named
//! edge fixtures: truncated AssertViolation details, `Pad` at ring tail,
//! max-size LogLine, dead manifest entry, and the packed `FaultDecision`
//! values. Each fixture is asserted byte-exact in BOTH directions:
//! `encode(x) == fixture` and `decode(fixture) == x`.
//!
//! Regenerate after an intentional format change with:
//! `GOLDEN_REGEN=1 cargo test -p detguest-wire --test golden_fixtures`
//! and commit the diff — a fixture diff IS a wire-format change and needs a
//! proto_version discussion per API.md §3.5.
//!
//! A few fixtures are additionally pinned as in-source literal byte arrays
//! derived by hand from API.md §3.0–§3.2, so the encoder is checked against
//! the spec text itself, not just against its own past output.

use detguest_wire::events::{
    decode_command, decode_event, decode_workload_ctrl, encode_command, encode_event,
    encode_workload_ctrl, pack_agent_version, Command, EventPayload, QuiesceMode, RegionEvent,
    ShutdownMode, WorkloadCtrl, CAP_FORCED_QUIESCE, CAP_REVERIFY_REGIONS, MAX_DETAILS, MAX_LOG_MSG,
};
use detguest_wire::header::ChannelHeader;
use detguest_wire::manifest::{
    init_manifest, Extent, RegionEntry, MANIFEST_TOTAL_SIZE, REGION_FLAG_DEAD, REGION_FLAG_HOT,
};
use detguest_wire::record::{encode_pad, FLAG_REACHABLE_DECL, FLAG_TRUNCATED, MAX_RECORD_LEN};
use detguest_wire::FaultDecision;

use std::path::PathBuf;

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

/// Compare (or regenerate) one fixture.
fn check(name: &str, bytes: &[u8]) {
    let path = golden_dir().join(name);
    if std::env::var_os("GOLDEN_REGEN").is_some_and(|v| v == "1") {
        std::fs::create_dir_all(golden_dir()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        return;
    }
    let fixture = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("missing golden fixture {name} ({e}); see module docs"));
    assert_eq!(
        bytes,
        fixture.as_slice(),
        "encode({name}) diverged from the checked-in golden bytes"
    );
}

fn encoded_event(seq: u32, vnanos: u64, flags: u8, ev: &EventPayload<'_>) -> Vec<u8> {
    let mut buf = [0u8; MAX_RECORD_LEN];
    let n = encode_event(&mut buf, seq, vnanos, flags, ev).unwrap();
    buf[..n].to_vec()
}

#[test]
fn event_fixtures_byte_exact() {
    let region = RegionEvent {
        region_id: 2,
        name_id: 7,
        layout_version: 1,
        manifest_generation: 4,
    };
    let long_details = vec![b'x'; MAX_DETAILS + 88]; // clipped to 512 + TRUNCATED
    let max_msg = vec![b'm'; MAX_LOG_MSG]; // exactly at cap, no flag
    let cases: Vec<(&str, u8, EventPayload<'_>)> = vec![
        (
            "hello.bin",
            0,
            EventPayload::Hello {
                proto_version: 1,
                agent_version: pack_agent_version(0, 1, 0),
                capabilities: CAP_FORCED_QUIESCE | CAP_REVERIFY_REGIONS,
            },
        ),
        (
            "name_intern.bin",
            0,
            EventPayload::NameIntern {
                name_id: 1,
                name: b"hp_within_max",
            },
        ),
        (
            "name_intern_decl.bin",
            FLAG_REACHABLE_DECL,
            EventPayload::NameIntern {
                name_id: 2,
                name: b"goal_reached",
            },
        ),
        (
            "assert_violation.bin",
            0,
            EventPayload::AssertViolation {
                name_id: 1,
                violation_count: 3,
                details: b"hp=120 max=100",
            },
        ),
        ("reachable.bin", 0, EventPayload::Reachable { name_id: 2 }),
        ("beacon.bin", 0, EventPayload::Beacon { beacon_id: 0x1234 }),
        (
            "inject_query.bin",
            0,
            EventPayload::InjectQuery {
                iseq: 9,
                name_id: 5,
            },
        ),
        (
            "region_register.bin",
            0,
            EventPayload::RegionRegister(region),
        ),
        ("region_update.bin", 0, EventPayload::RegionUpdate(region)),
        (
            "workload_started.bin",
            0,
            EventPayload::WorkloadStarted {
                guest_pid: 2,
                unit: 0,
            },
        ),
        (
            "workload_exited.bin",
            0,
            EventPayload::WorkloadExited {
                guest_pid: 2,
                exit_code: -1,
                term_signal: 9,
            },
        ),
        (
            "logline.bin",
            0,
            EventPayload::LogLine {
                stream: 1,
                level: 2,
                msg: b"boot: agent up",
            },
        ),
        (
            "quiesce_ready.bin",
            0,
            EventPayload::QuiesceReady { token: 0xDEAD_BEEF },
        ),
        (
            "frame_mark.bin",
            0,
            EventPayload::FrameMark { frame_index: 60 },
        ),
        (
            "ready.bin",
            0,
            EventPayload::Ready {
                unit: 0xFFFF_FFFF,
                region_count: 0,
                manifest_generation: 2,
            },
        ),
    ];
    for (name, flags, ev) in &cases {
        let bytes = encoded_event(5, 7, *flags, ev);
        check(name, &bytes);
        // Decode side: fixture bytes parse back to exactly this payload.
        let (hdr, back) = decode_event(&bytes).unwrap();
        assert_eq!(&back, ev, "{name}");
        assert_eq!(hdr.seq, 5);
        assert_eq!(hdr.vnanos, 7);
        assert_eq!(hdr.flags, *flags, "{name}");
    }

    // Spec-named edge fixtures.
    let bytes = encoded_event(
        5,
        7,
        0,
        &EventPayload::AssertViolation {
            name_id: 1,
            violation_count: 17,
            details: &long_details,
        },
    );
    check("assert_violation_truncated.bin", &bytes);
    let (hdr, back) = decode_event(&bytes).unwrap();
    assert!(hdr.flags & FLAG_TRUNCATED != 0);
    match back {
        EventPayload::AssertViolation { details, .. } => assert_eq!(details.len(), MAX_DETAILS),
        _ => panic!("wrong kind"),
    }

    let bytes = encoded_event(
        5,
        7,
        0,
        &EventPayload::LogLine {
            stream: 2,
            level: 0,
            msg: &max_msg,
        },
    );
    check("logline_max.bin", &bytes);
    let (hdr, back) = decode_event(&bytes).unwrap();
    assert_eq!(
        hdr.flags & FLAG_TRUNCATED,
        0,
        "exactly-at-cap is not truncated"
    );
    match back {
        EventPayload::LogLine { msg, .. } => assert_eq!(msg.len(), MAX_LOG_MSG),
        _ => panic!("wrong kind"),
    }
}

#[test]
fn pad_fixtures_byte_exact() {
    // Pad at an 8-byte ring tail: header prefix only, no vnanos field.
    let mut buf = [0xAAu8; 8];
    encode_pad(&mut buf, 8, 41).unwrap();
    check("pad_tail8.bin", &buf);
    // Hand-derived from API.md §3.0: len=8 LE, kind=0, flags=0, seq=41 LE.
    assert_eq!(buf, [0x08, 0x00, 0x00, 0x00, 41, 0x00, 0x00, 0x00]);

    // Pad covering a 40-byte tail: full header, zeroed body.
    let mut buf = [0xAAu8; 40];
    encode_pad(&mut buf, 40, 6).unwrap();
    check("pad_tail40.bin", &buf);
    let (hdr, ev) = decode_event(&buf).unwrap();
    assert_eq!(hdr.len, 40);
    assert_eq!(ev, EventPayload::Pad);
}

#[test]
fn command_fixtures_byte_exact() {
    let cases: Vec<(&str, Command)> = vec![
        (
            "cmd_start_workload.bin",
            Command::StartWorkload {
                unit: 0,
                log_mask: 0x1F,
            },
        ),
        (
            "cmd_quiesce_coop.bin",
            Command::Quiesce {
                token: 0x0123_4567_89AB_CDEF,
                mode: QuiesceMode::Coop,
            },
        ),
        (
            "cmd_quiesce_forced.bin",
            Command::Quiesce {
                token: 0x0123_4567_89AB_CDEF,
                mode: QuiesceMode::Forced,
            },
        ),
        (
            "cmd_resume.bin",
            Command::Resume {
                token: 0x0123_4567_89AB_CDEF,
            },
        ),
        (
            "cmd_shutdown_graceful.bin",
            Command::Shutdown {
                mode: ShutdownMode::Graceful,
            },
        ),
        (
            "cmd_shutdown_immediate.bin",
            Command::Shutdown {
                mode: ShutdownMode::Immediate,
            },
        ),
        ("cmd_set_log_mask.bin", Command::SetLogMask { mask: 0x3 }),
        ("cmd_reverify_regions.bin", Command::ReverifyRegions),
    ];
    for (name, cmd) in &cases {
        let mut buf = [0u8; 64];
        let n = encode_command(&mut buf, 1, cmd).unwrap();
        check(name, &buf[..n]);
        let (hdr, back) = decode_command(&buf[..n]).unwrap();
        assert_eq!(&back, cmd, "{name}");
        assert_eq!(hdr.vnanos, 0, "host records carry vnanos 0");
    }
}

#[test]
fn workload_ctrl_fixtures_byte_exact() {
    for (name, rec) in [
        ("wc_quiesce_req.bin", WorkloadCtrl::QuiesceReq { token: 99 }),
        ("wc_resume.bin", WorkloadCtrl::Resume { token: 99 }),
    ] {
        let mut buf = [0u8; 32];
        let n = encode_workload_ctrl(&mut buf, 0, 0, &rec).unwrap();
        check(name, &buf[..n]);
        let (_, back) = decode_workload_ctrl(&buf[..n]).unwrap();
        assert_eq!(back, rec, "{name}");
    }
}

#[test]
fn channel_header_fixture_byte_exact() {
    let h = ChannelHeader::canonical();
    let mut page = vec![0u8; 0x30];
    h.write_to(&mut page).unwrap();
    check("channel_header.bin", &page);
    assert_eq!(ChannelHeader::read_from(&page).unwrap(), h);
    // Hand-derived: magic spells "DETGUEST", proto 1 at 0x8.
    assert_eq!(&page[0..8], b"DETGUEST");
    assert_eq!(&page[8..12], &1u32.to_le_bytes());
}

#[test]
fn manifest_fixtures_byte_exact() {
    // A manifest area holding one live and one DEAD entry (the spec-named
    // "dead manifest entry" fixture) plus two extents.
    let mut area = vec![0u8; MANIFEST_TOTAL_SIZE];
    init_manifest(&mut area).unwrap();
    let live = RegionEntry {
        region_id: 0,
        name_id: 3,
        layout_version: 1,
        flags: REGION_FLAG_HOT,
        gva: 0x7000_0000_0000,
        len: 0x40_0000,
        extent_off: 0,
        extent_n: 2,
        name: RegionEntry::pack_name(b"wram").unwrap(),
    };
    let dead = RegionEntry {
        region_id: 1,
        name_id: 4,
        layout_version: 1,
        flags: REGION_FLAG_DEAD,
        gva: 0x7000_0040_0000,
        len: 0x1000,
        extent_off: 2,
        extent_n: 0,
        name: RegionEntry::pack_name(b"scratch").unwrap(),
    };
    live.write_to(&mut area, 0).unwrap();
    dead.write_to(&mut area, 1).unwrap();
    Extent {
        gpa: 0x1000_0000,
        len: 0x20_0000,
    }
    .write_to(&mut area, 0)
    .unwrap();
    Extent {
        gpa: 0x1080_0000,
        len: 0x20_0000,
    }
    .write_to(&mut area, 1)
    .unwrap();
    // Header counts: 1 live region (dead keeps its slot), 2 extents, gen 2.
    let mut hdr = detguest_wire::manifest::ManifestHeader::read_from(&area).unwrap();
    hdr.generation = 2;
    hdr.region_count = 1;
    hdr.extent_count = 2;
    hdr.write_to(&mut area).unwrap();

    check("manifest_area.bin", &area);

    let back_live = RegionEntry::read_from(&area, 0).unwrap();
    let back_dead = RegionEntry::read_from(&area, 1).unwrap();
    assert_eq!(back_live, live);
    assert_eq!(back_dead, dead);
    assert!(back_live.is_live());
    assert!(
        !back_dead.is_live(),
        "DEAD flag (bit 31) must mark the entry dead"
    );
}

#[test]
fn fault_decision_packed_golden_values() {
    // IMPLEMENTATION-PLAN M0 acceptance pins these packed values.
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
    assert_eq!(
        FaultDecision::unpack(0x0002_0002),
        FaultDecision::Platform { kind: 2, arg: 512 }
    );
    assert_eq!(
        FaultDecision::unpack(0xFFFF_FFC8),
        FaultDecision::Workload {
            kind: 200,
            arg: 0xFF_FFFF
        }
    );
}

/// In-source literals derived by hand from API.md §3.0–§3.2 — these check the
/// encoder against the spec text itself, independent of the .bin files.
#[test]
fn hand_derived_spec_literals() {
    // FrameMark, seq=5, vnanos=7, frame_index=60:
    // len=24 kind=13 flags=0 | seq=5 | vnanos=7 | frame_index=60 | 4 pad bytes.
    let bytes = encoded_event(5, 7, 0, &EventPayload::FrameMark { frame_index: 60 });
    #[rustfmt::skip]
    assert_eq!(bytes, [
        0x18, 0x00, 0x0D, 0x00, 0x05, 0x00, 0x00, 0x00,
        0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x3C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ]);

    // Ready, seq=5, vnanos=7, unit=0xFFFFFFFF, region_count=0, generation=2:
    // len=32 kind=14 | u32 unit | u32 region_count | u64 generation.
    let bytes = encoded_event(
        5,
        7,
        0,
        &EventPayload::Ready {
            unit: 0xFFFF_FFFF,
            region_count: 0,
            manifest_generation: 2,
        },
    );
    #[rustfmt::skip]
    assert_eq!(bytes, [
        0x20, 0x00, 0x0E, 0x00, 0x05, 0x00, 0x00, 0x00,
        0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x00,
        0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ]);

    // Hello, seq=0, vnanos=0, proto=1, agent_version=0.1.0, caps=3:
    // len=32 kind=1 | u32 proto | u32 version(0x100) | u64 caps.
    let bytes = encoded_event(
        0,
        0,
        0,
        &EventPayload::Hello {
            proto_version: 1,
            agent_version: pack_agent_version(0, 1, 0),
            capabilities: 3,
        },
    );
    #[rustfmt::skip]
    assert_eq!(bytes, [
        0x20, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x01, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
        0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ]);

    // WorkloadExited, pid=2, exit_code=-1, term_signal=9 — two's complement.
    let bytes = encoded_event(
        5,
        7,
        0,
        &EventPayload::WorkloadExited {
            guest_pid: 2,
            exit_code: -1,
            term_signal: 9,
        },
    );
    #[rustfmt::skip]
    assert_eq!(bytes, [
        0x20, 0x00, 0x0A, 0x00, 0x05, 0x00, 0x00, 0x00,
        0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x02, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF,
        0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ]);
}
