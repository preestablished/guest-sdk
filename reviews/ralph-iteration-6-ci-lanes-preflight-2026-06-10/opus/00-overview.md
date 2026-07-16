# CI Lanes + Intel Preflight — Review Overview

- **Branch:** `ralph/iteration-6-ci-lanes-preflight`
- **Base:** `main`
- **Date:** 2026-06-10
- **Reviewer:** Claude Opus
- **Scope:** 3 files, +251 lines
  - `.github/workflows/ci.yaml` (new, 129 lines)
  - `.github/workflows/fuzz.yaml` (new, 37 lines)
  - `scripts/intel-preflight.sh` (new, 85 lines)

## Summary

This change wires up the M0 CI tiering described in IMPLEMENTATION-PLAN: a hosted
lane (fmt/clippy/test), the `no_std` acceptance lane (lcv), miri-over-ring (51r),
loom interleavings (wq8), a static musl cross-build with a static-linkage assertion
(ssl), an aarch64 wire+host lane (xcc), and an Intel-only in-VM tier (rci) fronted by
a preflight gate (atd). A separate nightly `fuzz.yaml` runs the 30-minute clean
`decode_record` libFuzzer gate (m7g). The mechanics are largely correct and idiomatic:
dual-checkout for the `../control-plane` path dep is consistently applied to every job,
`defaults.run.working-directory: guest-sdk` lines up with the checkout layout, the
`#![cfg(loom)]` gate matches the loom job's `RUSTFLAGS`, the musl static-linkage grep
and `--check` smoke run are well-conceived, and the preflight script's FAIL-accumulation
pattern is sound and passes live on this machine. The dominant problem is a single,
serious supply-chain exposure: the in-VM job runs on a **self-hosted runner** triggered
by **`pull_request`** in a **public** repo, which lets arbitrary fork-PR code execute on
the Intel box. There are also two genuine correctness gaps (the `cargo install cargo-fuzz`
step is not covered by any cache, so it recompiles on every nightly run; and the fuzz
job lacks a fail-fast guard if the fuzz target binary never builds) plus several
hardening suggestions.

## Verdict

**REQUEST_CHANGES**

The self-hosted-runner-on-`pull_request` exposure is a blocking security defect for a
public repo and must be gated before this merges. Everything else is non-blocking.

## Stats

| Severity | Count |
|----------|-------|
| Critical | 1 |
| Important | 2 |
| Suggestions | 7 |
| Positive notes | 8 |

## Verification performed

- `bash -n scripts/intel-preflight.sh` → syntax OK
- `./scripts/intel-preflight.sh` → **exit 0**, all gates passed live (vmx, /dev/kvm rw,
  KVM API==12, perf_event_paranoid=1, 2 MiB hugepages, cargo 1.93.0, musl target,
  bzImage present)
- `python3 yaml.safe_load` on both workflows → OK
- `actionlint` → not installed on this host (could not run)
- Confirmed workspace members + package names referenced by the workflows all exist:
  `detguest-wire`, `detguest-host`, `detguest-agent`, `detguest-workloads`,
  `detguest-vmtest`; fuzz target `decode_record`; `fuzz/Cargo.lock` present (validates
  `rust-cache workspaces: guest-sdk/fuzz`); `crates/detguest-wire/tests/loom_ring.rs`
  carries `#![cfg(loom)]`; `pub mod ring` is public (validates `miri test --lib ring`).
