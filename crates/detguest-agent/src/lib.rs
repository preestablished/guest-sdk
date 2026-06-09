//! detguest-agent — the in-guest PID 1 agent.
//!
//! M2 (IMPLEMENTATION-PLAN) fills this crate out: init mounts, hugetlbfs
//! channel allocation, pagemap self-translation, CHANNEL_INIT detcall, boot
//! manifest parsing, workload supervision. This skeleton carries the wire-side
//! helpers those modules share.
//!
//! Unsafe policy (IMPLEMENTATION-PLAN M6): module-scoped. Unsafe is permitted
//! only in the documented modules — `translate` (pagemap GVA→GPA), and the
//! channel-mapping/PIO paths when they land — each carrying its own
//! `#![allow(unsafe_code)]` with a safety argument. Everything else inherits
//! this crate-level deny. (Crate-level `forbid` would make those modules
//! unwritable — deny is deliberate.)
#![deny(unsafe_code)]

use detguest_wire::events::{encode_event, encoded_event_len, EventPayload};
use detguest_wire::EncodeError;

/// Encode a spec-correct `Ready` record (EventKind 14, API.md §3.2) into `buf`,
/// returning the record length.
///
/// `unit` is the autostart unit started (`0xFFFF_FFFF` if none),
/// `region_count` the live regions at emit time, and `manifest_generation` the
/// manifest's even seqlock generation. The READY-point contract for *when* the
/// agent may emit this is ARCHITECTURE.md §4.1.
pub fn ready_record(
    buf: &mut [u8],
    seq: u32,
    vnanos: u64,
    unit: u32,
    region_count: u32,
    manifest_generation: u64,
) -> Result<usize, EncodeError> {
    let ev = EventPayload::Ready {
        unit,
        region_count,
        manifest_generation,
    };
    debug_assert!(buf.len() >= encoded_event_len(&ev));
    encode_event(buf, seq, vnanos, 0, &ev)
}

#[cfg(test)]
mod tests {
    use super::*;
    use detguest_wire::events::EventPayload;
    use detguest_wire::record::EventKind;

    #[test]
    fn ready_record_is_spec_correct() {
        let mut buf = [0u8; 64];
        let n = ready_record(&mut buf, 3, 1_000_000, 0xFFFF_FFFF, 0, 2).unwrap();
        // 16-byte header + 16-byte Ready payload (API.md §3.2) — not the old
        // ad-hoc 9-byte encoding this skeleton used to carry.
        assert_eq!(n, 32);
        assert_eq!(buf[2], EventKind::Ready as u8); // kind 14, not READY_RECORD=1
        let (hdr, ev) = detguest_wire::events::decode_event(&buf[..n]).unwrap();
        assert_eq!(hdr.seq, 3);
        assert_eq!(
            ev,
            EventPayload::Ready {
                unit: 0xFFFF_FFFF,
                region_count: 0,
                manifest_generation: 2
            }
        );
    }
}
