//! Decoder totality fuzz target (IMPLEMENTATION-PLAN M0 acceptance):
//! arbitrary bytes must decode to `Ok`/`Err` — never panic, never UB.
//!
//! Run: `cargo +nightly fuzz run decode_record` (30-minute CI gate).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Record-level decoders for all three ring namespaces.
    let _ = detguest_wire::events::decode_event(data);
    let _ = detguest_wire::events::decode_command(data);
    let _ = detguest_wire::events::decode_workload_ctrl(data);
    let _ = detguest_wire::record::RecordHeader::read_from(data);

    // Walk the input as a multi-record stream by framed lengths — the same
    // advance discipline ring drains use — so decoding at non-zero offsets
    // is exercised, not just offset 0.
    let mut off = 0usize;
    while off < data.len() {
        match detguest_wire::record::RecordHeader::read_from(&data[off..]) {
            Ok(h) => {
                let rec = &data[off..off + h.len as usize];
                let _ = detguest_wire::events::decode_event(rec);
                let _ = detguest_wire::events::decode_command(rec);
                let _ = detguest_wire::events::decode_workload_ctrl(rec);
                off += h.len as usize; // len >= 8, so this always advances
            }
            Err(_) => break,
        }
    }

    // Region-registration IPC datagram decoders (agent.sock).
    let _ = detguest_wire::regionipc::decode_request(data);
    let _ = detguest_wire::regionipc::decode_reply(data);

    // Channel/manifest structure parsers used on the attach path.
    if let Ok(h) = detguest_wire::header::ChannelHeader::read_from(data) {
        let _ = h.validate();
    }
    if let Ok(m) = detguest_wire::manifest::ManifestHeader::read_from(data) {
        let _ = m.validate();
    }
    for i in 0..detguest_wire::manifest::REGION_CAPACITY {
        let _ = detguest_wire::manifest::RegionEntry::read_from(data, i);
    }
    for i in [0, 1, 2, 3, 511, 1022, 1023] {
        let _ = detguest_wire::manifest::Extent::read_from(data, i);
    }

    // FaultDecision unpack is total by construction; pack(unpack(x)) must not
    // panic either (it may differ from x only in the ignored kind-0 arg bits).
    if data.len() >= 4 {
        let v = u32::from_le_bytes(data[..4].try_into().unwrap());
        let d = detguest_wire::FaultDecision::unpack(v);
        let _ = d.pack();
    }
});
