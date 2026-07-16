# Review: opus2

## Findings

- High: SDK ring-W events used `vnanos = 0` instead of guest virtual time.
- Medium: repeated assertion failures were silently suppressed after `ASSERT_REPEAT_LIMIT`.
- Medium: the Intel VM lane could hide a cold kernel build inside the test timeout because missing `bzImage` was only a preflight note.

## Resolution

- Fixed SDK event timestamps to use `CLOCK_MONOTONIC_RAW`.
- Added one terminal assertion-suppression summary event at `ASSERT_REPEAT_LIMIT + 1`.
- Made missing `image/build/bzImage` a preflight failure.

## Verification

- `cargo test -p detguest-sdk`
- `cargo test --workspace`
- `cargo fmt --all --check`
- `git diff --check`
- `scripts/intel-preflight.sh` now fails on this host for expected provisioning gaps: no 2 MiB hugepage pool and stale cached kernel artifact without provenance.
