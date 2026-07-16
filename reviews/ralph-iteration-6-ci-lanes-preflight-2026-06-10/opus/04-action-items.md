# Action Items

### Critical
- [ ] [.github/workflows/ci.yaml:119-120 + :8-11] Gate the `in_vm` self-hosted job so fork PRs can't run code on the Intel box. Minimum: add `if: github.event_name == 'push'` to `in_vm` (move in-VM tier to post-merge). Better: `environment: intel-vm` with required reviewers, or a maintainer label gate. THIS IS BLOCKING.

### Important
- [ ] [.github/workflows/fuzz.yaml:24-27] `cargo install cargo-fuzz` is uncached and recompiles every nightly run. Switch to `taiki-e/install-action@v2 with: { tool: cargo-fuzz }`, or cache `~/.cargo/bin/cargo-fuzz` via `actions/cache@v4`.
- [ ] [.github/workflows/fuzz.yaml:28-30] Add a minimum-elapsed-time assertion around the fuzz run so a fast clean exit (build/corpus problem) can't false-pass the 30-min M0 acceptance gate. Wrap with a `start`/`elapsed` check that fails if `< 1700s`.

### Suggestions
- [ ] [.github/workflows/ci.yaml top-level + fuzz.yaml top-level] Add explicit `permissions: { contents: read }` to both workflows (least privilege; public-repo checkouts + artifact upload need nothing more).
- [ ] [.github/workflows/ci.yaml:8] Add a `concurrency:` block with `cancel-in-progress: true` keyed on the ref to cancel superseded PR runs (also shrinks the self-hosted abuse window).
- [ ] [.github/workflows/ci.yaml — each Swatinem/rust-cache step] Give each lane a distinct `shared-key` (especially the `--cfg loom` lane) to avoid cross-job cache eviction.
- [ ] [.github/workflows/ci.yaml:119 + fuzz.yaml:16] Add `timeout-minutes` (e.g. 45) to the `in_vm` and fuzz jobs so a wedged run can't hold the runner for the 360-min default.
- [ ] [scripts/intel-preflight.sh:28-33] Optional: `os.close(fd)` in the KVM probe and/or accept `>= 12` for forward-compat (currently a strict `== 12`, which is acceptable for a pinned env).
- [ ] [scripts/intel-preflight.sh:44-49] Optional: comment that `-1` is the most-permissive accepted `perf_event_paranoid` value so it isn't "fixed" to `>= 0` later.
- [ ] [Harness iteration — not this PR] The `in_vm` job assumes a prebuilt `image/build/bzImage`; preflight only *notes* its absence (script:70-79). When the KVM harness lands, the `in_vm` job should build/cache the kernel+initramfs via `image/build.sh` before `cargo test -p detguest-vmtest`, otherwise the first cold run fails or is very slow. File as a follow-up bead.
