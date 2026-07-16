# Review: detguest-host crate (Milestone 1)

- **Branch:** `ralph/iteration-3-create-detguest-host-crate`
- **Compared against:** `main`
- **Date:** 2026-06-10
- **Reviewer:** Claude Opus
- **Commit:** `8457567` (ralph: iteration 3 checkpoint - detguest-host crate (Milestone 1))

## Summary

This branch adds the new `detguest-host` crate implementing Milestone 1 of the host
side of the detchannel: the `GuestMem` trait + segmented `MockGuestMem`, the
`ChannelWriteSink` input-log hook, `Channel::attach` validation, `drain_events` over
rings A/W, `push_command`/`push_workload_ctrl` host producers, seqlock-consistent
`read_manifest`/`read_region`, `InjectResponder` + `FaultPlan`
(`TableFaultPlan`/`LogFaultPlan`), `NameIntern` interning, and the 10^5-event loopback
acceptance test. The implementation is faithful to API.md §2–§5 and ARCHITECTURE.md §2–§3.
The platform's most load-bearing invariant — *no host mutation of channel memory without
a `ChannelWriteSink` report* — holds: every production-path `gm.write` of channel memory
is reported through the sink exactly once with byte-faithful spans and indices
(verified by enumeration below). The crate is `#![forbid(unsafe_code)]`; the only
`unsafe` is in the loopback test's raw-pointer `GuestMem`, with a sound disjoint-region
aliasing argument. Drain tolerances (mid-write stop, Pad skip, unknown-kind skip-by-len,
CorruptIndices), push arithmetic (matches `wire::ring::bytes_needed` exactly, including
tail-pad seq consumption), seqlock retry discipline, and extent-walk bounds are all
correct. No Critical or Important findings. A handful of low-risk suggestions (test
strength, a String allocation per inject answer, an unused `OwnedPayload::RegionUpdate`
in the test mapping, miri coverage) are non-blocking.

## Verdict

**APPROVE**

## Stats

- Files added: 7 source (`lib.rs`, `guestmem.rs`, `channel.rs`, `drain.rs`,
  `commands.rs`, `manifest.rs`, `inject.rs`) + 1 integration test (`loopback.rs`) +
  `Cargo.toml`; 1 wire change (`header.rs`: `RingId` gains `PartialOrd, Ord`).
- Lines changed: +2533 / −1 (per `git diff main...HEAD --stat`).
- Tests: `cargo test --workspace` — all green. `detguest-host`: 17 unit + 1 integration
  (`loopback_100k_mixed_events`) pass.
- Lints: `cargo clippy --workspace --all-targets` — zero warnings.
- Findings: 0 Critical, 0 Important, 5 Suggestions.
