# Suggestions (non-blocking)

These are optional refinements. None block the merge.

## S1 — Consider a workflow-level env var for the target triple

The triple `x86_64-unknown-linux-gnu` now appears once in the `run:` step.
If a future second fuzz target or a `fuzz build` smoke step is added, the
triple will be duplicated. A small `env: FUZZ_TARGET: x86_64-unknown-linux-gnu`
(job- or workflow-level) referenced as `--target ${FUZZ_TARGET}` would
centralize it. Low value for a single use site today — note for later, do
not action now.

## S2 — Gate-logic robustness across line continuation (verified OK, documented for confidence)

The shell `run:` block is a multi-statement script. The `--target` addition
sits on a backslash-continued line *inside* the `cargo` command, so:

```sh
start=$(date +%s)
cargo +nightly fuzz run decode_record \
  --target x86_64-unknown-linux-gnu -- -max_total_time=1800
elapsed=$(( $(date +%s) - start ))
if [ "$elapsed" -lt 1700 ]; then ... exit 1; fi
```

The backslash continuation joins lines 2–3 into one logical `cargo`
invocation; `start`/`elapsed`/`if` are unaffected. The gate still: (a) fails
on a libFuzzer crash (nonzero exit propagates because GitHub `run:` uses
`bash -eo pipefail` by default, so the failing `cargo` aborts before
`elapsed` is computed), and (b) fails a suspiciously-early clean exit via the
`-lt 1700` guard. No change needed — flagging only because the prompt asked
to confirm the gate survives the multi-line edit. It does.

## S3 — Alternatives to pinning the gnu target (chosen fix is the right trade-off)

Three alternatives were considered against the chosen `--target` pin:

1. **`rustup target add x86_64-unknown-linux-musl`** — would install the musl
   *target* but NOT the musl sanitizer/std runtime needed for an ASan fuzz
   build; rust-std for musl does not ship the instrumented sanitizer
   runtimes, so this likely still fails or fuzzes a non-instrumented musl
   build. Wrong direction.

2. **Build cargo-fuzz from source** (`cargo install cargo-fuzz` instead of
   `taiki-e/install-action`) — would produce a gnu-host binary whose
   `default_target()` is already gnu, fixing the root cause implicitly. But
   it trades a ~1s prebuilt download for a multi-minute compile every run and
   adds a moving dependency on cargo-fuzz's own build. Slower and less
   deterministic.

3. **Explicit `--target x86_64-unknown-linux-gnu` (chosen)** — one token,
   zero added install time, self-documenting via the comment, and decouples
   the fuzz build target from however the cargo-fuzz binary happens to be
   linked. This is the correct, most robust trade-off and is also resilient
   if taiki-e later switches back to a gnu-linked prebuilt.

The chosen fix is the right call. No action.
