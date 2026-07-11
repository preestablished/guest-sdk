# Plan: Execute Ms5 + Ms3 In-VM Acceptance And Close The Phase-3 Tail

Answers `.agents/requests/phase3-ms5-execution-in-vm-closeout/`. Read all
five request files before implementing this plan. Baseline used to write the
plan: clean `main` at `5313afe` (2026-07-10).

## Outcome

The Intel lane proves the remaining Ms3 and Ms5 behavior on the current real
workload image, the Ms5 lab acceptance records 1000/1000 consecutive seeded
iterations, the recurring push-only lane runs a deliberately smaller stated
budget, and every named Phase-3 tail bead is closed or has a precise written
disposition. The request resolution contains enough immutable evidence for a
clean-checkout rerun and an exit-gate-2 citation.

## Current facts that govern execution

- Round-1 groundwork landed at `ef36f42`; the replay scaffold at `c13ee1a`
  still has exactly three `MS5-STUB` markers in
  `tests/vm/tests/determinism_replay.rs`.
- The input-log and replay external beads are closed. The M9 and refwork
  capability beads remain stale in guest-sdk even though their cited upstream
  work landed. Re-diff, then close or rewrite them; do not flip by assumption.
- Preserve the hypervisor real-image corpus caveat (`determinism-hypervisor-jyo7`
  / `i74w`) separately from the already-green replay capability.
- Default guest-sdk `tests/vm` uses anonymous host memory and guest-internal
  hugepages. The current script's optional host-hugepage assertion and its
  claim about the hypervisor harness need evidence-based reconciliation; do
  not mutate the Intel host to make an optional probe green.
- The existing `in_vm` job has a 30-minute timeout and sweeps all ignored VM
  tests. A possibly multi-hour 1000-run cannot silently become that recurring
  job. The measured 10-run probe decides the lab-window and workflow shape.

## Packages and ordering

| File | Package | Gate |
|---|---|---|
| `01-ledger-preflight-and-evidence-contract.md` | Reconcile upstream handoffs, current preflight, image identity, and evidence schema | none; do first |
| `02-ms5-roundtrip-and-reattach.md` | Live workload inject calls, host answer/log round trip, replay-decision ingestion, restored-branch sequence proof | package 01 |
| `03-ms3-real-workload-and-pad-set.md` | Real-workload lifecycle and PAD_SET input-path acceptance | package 01; independent of package 02 |
| `04-determinism-replay-gate.md` | Replace all scaffold stubs, add resumable evidence, negative, 10-run probe, then 1000-run lab acceptance | packages 01–02 |
| `05-ci-docs-and-recurring-budget.md` | Wire explicit Ms3/Ms5 recurring gates into the push-only Intel lane | packages 03–04; timing result required |
| `06-tail-closeout-and-handback.md` | Quality gates, full bead disposition, request resolution, exit-gate citation, commit/push | packages 01–05 |

Packages 02 and 03 may be implemented in either order or separate sessions;
package 04 does not wait for package 03. Put the shared decoded-input adapter
needed by Ms5 in package 02, while package 03 proves the full Ms3 contract.
Do not hold Ms3 behind Ms5 once package 01 clears its subset gates. Never run
the lab acceptance across a reference-workload worker cutover window.

## Bead discipline

At the start of every package, re-read the relevant beads with `bd show` and
claim only ready implementation work with `bd update <id> --claim`. A stale
BLOCKED external tracker is first verified, annotated, and transitioned using
the supported `bd` lifecycle (or closed directly with an evidence-bearing
reason); never delete dependencies merely to manufacture readiness. Preserve NOTES
when using `bd update --notes` because it replaces the field. Close a bead only
when its own acceptance is evidenced; a closed upstream dependency is not
proof that this repo's validation passed.

At every session end follow `AGENTS.md`: commit the intended files, run
`bd dolt push`, pull/rebase if needed, push Git, and verify the branch is up to
date. Do not leave the long acceptance as an unpushed local-only result.
