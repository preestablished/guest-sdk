# Critical & Important Findings

---

## CRITICAL

### C1 — Public PRs execute arbitrary code on the maintainer's self-hosted box

**File:** `.github/workflows/ci.yaml:8-12` (trigger) + `:119-126` (`in_vm` job → `runs-on: [self-hosted, intel, kvm]` + `./scripts/intel-preflight.sh`)

**The chain (all verified on this repo, not theoretical):**

1. Repo is **public**; workflow triggers on `pull_request` (line 11) with no `if:` gate.
2. The `in_vm` job (line 120) and `scripts/intel-preflight.sh` (line 126) run on the **self-hosted** runner — confirmed online: `intel-box`, labels `[self-hosted, Linux, X64, intel, kvm]`.
3. Fork-PR approval policy is `first_time_contributors` (GitHub default, verified via API). This means a **previous** contributor, or anyone with a **compromised contributor account**, gets workflows auto-run with **no approval prompt**. GitHub's own hardening docs explicitly warn: "you should not use self-hosted runners with public repositories" for exactly this reason.
4. The runner user `infra-admin` is in groups `docker` (1001), `kvm` (994), and `sudo` (verified via `id`). Docker group membership is root-equivalent (`docker run -v /:/host ...` → host root). So arbitrary job code = **full compromise of a box that also hosts other repos' runners and personal services.**

A PR author controls `ci.yaml`, the agent build steps, AND `scripts/intel-preflight.sh` in their PR head — any of which can `curl … | sh`, exfiltrate `~/.ssh`, or pivot via docker. The preflight script merely being "verification" is irrelevant; it runs attacker-supplied code on the box.

**Fix (simplest correct option for a solo maintainer): gate the self-hosted job to `push` (main) only.** Fork PRs still get the full hosted lane (`test`, `no_std`, `miri`, `loom`, `musl`, `aarch64`); only the KVM tier is withheld until the change is on `main`.

```yaml
  in_vm:
    # Self-hosted runner on personal infra: never run from untrusted PR code.
    # Push-to-main only — fork PRs get every hosted lane but not the KVM tier.
    if: github.event_name == 'push'
    runs-on: [self-hosted, intel, kvm]
    steps:
      ...
```

**Even stronger (optional):** put `in_vm` behind a GitHub **Environment** with a required reviewer so even a `push` is gated, and/or move it to a separate workflow keyed on a maintainer-applied label. For a solo lab the one-line `if:` is sufficient and is the minimum bar to merge.

**Residual risk after the fix (acceptable, document it):** a malicious commit pushed to `main` via a *compromised maintainer account* would still hit the runner. That is an accepted risk for a personal lab box and is addressed by the least-privilege follow-ups in `02-suggestions.md` (S1–S3), not by this gate.

---

## IMPORTANT

### I1 — No `concurrency:` block → queue pileup on the single self-hosted runner

**File:** `.github/workflows/ci.yaml` (top level, missing) and `.github/workflows/fuzz.yaml`

There is exactly **one** self-hosted runner (verified: `total_count: 1`). Without a `concurrency` group, rapid pushes to `main` (or, pre-fix, a burst of PR events) serialize `in_vm` runs behind each other — and a single VM-tier run can be long. New runs should cancel superseded in-flight ones. Add at top level:

```yaml
concurrency:
  group: ${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: true
```

Note `cancel-in-progress: true` is correct for `pull_request`/feature pushes; if you want main runs to always finish, scope cancellation with `cancel-in-progress: ${{ github.event_name == 'pull_request' }}`. (Combined with the C1 push-only gate, the practical win is preventing back-to-back main pushes from stacking VM runs.)

### I2 — No top-level `permissions:` block; tokens rely on repo default

**File:** `.github/workflows/ci.yaml` (missing) and `.github/workflows/fuzz.yaml` (missing)

I verified the repo default is `default_workflow_permissions: read`, so the *current* blast radius is small (write only on `push` events). But this is implicit and a repo-setting flip silently re-broadens it. Make least-privilege explicit and local to the workflow. Neither workflow needs write — no job pushes commits, comments, or releases:

```yaml
permissions:
  contents: read
```

`fuzz.yaml` uploads an artifact via `actions/upload-artifact@v4`, which does **not** require any `GITHUB_TOKEN` write scope (it uses the Actions runtime token), so `contents: read` is sufficient there too.

### I3 — No `timeout-minutes` on jobs; runaway/hung job pins the runner

**File:** `.github/workflows/ci.yaml` — all jobs, especially `in_vm` (`:119`)

A hung KVM test or a malicious infinite-loop step (pre-fix) holds the single self-hosted runner indefinitely. Add a bound, e.g. `timeout-minutes: 30` on `in_vm` and a sensible default (10–15) on hosted jobs. `fuzz.yaml`'s fuzz step is wall-clock-bounded by `-max_total_time=1800`, but the surrounding job (`cargo install cargo-fuzz`, build) still benefits from `timeout-minutes: 60`.
