## Action Items

### Critical
- [ ] None.

### Important
- [ ] None.

### Suggestions
- [ ] [crates/detguest-wire/tests/golden_fixtures.rs:38-52] Add a `no_orphan_golden_fixtures` test that diffs the `tests/golden/` directory listing against the set of names live tests reference, so a renamed/removed case can't leave a stale `.bin` that silently stops being checked (the `GOLDEN_REGEN=1` rename path creates exactly this orphan).
- [ ] [crates/detguest-wire/tests/loom_ring.rs:118-172] Add a third loom model that pre-winds `prod`/`cons` to a multiple of `SIZE` just below `u32::MAX` before the threads run, so loom explores the publish/consume interleaving *across* the free-running u32 wrap — the property the SPSC research note flags as x86-invisible.
- [ ] [crates/detguest-wire/tests/loom_ring.rs:154-159] Add a comment that the model's record lengths (incl. 8-byte data records) are abstract index quanta, not wire lengths — in the real format an 8-byte record is only ever a `Pad` and real records are ≥16 bytes — so future edits don't assume model lengths track wire lengths.
- [ ] [fuzz/fuzz_targets/decode_record.rs:23-26] Loop `RegionEntry`/`Extent` `read_from` over the full `REGION_CAPACITY` (64) / `EXTENT_CAPACITY` (1024), or derive the slot index from a leading input byte, so the high-offset bounds paths get real fuzz coverage instead of only slots 0..4.
- [ ] [fuzz/fuzz_targets/decode_record.rs:9-14] Add a stream-walking fuzz body that decodes a record, advances by `hdr.len`, and re-decodes — exercising the multi-record / Pad-skip / unknown-kind-skip advance loop the host consumer actually runs, which the offset-0-only calls never reach.
- [ ] [crates/detguest-wire/tests/golden_fixtures.rs:40-44] Note in the module doc that `GOLDEN_REGEN` is not atomic (a panic mid-test leaves a partially regenerated tree); inspect the full `git diff` and re-run on failure.
- [ ] [crates/detguest-wire/tests/golden_fixtures.rs:220-236] Add a byte-pinned golden for a real record followed by an 8-byte tail `Pad` (a stream, as `Producer::try_push` emits at the ring end), so a regression in tail-pad placement shows up as a fixture diff rather than only being caught behaviorally.
