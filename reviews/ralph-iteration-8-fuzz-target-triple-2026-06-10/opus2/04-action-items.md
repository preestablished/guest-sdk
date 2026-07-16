# Action Items

### Critical

- [ ] **Add `rust-src` to the nightly toolchain step.** `cargo fuzz run`
  defaults `-Zbuild-std=true`, which requires the `rust-src` component;
  `dtolnay/rust-toolchain@nightly` uses `profile=minimal` and does not
  install it. Without this, the next dispatch run fails with
  `".../library/Cargo.lock" does not exist, unable to build with the
  standard library` (reproduced locally). Apply:
  ```yaml
  - uses: dtolnay/rust-toolchain@nightly
    with:
      components: rust-src
  ```
  This is the blocker — the target-triple fix alone trades E0463 for a
  build-std failure. (See C1.)

### Important

- [ ] **Strengthen the rationale comment** to record that cargo-fuzz's
  `--target` default tracks the *installed binary's* build-time host triple
  (musl for the prebuilt release), not the runtime rustc host. Prevents a
  future "this flag looks redundant locally" regression. (See I1.)

### Suggestions

- [ ] Consider pinning nightly to a date for stable Swatinem cache keys and
  predictable cold-build cost on a `-Zbuild-std` job. (See S1.)
- [ ] Consider deriving the gate threshold from `max_total_time`
  (e.g. 90%) or commenting that `1700 = 1800 − 100s slack`. (See S2.)
- [ ] Optionally pin `cargo-fuzz` version in `taiki-e/install-action`
  (`tool: cargo-fuzz@0.13.2`) for reproducibility against future default
  changes. (See S3.)

## Verdict

**REQUEST_CHANGES** — the target-triple pin is correct and necessary, but
it exposes a deterministic follow-on failure (missing `rust-src` for the
default build-std build) that will red the next run. Add `components:
rust-src` and this is ready.
