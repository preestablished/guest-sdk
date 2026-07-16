# CI Lanes + Intel Preflight — Review Overview (2nd reviewer)

- **Branch:** `ralph/iteration-6-ci-lanes-preflight`
- **Date:** 2026-06-10
- **Reviewer:** Claude Opus (2nd reviewer) — CI/CD security focus
- **Scope:** 3 new files — `.github/workflows/ci.yaml`, `.github/workflows/fuzz.yaml`, `scripts/intel-preflight.sh` (251 insertions, all additions vs `main`).

## Summary

The CI tiering is functionally well-constructed: all referenced packages (`detguest-vmtest`, `detguest-workloads`), test targets (`loom_ring`, fuzz `decode_record`), and the `ring` module exist and are wired into the workspace correctly; both YAML files parse and the preflight script passes `bash -n`. The blocking problem is **not correctness — it is the self-hosted runner threat model**. This is a **public** repo whose `in_vm` job routes to a self-hosted runner (`intel-box`, confirmed online, labels `[self-hosted, Linux, X64, intel, kvm]`) on the maintainer's personal box, and the workflow triggers on **`pull_request`** with no event/branch/label gate. I verified the repo's fork-PR approval policy is `first_time_contributors` (GitHub default) — which does **not** gate previous contributors or compromised accounts — and that the runner user `infra-admin` is in the `docker`, `kvm`, **and** `sudo` groups. That combination means any attacker who can land a PR that GitHub auto-runs gets arbitrary code execution as a docker-group (root-equivalent) user on a box that also hosts other repos' runners and personal services: full-box compromise. The fix is a one-line gate. Secondary items: no per-workflow `permissions:` block (repo default is `read` — verified — so low severity but worth making explicit), actions pinned by floating tag rather than SHA, and a missing `concurrency:` block (queue pileup on the single self-hosted runner).

## Verdict

**REQUEST_CHANGES** — one Critical (self-hosted runner exposed to public PR execution) must be fixed before this merges. Everything else is Important/Suggestion and can follow.

## Stats

| Severity | Count |
|---|---|
| Critical | 1 |
| Important | 3 |
| Suggestion | 6 |
| Positive notes | 7 |

## Verification performed

- `bash -n scripts/intel-preflight.sh` → OK
- `python3 -c yaml.safe_load(...)` on both workflows → valid
- `gh api .../actions/permissions/workflow` → `default_workflow_permissions: read`, `can_approve_pull_request_reviews: false`
- `gh api .../actions/permissions` → `enabled: true`, `sha_pinning_required: false`
- `gh api .../actions/permissions/fork-pr-contributor-approval` → `approval_policy: first_time_contributors`
- `gh api .../actions/runners` → 1 runner, `intel-box` online, labels include `self-hosted, intel, kvm`
- `id infra-admin` → groups include `sudo`, `kvm (994)`, `docker (1001)`
- Confirmed `crates/detguest-wire/src/ring.rs` exists with `mod tests`; `tests/vm` (`detguest-vmtest`) and `tests/vm/workloads` (`detguest-workloads`) are workspace members; `loom_ring.rs` and `fuzz/fuzz_targets/decode_record.rs` exist.
