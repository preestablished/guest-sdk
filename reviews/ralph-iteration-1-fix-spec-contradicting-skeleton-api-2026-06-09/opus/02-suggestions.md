# Suggestions (non-blocking)

### SUG-1 — `manual_range_contains` clippy warning in `try_push`

- **File:** `crates/detguest-wire/src/ring.rs:136-140`

```rust
debug_assert!(
    total_len % RECORD_ALIGN == 0
        && total_len >= PAD_MIN_LEN
        && total_len <= MAX_RECORD_LEN
);
```

clippy suggests the range form. It is a warning (not deny), but once IMP-1 is fixed
this is the only remaining clippy output, so clearing it gives a clean lint run:

```rust
debug_assert!(
    total_len % RECORD_ALIGN == 0
        && (PAD_MIN_LEN..=MAX_RECORD_LEN).contains(&total_len)
);
```

### SUG-2 — Redundant `RecordHeader::read_from` re-validation in `pop_into`

- **File:** `crates/detguest-wire/src/ring.rs:262-263`

```rust
// Validate the full header now that all bytes are local.
RecordHeader::read_from(&scratch[..len])?;
```

By the time this runs, `pop_into` has already independently re-derived and checked
`len` (alignment, `min`, `> MAX_RECORD_LEN`, `> avail`, `> tail`) from the local copy,
so `read_from` re-parses bytes that cannot now fail those same checks. It is harmless
(and arguably good defense-in-depth for the `vnanos`-prefix length branch), but the
comment overstates its role — it is not *the* validation, it is a second one. Either
drop it and rely on the inline checks, or keep it and reword the comment to
"defense-in-depth re-parse" so the next reader doesn't think the earlier checks are
removable. If kept, consider asserting the two `len` values agree under `debug_assert`.

### SUG-3 — Defensive `free()` underflow guard for consumer-derived occupancy

- **File:** `crates/detguest-wire/src/ring.rs:36-39`

```rust
pub const fn free(prod: u32, cons: u32, size: u32) -> u32 {
    size - used(prod, cons)
}
```

`free` subtracts `used` from `size`; if `used > size` this underflows (debug panic /
release wrap). On the producer path that cannot happen (the producer owns `prod` and
never over-advances). It is only a concern if a *forged/corrupted* index ever reaches
this function — and per `ARCHITECTURE.md` §2 the peer index a consumer acquires is
host-written channel memory. `pop_into` itself never calls `free` (it compares against
raw `used`), so there is no live hazard today. Still, since the crate's stated posture
is "arbitrary bytes never cause UB," a saturating form documents intent and is
panic-free even if a future caller passes a corrupt pair:

```rust
pub const fn free(prod: u32, cons: u32, size: u32) -> u32 {
    size.saturating_sub(used(prod, cons))
}
```

### SUG-4 — `FaultDecision::Platform { kind: 0, .. }` is constructible but lossy

- **File:** `crates/detguest-wire/src/ports.rs:100-107`

`pack()` maps any `Platform`/`Workload` with `kind == 0` to the packed value
`(0) | (arg << 8)`, which `unpack()` then reads back as `Proceed`, silently dropping
`arg`. The documented `Platform` kind range is `1..=63`, so `kind: 0` is caller
misuse, not a wire concern — but since the type lets it be built, a `debug_assert!(kind
!= 0)` in `pack` (or a doc line on the `Platform` variant restating the `1..=63`
invariant) would catch the mistake at the source instead of letting it round-trip into
a wrong-but-valid `Proceed`. Purely a hardening nicety.

### SUG-5 — Decoders accept non-UTF-8 / embedded-NUL names and messages

- **File:** `crates/detguest-wire/src/events.rs` (`NameIntern` :518-530, `LogLine`
  :605-618, `AssertViolation` :531-544)

The specs say `NameIntern.name` is "UTF-8, no NUL", `LogLine.msg` is UTF-8 with
"invalid sequences lossily replaced by the producer", and `AssertViolation.details`
is UTF-8. The decoders correctly borrow the raw bytes without validating UTF-8 or
NUL-freedom. That is the right call for a *total, allocation-free, borrowing* decoder
(validation belongs to the producer per the spec, and `&[u8]` is the honest return
type), so this is **not** a bug. Worth a one-line doc note on `decode_event` stating
"string fields are returned as raw bytes; UTF-8/NUL validity is the producer's
contract, not enforced here" so a host consumer doesn't assume `from_utf8` will always
succeed.

### SUG-6 — Tests assert round-trips but no byte-exact golden fixtures yet

- **File:** `crates/detguest-wire/src/events.rs` tests, and the manifest/header tests

The research note (`rust-nostd-wire-codecs`) is explicit: round-trip tests alone
cannot catch a wrong-but-symmetric layout; only byte-exact golden fixtures pin the
actual wire bytes. `API.md` §3.5 also mandates "Golden tests pin every byte of every
v1 payload." The current suite is round-trip + a few field-position asserts, which is
good but not the golden coverage the spec calls for. Recommend adding at least one
hard-coded `assert_eq!(&buf[..n], &[0x.., ..])` golden per v1 payload (and per
manifest entry/extent) before this layout ships, so future refactors are caught
against pinned bytes rather than self-consistent re-encodings. (Tracking-only; not a
blocker for a skeleton checkpoint.)
