# Suggestions (non-blocking)

## S1: Swatinem cache effectiveness with `-Zbuild-std`

`Swatinem/rust-cache@v2` keys on the toolchain hash, lockfile, and Cargo
manifests. With `-Zbuild-std`, std itself is recompiled into the target dir;
this *does* cache within a pinned nightly, but `dtolnay/rust-toolchain@nightly`
floats to whatever nightly is current that day, so every time nightly rolls
the cache key changes and the full std + sanitizer rebuild happens cold.
For a daily-scheduled job that is mostly fine (the 60-min timeout absorbs a
cold build), but if cold-build time ever approaches the budget, consider
pinning nightly to a date (`dtolnay/rust-toolchain@nightly` →
`@master` with `toolchain: nightly-2026-06-01`) for cache stability. Not
required now; flagging because the first dispatch is the moment to notice
cold-build cost.

## S2: Gate lower bound is a fixed 1700s magic number

`max_total_time=1800` paired with `elapsed < 1700` hard-codes a 100s slack.
On a cold build-std run, the *build* time is outside the fuzz loop (the
timer starts before `cargo fuzz run`, which builds then fuzzes), so a slow
cold build actually inflates `elapsed` and the gate stays satisfied — good.
But if libFuzzer ever exits clean a bit early on a fast runner, 100s of
slack is thin. Consider deriving the threshold from the configured time
(e.g. `9/10 * 1800`) or at least a comment that 1700 = 1800 − 100s slack.
Minor.

## S3: Consider `--release`-equivalent flags explicitly

cargo-fuzz builds with sanitizer + opt by default; no action needed. Noting
only that the invocation relies entirely on cargo-fuzz defaults
(sanitizer=address, build-std=true). If a future cargo-fuzz release changes
a default, this workflow inherits it silently. Pinning cargo-fuzz version in
`taiki-e/install-action` (`tool: cargo-fuzz@0.13.2`) would make the
toolchain reproducible. Optional.

## S4: `if-no-files-found: ignore` on the crash upload

Reasonable choice — a clean run produces no artifacts and `ignore` avoids a
spurious warning. No change needed; calling it out as correct intent so it
isn't "fixed" later into `warn`/`error`, which would noise up every healthy
nightly run.
