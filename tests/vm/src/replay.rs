//! Comparable-run digests for the Ms5 `determinism_replay` gate
//! (`guest-sdk-m5-determinism-replay-ci-gate`).
//!
//! Surface ownership split (decided in the Ms5 groundwork round — do not
//! re-litigate in round 2): the hypervisor's `VerifyReplay` already proves
//! RAM/framebuffer `end_state_hash` bit-identity (DRL-4/DRL-5, handback at
//! dh `0831f92`); the guest-sdk gate owns the four surfaces the CI-gate bead
//! enumerates — S1 ring C/I pushes, S2 ring A/W consumer bumps, S3 pio
//! answers, S4 SDK event / drop-counter equivalence. Where the groundwork
//! request's surface wording ("the LogLine digest" family, RAM/framebuffer)
//! and the bead's differ, the bead + IMPLEMENTATION-PLAN wording wins: it is
//! the CI-gate contract.
//!
//! One FNV-1a-64 per surface (the repo's line-hash convention, previously
//! private to `m2_acceptance.rs` — [`fnv1a64_lines`] now lives here so the
//! third user doesn't copy it a third time).

use detguest_host::{DropCounters, GuestEvent, SinkOp};

/// FNV-1a-64 over newline-terminated lines (the m2/m3 golden-hash
/// convention).
pub fn fnv1a64_lines(lines: &[String]) -> u64 {
    let mut hash = Fnv::new();
    for line in lines {
        hash.update(line.as_bytes());
        hash.update(b"\n");
    }
    hash.finish()
}

/// Incremental FNV-1a-64 (same constants as [`fnv1a64_lines`], usable over
/// raw byte spans).
struct Fnv(u64);

impl Fnv {
    fn new() -> Fnv {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    fn update(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    fn update_u32(&mut self, v: u32) {
        self.update(&v.to_le_bytes());
    }
    fn update_u64(&mut self, v: u64) {
        self.update(&v.to_le_bytes());
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

/// One comparable run: a hash per gate surface. Two runs are bit-identical
/// (for the surfaces this repo owns) iff their digests are equal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunDigest {
    /// S1 — host ring C/I pushes (ring id, span bytes incl. pads, new_prod).
    pub s1_ring_pushes: u64,
    /// S2 — host ring A/W consumer bumps (ring id, new_cons).
    pub s2_cons_bumps: u64,
    /// S3 — pio answers (port, packed `FaultDecision`).
    pub s3_pio_answers: u64,
    /// S4 — normalized drained SDK events plus channel drop counters.
    pub s4_sdk_events: u64,
}

/// The first surface two digests diverge on (S1→S4 order). A 1000-iteration
/// failure that doesn't name its surface is undebuggable — this is the
/// gate's failure message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceMismatch {
    /// Human-readable surface name (e.g. `"S1 ring C/I pushes"`).
    pub surface: &'static str,
    /// The left run's hash for that surface.
    pub a: u64,
    /// The right run's hash for that surface.
    pub b: u64,
}

impl std::fmt::Display for SurfaceMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "surface {} diverged: {:#018x} != {:#018x}",
            self.surface, self.a, self.b
        )
    }
}

/// Equal, or the first divergent surface.
pub fn assert_digests_equal(a: &RunDigest, b: &RunDigest) -> Result<(), SurfaceMismatch> {
    for (surface, va, vb) in [
        ("S1 ring C/I pushes", a.s1_ring_pushes, b.s1_ring_pushes),
        ("S2 ring A/W cons bumps", a.s2_cons_bumps, b.s2_cons_bumps),
        ("S3 pio answers", a.s3_pio_answers, b.s3_pio_answers),
        ("S4 SDK events/drops", a.s4_sdk_events, b.s4_sdk_events),
    ] {
        if va != vb {
            return Err(SurfaceMismatch {
                surface,
                a: va,
                b: vb,
            });
        }
    }
    Ok(())
}

/// One normalized line per drained event: ring, seq, truncated flag, and the
/// payload's `Debug` form (total over every `OwnedPayload` variant, so no
/// event class silently escapes the digest).
pub fn normalized_event_lines(events: &[GuestEvent]) -> Vec<String> {
    events
        .iter()
        .map(|e| {
            format!(
                "{:?}:{}:{}:{:?}",
                e.ring,
                e.seq,
                u8::from(e.truncated),
                e.payload
            )
        })
        .collect()
}

/// Fold one run's observables into a [`RunDigest`]: S1–S3 from the
/// `RecordingSink` trace (the §C ordered-trace shape), S4 from the
/// normalized drained events plus the channel drop counters.
pub fn digest_from_trace(ops: &[SinkOp], events: &[GuestEvent], drops: &DropCounters) -> RunDigest {
    let mut s1 = Fnv::new();
    let mut s2 = Fnv::new();
    let mut s3 = Fnv::new();
    for op in ops {
        match op {
            SinkOp::RingPush {
                ring,
                bytes,
                new_prod,
            } => {
                s1.update(&[*ring as u8]);
                s1.update_u32(*new_prod);
                s1.update_u32(bytes.len() as u32);
                s1.update(bytes);
            }
            SinkOp::ConsBump { ring, new_cons } => {
                s2.update(&[*ring as u8]);
                s2.update_u32(*new_cons);
            }
            SinkOp::PioAnswer { port, value } => {
                s3.update(&port.to_le_bytes());
                s3.update_u32(*value);
            }
        }
    }
    let mut s4 = Fnv::new();
    s4.update_u64(fnv1a64_lines(&normalized_event_lines(events)));
    s4.update_u64(drops.ring_a_records);
    s4.update_u64(drops.ring_a_bytes);
    s4.update_u64(drops.ring_w_records);
    s4.update_u64(drops.ring_w_bytes);
    for k in drops.ring_w_by_kind {
        s4.update_u64(k);
    }
    RunDigest {
        s1_ring_pushes: s1.finish(),
        s2_cons_bumps: s2.finish(),
        s3_pio_answers: s3.finish(),
        s4_sdk_events: s4.finish(),
    }
}
