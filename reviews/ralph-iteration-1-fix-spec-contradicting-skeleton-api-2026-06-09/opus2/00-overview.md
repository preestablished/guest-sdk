# Code Review — Overview

- **Branch:** `ralph/iteration-1-fix-spec-contradicting-skeleton-api`
- **Date:** 2026-06-09
- **Reviewer:** Claude Opus (2nd reviewer)
- **Base:** `main` … `HEAD` (`60af133`)

## Summary

This branch introduces the first real code for the deterministic-execution guest
SDK: a Cargo workspace containing `detguest-wire` (a `#![no_std]` byte-level wire
codec — channel header, SPSC ring framing/discipline, event/command/workload-ctrl
payloads, the region manifest with seqlock helpers, and the detcall PIO ABI
constants) plus a thin `detguest-agent` skeleton that re-exports a spec-correct
`Ready` record helper. I cross-checked every constant, field offset, payload size,
and semantic rule (criticality classes, Pad seq semantics, flag bits, detcall
ports/packing) against the normative `API.md` (§3–§5) and `ARCHITECTURE.md`
(§2 layout, §3 ring discipline, §7 determinism). The implementation is
**byte-for-byte faithful to the specs** in every place I checked: header offsets,
the 16-byte record header, all 15 event payloads, the 6 commands, the 96-byte
region entry, the 16-byte extent, the manifest seqlock, and the FaultDecision
24-bit packing (golden values match). The known deliberate ring-W deviation
(1 MiB power-of-two vs the doc's non-power-of-two 0x1E0000) is correctly reasoned
and well documented. The SPSC ring's acquire/release discipline, free-running
u32 wrap math, and never-wrap pad framing are all correct, and the full-ring /
empty-ring capacity math has no off-by-one. The unsafe surface is small,
encapsulated, and carries real safety contracts.

The findings below are about **test coverage and defense-in-depth**, not wire
correctness. The single most important gap is that `API.md` §3.5 *normatively*
requires byte-exact golden fixtures ("Golden tests pin every byte of every v1
payload") and the crate ships only round-trip tests — which by construction
cannot catch a wrong-but-symmetric layout. Everything else is a suggestion.

## Verdict

**APPROVE** (with one Important follow-up: add the spec-mandated golden byte
fixtures before this layout is consumed by the host crate).

## Stats

- **Files changed:** 12 (all additions)
- **Lines:** +3132 / −0 (per `--stat`; raw diff +3144 / −12 counting churn within added files)
- **Commits:** 1 (`60af133` — "ralph: iteration 1 checkpoint - spec-correct detguest-wire crate + agent skeleton")
- **Tests:** `cargo test --workspace` → 35 passed, 0 failed
- **no_std build:** `cargo build -p detguest-wire --no-default-features` → clean
