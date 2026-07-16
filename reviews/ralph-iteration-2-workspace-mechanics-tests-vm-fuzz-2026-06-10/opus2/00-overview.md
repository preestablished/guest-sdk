# Iteration 2 Review — Workspace Mechanics, Wire Test Suite, VM/Fuzz Scaffolding

- **Branch:** `ralph/iteration-2-workspace-mechanics-tests-vm-fuzz`
- **Base:** `main`
- **Date:** 2026-06-10
- **Reviewer:** Claude Opus (2nd reviewer)
- **Commit under review:** `612f238` (single checkpoint commit)

## Summary

This branch adds M0 test/workspace infrastructure on top of the iteration-1 `detguest-wire`
crate: a `tests/vm` stub workspace member (`detguest-vmtest`, KVM tests `#[ignore]` +
`DETGUEST_VM_TESTS=1` env-gated), an excluded `fuzz/` crate carrying the `decode_record`
cargo-fuzz target, 31 byte-exact golden `.bin` fixtures with hand-derived spec literals, a
proptest round-trip + decoder-totality suite, and loom models of the SPSC index protocol. The
only production change is a genuine bug fix in `RecordHeader::payload_range()`: an 8-byte tail
`Pad` previously returned `16..16`, out of bounds for the record's own 8 bytes, panicking
`decode_event` — found by the fuzz target and now covered by an in-source regression test. I
verified the fix is complete: I enumerated all three `payload_range()` decode call sites
(`decode_event`, `decode_command`, `decode_workload_ctrl`) and confirmed the empty-range case is
the *only* sub-16 case any of them can reach, because `read_from` rejects every non-`Pad` record
with `len < 16` before `payload_range` is ever called. The change is well-engineered, the
commentary is unusually disciplined (every non-obvious decision is justified against the spec),
and the test coverage is broad and largely correct. My findings are all non-blocking: they are
coverage/maintainability gaps in the *tests themselves* (orphan-fixture detection, loom never
exploring the u32-wrap interleaving, the fuzz target's narrow slot range), not correctness
defects in the shipped code.

## Verdict

**APPROVE**

The production fix is correct and complete; the new infrastructure builds, lints clean, and all
tests pass. Remaining items are test-coverage hardening that can land as follow-ups.

## Stats

- Files changed: 45 (`git diff main...HEAD --stat`), of which 31 are binary golden fixtures.
- Production code delta: `src/record.rs` (+5/-1, the `payload_range` fix + doc), `src/ring.rs`
  (test-only miri load-shrink), `Cargo.toml` / `crates/detguest-wire/Cargo.toml` (workspace +
  dev-deps).
- New test code: `golden_fixtures.rs` (482), `proptest_roundtrip.rs` (285), `loom_ring.rs` (172),
  `tests/vm/src/lib.rs` (57), `fuzz/fuzz_targets/decode_record.rs` (35).
- Verification I ran:
  - `cargo test --workspace` → wire 38 + golden 8 + proptest 8 + agent/client/vmtest all pass;
    `detguest-vmtest` shows `1 passed; 1 ignored`.
  - `cargo test -p detguest-vmtest -- --ignored` with **no** env var → the gated test passes soft
    (prints skip, returns `Ok`), as intended.
  - `cargo clippy --workspace --all-targets` → exit 0, no warnings.
  - `RUSTFLAGS="--cfg loom" cargo build -p detguest-wire --tests --release` → compiles clean.
  - Orphan scan: all 31 `.bin` files are currently referenced by name in `golden_fixtures.rs`.

## Findings count

- Critical: 0
- Important: 0
- Suggestions: 7
