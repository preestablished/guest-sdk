# Action Items

### Critical
- [ ] [.github/workflows/ci.yaml:119-126] Gate the `in_vm` self-hosted job so untrusted PR code never runs on the personal box. Add `if: github.event_name == 'push'` to the `in_vm` job (fork PRs keep every hosted lane; only the KVM tier is withheld until merged to main). Optionally also place it behind a GitHub Environment with a required reviewer. Verified: public repo + `pull_request` trigger + online self-hosted runner `intel-box` + fork approval policy `first_time_contributors` + runner user in `docker`/`sudo` = full-box compromise from a single PR.

### Important
- [ ] [.github/workflows/ci.yaml: top level] Add a `concurrency:` block (`group: ${{ github.workflow }}-${{ github.ref }}`, `cancel-in-progress: true`) to prevent queue pileup on the single self-hosted runner (`total_count: 1` verified).
- [ ] [.github/workflows/ci.yaml + fuzz.yaml: top level] Add explicit `permissions: { contents: read }`. Default is `read` (verified) but make it local and tamper-evident; no job needs write, including the artifact upload in fuzz.yaml.
- [ ] [.github/workflows/ci.yaml: all jobs, esp. :119] Add `timeout-minutes` (e.g. 30 on `in_vm`, 10–15 on hosted jobs; 60 on the fuzz job) so a hung/malicious step can't pin the single runner.

### Suggestions
- [ ] [runner host config] Run the Actions runner as a dedicated unprivileged user in `kvm` only — not `docker`, not `sudo` (S1). Highest-value hardening after the Critical fix.
- [ ] [runner host config] Remove the runner user from the `docker` group; nothing in CI uses docker (S2).
- [ ] [runner host config] Register the runner with `--ephemeral` for per-job isolation (S3; low priority for a solo lab).
- [ ] [all 3 workflows] Pin actions by full commit SHA instead of floating tags (`@v4`/`@stable`/`@nightly`/`@v2`); note `dtolnay/rust-toolchain` has no immutable tags. Enable Dependabot; optionally flip repo `sha_pinning_required` on (S4).
- [ ] [.github/workflows/fuzz.yaml:27] Pin `cargo install cargo-fuzz --version X.Y.Z --locked` (S5).
- [ ] [scripts/intel-preflight.sh:28-33,44-45] Minor robustness: close the KVM fd; integer-validate `perf_event_paranoid` before arithmetic. Keep `set -uo pipefail` as-is (S6).
