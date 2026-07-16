# Suggestions (non-blocking)

## Runner least-privilege follow-ups (defense for the post-C1-fix residual risk)

These reduce blast radius if `main` is ever pushed maliciously (compromised maintainer account) or if C1 is only partially mitigated. Severity-rated for a **personal lab box**, not a fleet.

### S1 — Dedicated unprivileged runner user (severity: Medium for a lab)

The runner currently executes as `infra-admin`, who is in `sudo`, `docker`, and `kvm`. Run the Actions runner as a **dedicated service user** that is in `kvm` only (needed for the VM tier) and **not** in `docker` or `sudo`. Docker-group membership is root-equivalent; removing it is the single highest-value hardening step after C1. The KVM tests only need `/dev/kvm` (kvm group) per the preflight's own check (`scripts/intel-preflight.sh:20-24`), not docker.

### S2 — Drop the runner user from the `docker` group (severity: Medium)

Even if you keep `infra-admin`, the preflight and the visible job steps never invoke docker — the VM tier uses KVM directly. There's no reason this runner needs docker group membership. If something does need containers, prefer rootless docker/podman.

### S3 — Ephemeral runners (severity: Low for a solo lab)

Use `--ephemeral` registration so each job gets a fresh runner process and the workspace/job state can't persist between runs (no cross-job credential/cache poisoning). Low priority for a single-maintainer box but cheap if you're already scripting registration.

## Supply chain

### S4 — Pin actions by SHA, not floating tag (severity: Low–Medium)

`actions/checkout@v4`, `dtolnay/rust-toolchain@stable`/`@nightly`, `Swatinem/rust-cache@v2`, `actions/upload-artifact@v4` are all **mutable** references. `dtolnay/rust-toolchain` notably has **no immutable version tags at all** — `@stable`/`@nightly` are branches that move, so you're trusting whatever HEAD is at run time. Both `dtolnay` and `Swatinem` are high-reputation maintainers and `actions/*` are first-party, so practical risk is **low for this repo today**; but the recurring lesson (tj-actions/changed-files, 2025) is that even popular actions get compromised. Pin to full commit SHAs with a `# vX.Y.Z` comment, and let Dependabot bump them. Note the repo setting `sha_pinning_required: false` (verified) — you could flip that on to enforce it.

### S5 — `cargo install cargo-fuzz --locked` is unpinned to a version (severity: Low)

`fuzz.yaml:27` installs the latest `cargo-fuzz`. `--locked` respects cargo-fuzz's own lockfile (good) but not which *version* you get, so a bad release silently lands in the nightly job. Pin `cargo install cargo-fuzz --version X.Y.Z --locked`, or cache the binary. Low severity since this is a hosted-runner-only, schedule-triggered job (no self-hosted exposure).

## Functional polish

### S6 — Minor robustness in `scripts/intel-preflight.sh`

Non-blocking nits:
- **Line 28-33 (KVM API probe):** the python heredoc opens `/dev/kvm` with `O_RDWR` and never closes the fd. Harmless (process exits) but `os.close(fd)` would be tidy. Also `fcntl.ioctl(fd, 0xAE00)` is hardcoded — a one-line comment mapping `0xAE00` → `KVM_GET_API_VERSION` already exists (line 25), good; consider also noting it's `_IO(KVMIO, 0x00)`.
- **Line 44-45 (`perf_event_paranoid`):** if the sysctl reads a non-numeric value, `[[ "$para" -le 1 ]]` errors under `set -u`-adjacent arithmetic; the `-n "$para"` guard covers the empty case but not a stray non-integer. Low risk on a real kernel. Consider `[[ "$para" =~ ^-?[0-9]+$ ]] && (( para <= 1 ))`.
- **Line 6 (`set -uo pipefail`, no `-e`):** intentional and correct here — the script accumulates `FAIL` and exits explicitly at line 81-84. Good choice; flagging only so reviewer 1 doesn't "fix" it by adding `-e`, which would break the `if grep …; then fail` pattern.
