# Iteration 2 Review — Overview

- **Branch:** `ralph/iteration-2-workspace-mechanics-tests-vm-fuzz`
- **Base:** `main`
- **Date:** 2026-06-10
- **Reviewer:** Claude Opus

## Summary

This branch adds the M0 test and workspace infrastructure on top of the already-merged
`detguest-wire` crate: it joins `tests/vm` as the `detguest-vmtest` stub member with
KVM tests double-gated (`#[ignore]` + `DETGUEST_VM_TESTS=1`), excludes `fuzz/` as its own
workspace root, checks in 31 byte-exact golden binary fixtures with hand-derived spec
literals, adds a proptest round-trip suite, two loom models of the SPSC index protocol,
and a cargo-fuzz `decode_record` target (which already found and fixed a real decoder
panic on 8-byte tail `Pad`s). The two source-level changes — the `payload_range` empty-range
fix in `src/record.rs` and the miri-friendly load shrink in `src/ring.rs` — are both small,
correct, and well-commented. The golden fixtures and the in-source hand-derived literals
I cross-checked against API.md §3.0–§3.2 are byte-correct. `cargo test --workspace`,
`cargo clippy --workspace --all-targets`, and the loom suite all pass cleanly on this branch.

The work is high quality. The findings are non-blocking: the most notable is that the
proptest `arb_event()` strategy and the golden event-fixtures table omit the `Pad`
(kind 0) event from the *round-trip / strategy* coverage even though the prompt frames
this as "all 14+1 event kinds" — `Pad` is exercised separately (dedicated fixtures + the
regression test), so totality is covered, but the "+1" is not in the property generator.

## Verdict

**APPROVE**

The `payload_range` fix is sound for all reachable `len` values, the loom model faithfully
mirrors the real producer/consumer orderings, the fixtures are spec-correct, and the
workspace gating correctly prevents both KVM tests and the nightly fuzz crate from entering
hosted `--workspace` lanes. The suggestions below are polish, not blockers.

## Stats

- Files changed: 45 (`git diff main...HEAD --stat`), of which 31 are checked-in `.bin` fixtures.
- Non-fixture source/test additions: `golden_fixtures.rs` (482 lines), `proptest_roundtrip.rs`
  (285), `loom_ring.rs` (172), `tests/vm/src/lib.rs` (57), `fuzz/fuzz_targets/decode_record.rs` (35).
- Source changes: `src/record.rs` (+15/-1, the `payload_range` fix + regression test),
  `src/ring.rs` (+3/-1, miri load shrink). Plus 3 `Cargo.toml`/2 `Cargo.lock` mechanics.
- Commits on branch: 1 (`612f238 ralph: iteration 2 checkpoint - workspace mechanics + wire test suite`).
- Issue counts: Critical 0, Important 0, Suggestions 6.
- Quality gates run on branch: `cargo test --workspace` PASS, `cargo clippy --workspace
  --all-targets` PASS (clean), `RUSTFLAGS="--cfg loom" cargo test -p detguest-wire --test
  loom_ring --release` PASS (2 tests).
