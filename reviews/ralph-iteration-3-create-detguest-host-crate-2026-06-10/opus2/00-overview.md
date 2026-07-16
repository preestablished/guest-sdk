# Review — detguest-host crate (Milestone 1)

- **Branch:** `ralph/iteration-3-create-detguest-host-crate`
- **Base:** `main`
- **Date:** 2026-06-10
- **Reviewer:** Claude Opus (2nd reviewer)
- **Scope:** `git diff main...HEAD` — new `crates/detguest-host/` crate (lib, channel, commands, drain, guestmem, inject, manifest, loopback test) + a `RingId` `PartialOrd`/`Ord` derive in `detguest-wire`.

## Summary

This is a careful, spec-anchored implementation of the host side of the detchannel. The
SPSC ring math is shared verbatim with the SDK producer (`bytes_needed`/`free`/
`contiguous_tail`/`encode_pad`), so push framing and the drain's wrap/pad handling agree
by construction; `#![forbid(unsafe_code)]` is honored everywhere except the loopback test's
explicit `RawChannelMem`, whose aliasing is sound under the single-threaded phase-alternation
the test enforces. The `ChannelWriteSink` invariant is interpreted correctly against
ARCHITECTURE.md §2 (the producer-index publish is folded into the single `ring_push` record,
*not* double-logged; cons bumps and PIO answers are separate records). Tests are green
(`cargo test --workspace`: all pass incl. the 10^5-event loopback) and `cargo clippy
--workspace --all-targets` is clean. The one issue that rises to blocking is a **restore-
correctness gap**: the host-produced ring C/I sequence counters (`next_seq_c`/`next_seq_i`)
are host-only state that the crate's own doc claims is "reconstructible from the event
stream," but the host never drains C/I, `attach` resets them to 0, and no derive/getter/setter
exists — so re-attaching after a snapshot restore silently re-emits seq 0 into a ring that
already holds a seq-0 record. Secondary issues: an unchecked `u64` add (`x.gpa + to_skip`)
and an unchecked extent-length `sum` in `read_region` can panic in debug / wrap in release on
guest-corrupt manifest data, contradicting the crate's "arbitrary bytes never panic" posture;
a `drop_counters` signature that deviates from the normative API.md §2 declaration; and a few
folding/test-coverage gaps (duplicate-intern-different-name is silent and uncounted; the
REACHABLE_DECL OR-on-second-occurrence and the doorbell-retry path are not exercised by any test).

## Verdict

**REQUEST_CHANGES** — primarily for the restore/re-attach seq gap (Critical) and the
`read_region` overflow panic (Important). Everything else is small. None of the findings
block the *happy-path* M1 acceptance, which the implementation meets cleanly.

## Stats

- Files added: 10 (`crates/detguest-host/{Cargo.toml, src/*.rs (7), tests/loopback.rs}`)
- Files changed: `crates/detguest-wire/src/header.rs` (+1 derive), `Cargo.toml`, `Cargo.lock`
- Net: ~2533 insertions, 1 deletion
- Tests: full workspace green; loopback (10^5 events) passes in ~0.17s
- Clippy: clean (`--all-targets`)
- Findings: 1 Critical, 3 Important, 6 Suggestions
