# Review Overview — Pin fuzz job target triple

- **Reviewer:** Claude Opus
- **Branch:** ralph/iteration-8-fuzz-target-triple
- **Date:** 2026-06-10
- **Verdict:** APPROVE

## Scope

A single-file CI fix. `git diff main...HEAD` touches only
`.github/workflows/fuzz.yaml`: the `cargo +nightly fuzz run decode_record`
invocation gains an explicit `--target x86_64-unknown-linux-gnu`, split
across two lines with a backslash continuation, plus a three-line comment
explaining why.

```
-          cargo +nightly fuzz run decode_record -- -max_total_time=1800
+          cargo +nightly fuzz run decode_record \
+            --target x86_64-unknown-linux-gnu -- -max_total_time=1800
```

## What the change fixes

The first `workflow_dispatch` of the fuzz job failed in seconds with
`E0463 can't find crate for std` (run 27254629026). Root cause: the
`taiki-e/install-action` prebuilt `cargo-fuzz` binary is musl-linked, and
cargo-fuzz derives its default `--target` from the *host triple it was
compiled for*. A musl-built binary therefore defaults the fuzz build to
`x86_64-unknown-linux-musl`, for which the sanitizer (ASan) std runtime is
not installed on the GitHub runner — hence the immediate std-missing error.
Pinning the target to the gnu triple makes the fuzz build use the toolchain's
default, fully-installed std + sanitizer support.

## Verification performed

1. **Diff exactness** — confirmed the diff is *only* the `--target` addition
   plus comment; no other files or hunks. Single commit `77b68d0`.
2. **YAML validity** — `python3 -c "import yaml; yaml.safe_load(...)"` parses
   cleanly; top-level keys `name`, `on` (parsed as bool `true`, normal),
   `permissions`, `env`, `jobs` all present. The multi-line `run:` block
   parses as a literal scalar.
3. **Reasoning soundness** — confirmed against cargo-fuzz 0.13.2 source
   (`src/utils.rs`): `default_target()` returns
   `current_platform::CURRENT_PLATFORM`, a compile-time constant baked from
   the *build host* triple. This proves a musl-built cargo-fuzz defaults to
   musl. `--target <TRIPLE>` help text confirms it overrides this default.
4. **Gate logic** — the `start`/`elapsed`/`-lt 1700` early-exit guard still
   works across the line continuation (verified shell semantics; see 02).
5. **Local build** — `cargo +nightly fuzz build --target
   x86_64-unknown-linux-gnu decode_record` from `fuzz/` exits 0 and produces
   an 18 MB instrumented binary. (Local host is gnu, so this confirms the
   target is valid + buildable but cannot reproduce the musl-default failure.)

## Bottom line

Minimal, correct, well-commented infra fix that resolves a real,
log-evidenced CI failure. The reasoning is verified against upstream source.
Approve.
