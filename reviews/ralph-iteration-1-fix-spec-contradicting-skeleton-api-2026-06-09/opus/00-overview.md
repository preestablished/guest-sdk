# Review Overview

- **Branch:** `ralph/iteration-1-fix-spec-contradicting-skeleton-api`
- **Base:** `main`
- **Date:** 2026-06-09
- **Reviewer:** Claude Opus

## Summary

This branch introduces the first real code for `guest-sdk`: a Cargo workspace with
the `#![no_std]` `detguest-wire` crate (channel header, SPSC ring framing, event /
command / workload-control payloads, region manifest with seqlock helpers, and the
detcall PIO ABI constants) plus a thin `detguest-agent` skeleton that re-encodes a
spec-correct `Ready` record. The implementation is unusually faithful to the
normative specs: I cross-checked every offset, size, kind number, field width, and
flag bit in `API.md` §3–§5 and `ARCHITECTURE.md` §2/§7 against the code and found the
byte layouts correct throughout (record framing, all 15 event kinds, both region
events' `u32` vs `u64` generation widths, the manifest header/entry/extent strides,
and the PIO port map / `FaultDecision` packing). The decoders are written in the
total, bounds-checked style the research notes call for — forged `len` / `name_len` /
`details_len` / `msg_len` fields are rejected, not read past — and the only `unsafe`
module (`ring.rs`) carries a sound disjoint-region split-borrow argument with correct
acquire/release discipline and wrap-safe `u32` index math. The deliberate
`RING_W_SIZE` deviation (1 MiB power-of-two instead of the spec's non-power-of-two
`0x1E0000`) is well-reasoned and correctly documented; I verified both the index
discipline and the attach-validation path genuinely require a power of two, so the
deviation is justified.

The one blocking issue is a soundness *smell* the compiler agrees with: `clippy`
fails the build with a deny-level `mut_from_ref` error on `Producer::slice_mut`, which
hands out a `&mut [u8]` from a `&self`. No UB exists today (the two slices it produces
are provably disjoint), but the signature defeats the borrow checker's aliasing
guarantee and breaks any CI that runs clippy. There are also a few minor non-blocking
items (a redundant header re-validation, a defensive `free()` underflow guard, a
`manual_range_contains` lint).

## Verdict

**REQUEST_CHANGES** — one Important fix (the `mut_from_ref` clippy-deny on
`slice_mut`, which both breaks clippy-gated CI and removes a borrow-checker safety
net in the only unsafe module). Everything else is suggestion-level. No critical
soundness holes, spec byte-mismatches, or decoder panics were found.

## Stats

- **Files changed:** 12 (all additions; no deletions/modifications of prior files)
- **Lines added / removed:** +3132 / -0
- **Commits:** 1 (`60af133` — "ralph: iteration 1 checkpoint - spec-correct detguest-wire crate + agent skeleton")
- **Tests:** `cargo test --workspace` → 35 passed, 0 failed.
- **no_std build:** `cargo build -p detguest-wire --no-default-features` → clean.
- **clippy:** 1 deny-level error (`mut_from_ref`), 1 warning (`manual_range_contains`).

## Issue counts

- Critical: 0
- Important: 1
- Suggestions: 5
