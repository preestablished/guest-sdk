# Critical & Important Issues

No **Critical** (must-fix-before-merge, correctness-breaking) issues were found.
The wire format matches both specs byte-for-byte everywhere I checked, the SPSC
ring discipline is sound, and the decoders are total (no panics on the paths I
traced). The items below are **Important** (should-fix) — they are about
spec-mandated test coverage and a couple of latent soundness/robustness gaps,
not about bytes currently going out wrong.

---

## IMPORTANT

### I-1. Spec mandates byte-exact golden fixtures; only round-trip tests exist
**Severity:** Important
**Files:** `crates/detguest-wire/src/events.rs` (tests, ~L837–1078); `manifest.rs`, `header.rs`, `ports.rs` (tests)
**Spec:** `API.md` §3.5 line 522 — *"Golden tests pin every byte of every v1 payload."*

This is a normative requirement, not a nicety. Every test in the crate is a
`decode(encode(x)) == x` round-trip (`events::tests::all_event_kinds_round_trip`,
`commands_round_trip`, `header::tests::canonical_header_round_trips_and_validates`,
`manifest::tests::entry_and_extent_round_trip`, …). As the project's own research
note (`rust-nostd-wire-codecs.md`) states: *"Round-trip tests alone can't catch
wrong-but-symmetric layouts; golden fixtures pin the actual bytes."* If, say, the
`Ready` encoder and decoder both swapped `unit` and `region_count`, every current
test would still pass while the host (a separate codebase) would read garbage. The
whole point of this crate is bit-for-bit agreement between two independently
compiled sides (`lib.rs` L4), so the symmetric-bug class is exactly the risk.

This is the only reason `Ready` (the determinism root-snapshot key) is currently
verified only against itself rather than against pinned bytes.

**Suggested fix** — add per-payload golden byte assertions, e.g.:

```rust
#[test]
fn ready_golden_bytes() {
    let mut buf = [0u8; 32];
    let n = encode_event(&mut buf, 3, 0x0F_4240, 0,
        &EventPayload::Ready { unit: 0xFFFF_FFFF, region_count: 0, manifest_generation: 2 }).unwrap();
    assert_eq!(n, 32);
    assert_eq!(&buf[..32], &[
        // header: len=32, kind=14, flags=0, seq=3, vnanos=1_000_000
        0x20,0x00, 0x0E, 0x00, 0x03,0x00,0x00,0x00, 0x40,0x42,0x0F,0x00,0x00,0x00,0x00,0x00,
        // payload: unit=0xFFFFFFFF, region_count=0, manifest_generation=2
        0xFF,0xFF,0xFF,0xFF, 0x00,0x00,0x00,0x00, 0x02,0x00,0x00,0x00,0x00,0x00,0x00,0x00,
    ]);
}
```

Repeat for at least one record per ring namespace and for the manifest header +
region entry + extent. A pinned `CHANNEL_MAGIC`/`MANIFEST_MAGIC` byte test already
exists (`magic_bytes_spell_detguest`) — extend that style to payloads.

---

### I-2. `RingDesc::validate` permits ring-to-ring overlap (aliasing rings)
**Severity:** Important
**File:** `crates/detguest-wire/src/header.rs:204–217` (`RingDesc::validate`), `305–316` (`ChannelHeader::validate`)
**Spec:** `API.md` §2 / L309 — attach validates "ring descriptors (within the 2 MiB page, power-of-two sizes)".

`validate()` checks each descriptor in isolation: power-of-two, nonzero,
`off >= OFF_RING_C_DATA`, `off+size <= CHANNEL_SIZE`, 8-aligned. It never checks
that the four ring data areas are mutually disjoint. A header that declares, say,
ring W `{offset: 0x8000, size: 0x4000}` (the canonical ring-C area) passes
`ChannelHeader::validate()` cleanly. The host attach path then maps two SPSC rings
onto the same bytes, breaking the single-producer/single-consumer ownership
argument that the entire `ring.rs` soundness story (L17–22) rests on.

The spec wording is loose here ("within the page"), so this is not strictly a
spec contradiction — but `validate` is explicitly the attach-time gate against a
malformed/hostile header, and overlap is the one malformation it lets through.
Since the layout is canonical and fixed in v1, the cheapest correct fix is to
require canonical placement (the crate already has `RingId::canonical_desc()`):

```rust
// In ChannelHeader::validate, after per-descriptor checks:
for id in RingId::ALL {
    if self.ring_desc[id as usize] != id.canonical_desc() {
        return Err(DecodeError::BadField);
    }
}
```

If non-canonical-but-valid layouts must stay legal, instead add an explicit
pairwise non-overlap check across the four `[off, off+size)` ranges.

---

### I-3. `size >= MAX_RECORD_LEN` is the load-bearing ring invariant but only a `debug_assert`
**Severity:** Important
**File:** `crates/detguest-wire/src/ring.rs:96` (`Producer::from_raw`), `205` (`Consumer::from_raw`)
**Research:** `rust-unsafe-review.md` — *"bounds that protect unsafe code must be real asserts or checked errors; debug_assert disappears in release."*

`from_raw` documents (correctly) that `size` must be a power of two and
`>= MAX_RECORD_LEN`, but enforces it only via `debug_assert!`. In a release build
a caller that maps a ring smaller than 4096 gets no diagnostic, and the
never-deadlock argument for critical events (a MAX-sized record + tail pad always
fits an empty ring **only because** `size >= 2*MAX` is comfortably true for all
real rings, and minimally because pad triggers only when `tail < len`) silently
loses its footing for tiny `size`. This is an `unsafe fn` so it is technically the
caller's contract — but because the value flows directly into pointer/length math
that produces `&mut [u8]`, a real assert (or a fallible constructor) is cheap
insurance against a release-only soundness footgun.

**Suggested fix:** keep the `unsafe fn` but make the invariant a hard guard, e.g.
`assert!(size.is_power_of_two() && size as usize >= MAX_RECORD_LEN);` (real
`assert!`, not `debug_assert!`), or expose a safe checked constructor that returns
`Result`. The cost is one compare on a cold setup path.
