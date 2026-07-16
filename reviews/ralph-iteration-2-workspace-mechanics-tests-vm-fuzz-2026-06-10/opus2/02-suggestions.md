# Suggestions (non-blocking)

All seven are coverage / maintainability hardening of the *test infrastructure itself*. None
affect the correctness of the shipped wire code.

---

## S1 — Golden suite has no orphan-fixture detection (silent coverage rot)

**File:** `crates/detguest-wire/tests/golden_fixtures.rs:38-52` (`check()`)

`check()` only ever *reads named* fixtures; nothing enumerates the `tests/golden/` directory to
assert every `.bin` on disk is referenced by a live test. If a case is renamed or removed (e.g.
`cmd_resume.bin` → `cmd_unpark.bin`), the old `.bin` lingers, is never asserted again, and `git`
will happily keep it. Today all 31 files are referenced (I scanned), but this is exactly the kind
of rot that goes unnoticed for months. The risk is amplified because `GOLDEN_REGEN=1` *writes*
the new name without deleting the old one — so a rename-and-regen leaves a stale orphan by
construction.

**Fix:** add a guard test that diffs the directory listing against the set of expected names.

```rust
#[test]
fn no_orphan_golden_fixtures() {
    let expected: std::collections::BTreeSet<&str> = [
        "hello.bin", "name_intern.bin", /* ... all 31 ... */
    ].into_iter().collect();
    let on_disk: std::collections::BTreeSet<String> = std::fs::read_dir(golden_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".bin"))
        .collect();
    let on_disk: std::collections::BTreeSet<&str> = on_disk.iter().map(String::as_str).collect();
    assert_eq!(on_disk, expected, "orphaned or missing golden fixture");
}
```

(Maintaining the literal set is a small cost; the alternative is having `check()` record each
name it touches into a shared set and a teardown that diffs — more machinery for the same effect.)

---

## S2 — Loom model never explores the u32 free-running wrap

**File:** `crates/detguest-wire/tests/loom_ring.rs:118-172`

Both loom models start from `prod = cons = 0` and push at most 3 records, so the indices never
approach `u32::MAX`. The free-running-index wrap (`prod.wrapping_sub(cons)` staying correct across
`2^32`) is precisely the subtle property the SPSC research note flags as "passes on x86, breaks on
ARM, caught by loom" — yet loom here never interleaves a *wrap* with a concurrent pop. The
single-threaded `free_running_indices_survive_u32_wrap` test (ring.rs:457-474) covers the math but
not the concurrent ordering. Consider a third loom model that pre-winds `prod`/`cons` to a
multiple of `SIZE` just below `u32::MAX` (mirroring the ring.rs test's `aligned` setup) before the
producer/consumer threads run, so loom explores the publish/consume interleaving across the wrap.

---

## S3 — Loom model uses 8-byte *data* records, which the real protocol forbids

**File:** `crates/detguest-wire/tests/loom_ring.rs:154-159`

`spsc_full_ring_unblocks_after_pop` pushes `try_push(16, 3)` and `try_push(8, 5)` as data. In the
real wire format an 8-byte record is *only* ever a `Pad` (kind 0); a real record is ≥16 bytes
(`MIN_RECORD_LEN`). The model also distinguishes pad-vs-data purely by `marker == 0`
(loom_ring.rs:85, 132), conflating "pad" with "marker-0 data" — benign only because no test pushes
marker 0. This is acceptable for an *index/ordering* abstraction (the placement math is delegated
to the real `bytes_needed`/`contiguous_tail`), but a reader could mistake the model for a faithful
length model. A one-line comment noting "lengths here are abstract index quanta, not real record
lengths; a real record is ≥16 and a pad is the only 8-byte record" would prevent that
misreading and keep future edits from assuming model lengths track wire lengths.

---

## S4 — Fuzz target only walks 4 of 64 region slots / 4 of 1024 extent slots

**File:** `fuzz/fuzz_targets/decode_record.rs:23-26`

`RegionEntry::read_from(data, i)` / `Extent::read_from(data, i)` are looped only for `i in 0..4`.
`REGION_CAPACITY` is 64 and `EXTENT_CAPACITY` is 1024. The high-index slots read *deeper* into the
input buffer (extent 1023 starts at offset `OFF_EXTENTS + 1023*16`), and short fuzz inputs rarely
reach those offsets, so the higher-offset bounds paths get little fuzz coverage. The code paths
are identical and *are* bounds-checked (verified: `i >= CAPACITY → BadField`,
`m.len() < at+SIZE → Truncated`), and proptest covers `slot in 0..64` / `0..1024`, so this is not
a soundness gap — but the fuzz target advertises "decoder totality" and quietly skips most of the
slot space. Cheap fix: loop `i` over the full capacities, or derive `i` from a leading byte of
`data` so the corpus minimizer can target boundary slots.

---

## S5 — Fuzz target never decodes at non-zero offsets / multi-record streams

**File:** `fuzz/fuzz_targets/decode_record.rs:9-14`

The three record decoders are only ever called on `data` starting at offset 0. The real consumer
loop decodes a *stream*: decode one record, advance by `hdr.len`, decode the next — including
skipping `Pad`s and unknown kinds by `len` (API.md §3.5). The fuzz target never exercises that
advance-and-redecode loop, so a bug in stream-walking (e.g. a record whose `len` advances past a
truncated tail) wouldn't be found here. Consider a second fuzz body that loops:
`while let Ok((hdr, _)) = decode_event(&data[off..]) { off += hdr.len as usize; if off >= data.len() { break } }`,
which is closer to how the host actually consumes a ring snapshot.

---

## S6 — `GOLDEN_REGEN` partial-regeneration on panic leaves a mixed tree

**File:** `crates/detguest-wire/tests/golden_fixtures.rs:40-44`

`check()` under `GOLDEN_REGEN=1` writes each fixture as it goes. If an `encode_*` call panics
partway through `event_fixtures_byte_exact` (15 cases in one test fn), the fixtures before the
panic are rewritten and the rest are not, leaving a half-regenerated `tests/golden/`. Because the
regen short-circuits *before* the encode in `check()` but the encode happens in the caller, this
is unlikely in practice, but a `git diff` after a failed regen could look deceptively partial.
Worth a one-line note in the module doc ("regen is not atomic; inspect the full `git diff` and
re-run on failure") so a future maintainer doesn't commit a partial regen. (Positive: the decode-
side assertions are *not* gated by `GOLDEN_REGEN`, so a regen run still validates the decode path —
that part is well done.)

---

## S7 — No byte-pinned golden for an 8-byte tail pad *inside a stream*

**File:** `crates/detguest-wire/tests/golden_fixtures.rs:220-236`

`pad_tail8.bin` / `pad_tail40.bin` pin standalone single pad records. The behaviorally-important
case — a sequence of real records terminated by an 8-byte tail pad, as actually produced by
`Producer::try_push` at the ring end — is exercised by `ring.rs::wrap_inserts_pad_*` but never
*byte-pinned* as a golden. Given the whole point of goldens is to make a wire-format change a
visible diff requiring a `proto_version` discussion (per the module doc and API.md §3.5), a pinned
"record + tail-pad" stream fixture would catch a regression in pad placement that the behavioral
test (which only checks seq monotonicity and payload equality) might let through. Low priority,
but cheap and on-theme.
