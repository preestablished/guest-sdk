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

    // Channel/manifest structure parsers used on the attach path.
    if let Ok(h) = detguest_wire::header::ChannelHeader::read_from(data) {
        let _ = h.validate();
    }
    if let Ok(m) = detguest_wire::manifest::ManifestHeader::read_from(data) {
        let _ = m.validate();
    }
    for i in 0..4 {
        let _ = detguest_wire::manifest::RegionEntry::read_from(data, i);
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
