# Positive Notes

Good patterns worth keeping, with exact references.

1. **The fuzz job stays on hosted runners.** `fuzz.yaml:17` uses `runs-on: ubuntu-latest`, and the schedule (`:8-10`) plus `workflow_dispatch` introduce **no self-hosted exposure**. This is exactly the right call — scheduled jobs on a public repo are a classic vector for runner abuse, and keeping them hosted neutralizes it.

2. **Repo default token permission is already `read`.** Verified `default_workflow_permissions: read` and `can_approve_pull_request_reviews: false`. The maintainer has already tightened the account-level default, which limits blast radius even before the per-workflow `permissions:` block (I2) lands.

3. **Job/bead coverage is complete and accurate.** Every lane the plan calls for is present and maps to a real, buildable target: `test`, `no_std` (`-p detguest-wire --no-default-features`), `miri` (`--lib ring`), `loom` (`--test loom_ring` with `RUSTFLAGS: --cfg loom`), `musl` (static cross-build + `--check` smoke + `file | grep static`), `aarch64` (hosted `ubuntu-24.04-arm`), `in_vm`, and the separate `fuzz` workflow. I confirmed `detguest-vmtest`, `detguest-workloads`, the `ring` module, `loom_ring.rs`, and the `decode_record` fuzz target all exist and are correctly in/excluded from the workspace.

4. **Miri filter is correct.** `cargo miri test -p detguest-wire --lib ring` (`ci.yaml:63`) — the ring tests live in `mod tests` inside `crates/detguest-wire/src/ring.rs:308`, so their test paths are `ring::tests::*`; the substring filter `ring` matches them. No `MIRIFLAGS` is required for the `AtomicU32::from_ptr` usage (it's a safe-on-valid-pointer API; UB would only arise from an invalid/misaligned pointer, which the tests don't construct). Consistent with "we run the same locally fine."

5. **musl lane verifies what it claims.** Not just a build: it runs the produced binary (`--check`, `:96`) AND asserts true static linking via `file … | grep -E 'static(ally|-pie) linked'` (`:97-99`). The regex correctly handles both `statically linked` and `static-pie linked` `file(1)` outputs — a detail that often regresses silently.

6. **VM tests are correctly double-gated.** `tests/vm` is a normal workspace member (so it gets fmt/clippy/build on every hosted lane) while the KVM-requiring tests are `#[ignore]` + env-gated (`DETGUEST_VM_TESTS=1 … -- --ignored --test-threads=1`, `:127-129`). This keeps coverage broad without breaking the hosted `cargo test --workspace`.

7. **Preflight fails loud and specific.** `scripts/intel-preflight.sh` accumulates a `FAIL` flag and exits non-zero with an actionable message per gate (e.g. line 23 tells you to add the user to the `kvm` group; line 67 gives the exact `rustup target add` command). The `set -uo pipefail` choice (no `-e`) is deliberately correct for its `if check; then fail` structure. The bzImage check is appropriately non-fatal with an explanatory `note` (`:76-79`).
