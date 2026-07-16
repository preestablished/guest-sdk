# Critical & Important Findings

---

## CRITICAL

### C1 — Self-hosted runner executes arbitrary fork-PR code on a public repo

**File:** `.github/workflows/ci.yaml:8-11` (trigger) + `:119-120` (job)

```yaml
on:
  push:
    branches: [main]
  pull_request:           # <-- fires for ANY fork PR
...
  in_vm:
    runs-on: [self-hosted, intel, kvm]   # <-- the physical Intel box
```

**Why this is critical.** The repo is public. `pull_request` (not
`pull_request_target`) checks out and runs the PR head commit. Because the `in_vm`
job targets a self-hosted runner, *any* GitHub user can open a PR whose code runs on
your Intel box with the runner user's privileges. The job already calls
`./scripts/intel-preflight.sh` and `cargo test` from the checked-out tree — both of
which a malicious PR fully controls. This is the canonical "self-hosted runner +
public repo + pull_request" supply-chain hazard: it yields arbitrary code execution
on hardware that, by its very nature, has `/dev/kvm` access, perf_event access, and a
registered runner token. The hosted lanes (`test`, `no_std`, `miri`, etc.) share the
same exposure in principle, but ephemeral GitHub-hosted VMs are the designed
blast-radius for that; the self-hosted box is not.

**Fix (recommended — gate the in_vm job to trusted events only).** The simplest
robust option is to run `in_vm` only on `push` to `main` (post-merge), where code has
already been reviewed:

```yaml
  in_vm:
    runs-on: [self-hosted, intel, kvm]
    if: github.event_name == 'push'        # never on fork pull_request
    steps:
      ...
```

**Stronger / alternative options** (pick per how fast you need pre-merge in-VM
signal):

1. **GitHub Environment with required reviewers.** Add `environment: intel-vm` to the
   `in_vm` job and configure that environment with manual approval. PR runs then pause
   for a maintainer to click "approve" before the self-hosted steps execute. This is
   the cleanest way to keep pre-merge coverage without auto-running fork code.
2. **`pull_request_target` + label gate.** Run in-VM only when a maintainer applies a
   trusted label (`if: contains(github.event.pull_request.labels.*.name, 'safe-to-vm')`).
   Note `pull_request_target` runs against the *base* ref by default, so you must
   explicitly check out the PR head — which re-introduces the risk unless combined with
   the label/approval gate above. Use with care.

Also harden the runner itself regardless of which gate you choose: ensure it is
non-ephemeral only if isolated, scope the runner to this repo (not org-wide), and run
the in-VM steps as an unprivileged user inside a throwaway namespace where feasible.

**Recommended minimum to unblock merge:** add `if: github.event_name == 'push'` to
`in_vm`. The wire/host/no_std/miri/loom/musl/aarch64 lanes still give full pre-merge
signal on hosted runners; the in-VM tier moves to post-merge, which matches its cost
profile anyway.

---

## IMPORTANT

### I1 — `cargo install cargo-fuzz` is not cached; it recompiles every nightly run

**File:** `.github/workflows/fuzz.yaml:24-27`

```yaml
      - uses: Swatinem/rust-cache@v2
        with: { workspaces: guest-sdk/fuzz }
      - name: Install cargo-fuzz
        run: cargo install cargo-fuzz --locked
```

`Swatinem/rust-cache` caches the registry and the workspace `target/` dir, but it does
**not** cache `~/.cargo/bin` binaries installed via `cargo install`. So every nightly
run compiles `cargo-fuzz` from source (several minutes) before any fuzzing starts —
pure waste, and it lengthens the job well past the 30-minute fuzz budget.

**Fix (preferred — fetch a prebuilt binary):**

```yaml
      - uses: taiki-e/install-action@v2
        with: { tool: cargo-fuzz }
```

`taiki-e/install-action` downloads a prebuilt `cargo-fuzz` (seconds, no compile).

**Fix (alternative — cache the cargo bin dir):**

```yaml
      - uses: actions/cache@v4
        with:
          path: ~/.cargo/bin/cargo-fuzz
          key: cargo-fuzz-${{ runner.os }}
      - run: command -v cargo-fuzz || cargo install cargo-fuzz --locked
```

### I2 — Fuzz job can silently "pass" the 30-min gate if the target never builds

**File:** `.github/workflows/fuzz.yaml:28-30`

```yaml
      - name: Fuzz decode_record for 30 minutes
        working-directory: guest-sdk
        run: cargo +nightly fuzz run decode_record -- -max_total_time=1800
```

This is mostly correct: libFuzzer exits non-zero on a crash, so a real finding fails
the job and `if: failure()` uploads the reproducer (good). The subtle gap is the
**clean-run gate semantics** the plan requires ("30-minute *clean* decode_record fuzz
gate"). `-max_total_time=1800` measures *fuzzing* wall time, but if a build problem,
empty corpus, or a `-runs`-style early exit causes libFuzzer to terminate in seconds
with exit 0, the job goes green without having fuzzed for 30 minutes — a false-pass on
the acceptance gate. Recommend asserting a minimum elapsed time, e.g.:

```yaml
      - name: Fuzz decode_record for 30 minutes
        working-directory: guest-sdk
        run: |
          start=$(date +%s)
          cargo +nightly fuzz run decode_record -- -max_total_time=1800
          elapsed=$(( $(date +%s) - start ))
          if [ "$elapsed" -lt 1700 ]; then
            echo "::error::fuzz run exited after ${elapsed}s (<1700s) — gate not satisfied"
            exit 1
          fi
```

This keeps a real crash failing fast (the `cargo fuzz run` line still exits non-zero
first) while catching the "exited clean but didn't actually fuzz" case. (Lower
priority than I1, but it directly concerns the correctness of the M0 acceptance gate.)
