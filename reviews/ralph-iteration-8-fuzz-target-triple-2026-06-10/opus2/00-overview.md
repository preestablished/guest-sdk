# Review Overview — fuzz target triple pin (2nd reviewer)

- **Reviewer:** Claude Opus (2nd reviewer)
- **Branch:** ralph/iteration-8-fuzz-target-triple
- **Date:** 2026-06-10
- **Scope:** `git diff main...HEAD` — single change: `.github/workflows/fuzz.yaml`
- **Verdict:** REQUEST_CHANGES

## The change

The diff adds an explicit `--target x86_64-unknown-linux-gnu` to the
`cargo +nightly fuzz run decode_record` invocation, plus a three-line
comment explaining why. Rationale per the prompt: the prebuilt
`taiki-e/install-action` cargo-fuzz binary is musl-linked, so its baked
`DEFAULT_TARGET` is `x86_64-unknown-linux-musl`; the first dispatch run
(27254629026) failed with E0463 (`can't find crate for std`) because the
musl sanitizer/build-std build has no usable std.

## Assessment summary

The target-triple diagnosis is **correct**, and pinning gnu is the right
fix for the E0463 the run hit. I verified empirically that cargo-fuzz's
`--target` default is its own *compile-time* host triple (the help text
shows gnu here only because the locally-installed cargo-fuzz is gnu-linked,
built from source) — a musl-built release binary would default to musl.
So the fix addresses a real bug.

**However**, the fix is necessary but almost certainly **not sufficient**.
`cargo fuzz run` defaults `-Zbuild-std=true`, which rebuilds `std` from
source and therefore hard-requires the `rust-src` rustup component.
`dtolnay/rust-toolchain@nightly` installs with `profile = minimal`, which
does **not** include `rust-src`. So the very next dispatch run will fail
again — this time with `"…/library/Cargo.lock" does not exist, unable to
build with the standard library` — unless `components: rust-src` is added
to the toolchain step. I reproduced this exact failure locally.

This is the central finding the first reviewer is most likely to have
missed, because it only surfaces *after* the target fix lets the build
get past target resolution and into the build-std phase.

## What was verified

- cargo-fuzz `run` defaults: sanitizer=address, `-Zbuild-std` defaults true. (confirmed via `cargo fuzz run --help`)
- `-Zbuild-std` fails without `rust-src`. (reproduced locally: removed component, got the Cargo.lock-missing error)
- Host triple on ubuntu-latest is gnu and preinstalled — no `target add` needed for gnu. (confirmed)
- `llvm-tools` is **not** required for `fuzz run` (only `fuzz coverage`), so its absence is not a blocker.
- Crash artifacts are produced at runtime under `fuzz/artifacts/decode_record/` and are gitignored; the upload path is correct.
- The elapsed-time gate under `bash -e` behaves as intended: a crash exits the step before the gate (job fails), which is the desired behavior.

See `01-critical-and-important.md` for details.
