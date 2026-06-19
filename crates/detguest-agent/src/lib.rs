//! detguest-agent — the in-guest PID 1 agent (ARCHITECTURE.md §4).
//!
//! Lifecycle: mount /proc /sys devtmpfs hugetlbfs → allocate + initialize the
//! 2 MiB detchannel → resolve its GPA via pagemap → CHANNEL_INIT detcall →
//! `Hello` → parse `/etc/detguest/boot.toml` → autostart (agent-local; no
//! host input precedes READY) → `Ready` → single-threaded supervise loop
//! (pipes → LogLine, SIGCHLD → WorkloadExited, ring C commands).
//!
//! Unsafe policy (IMPLEMENTATION-PLAN M6, module-scoped): unsafe is permitted
//! only in the documented modules — [`pio`] (iopl + OUT/IN inline asm),
//! [`channel`] (hugetlbfs mmap of shared channel memory), and the libc
//! process/epoll plumbing in [`supervise`] and [`runtime`]. Everything else
//! inherits this crate-level deny. (`translate` needs no unsafe: pagemap is
//! plain file I/O.)
#![deny(unsafe_code)]

pub mod boot;
pub mod channel;
pub mod commands;
pub mod control;
pub mod pio;
pub mod runtime;
pub mod supervise;
pub mod translate;

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

/// Packed `Hello.agent_version` for this crate version.
pub fn agent_version() -> u32 {
    const fn parse_u8(s: &str) -> u8 {
        // const-friendly tiny parser for the env! version components
        let b = s.as_bytes();
        let mut v = 0u8;
        let mut i = 0;
        while i < b.len() {
            v = v * 10 + (b[i] - b'0');
            i += 1;
        }
        v
    }
    detguest_wire::events::pack_agent_version(
        parse_u8(env!("CARGO_PKG_VERSION_MAJOR")),
        parse_u8(env!("CARGO_PKG_VERSION_MINOR")),
        parse_u8(env!("CARGO_PKG_VERSION_PATCH")),
    )
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
        assert_eq!(n, 32);
        assert_eq!(buf[2], EventKind::Ready as u8);
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

    #[test]
    fn agent_version_packs() {
        assert_eq!(agent_version() >> 16, 0); // major 0 for now
    }
}
