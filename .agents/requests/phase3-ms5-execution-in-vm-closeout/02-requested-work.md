# Requested Work

## Ungated Prep (Do Now — No Gate)

A. **Disposition `ext-hyp-m9-linux-guest`.** Receiving-side diff
   against the hypervisor's M9 acceptance evidence; flip or record the
   precise gap; FYI note in their request dir (no hypervisor action
   expected unless a gap is found).
B. **Settle the hugepage question.** Reconcile the preflight comment
   ("nothing in tests/vm needs host hugepages") with the absence of
   any hugepage usage in the hypervisor repo. Outcome is one of:
   (i) requirement dissolves — update the script comment and the e2e
   bead NOTES, gate closed; or (ii) requirement is real for the
   canonical lane — file the one-line ask into the hypervisor's
   host-config regime (`host-config-intel-box.md` /
   `apply-host-config.sh`), operator-executed; guest-sdk never
   mutates the shared box out-of-band. (Kernel provenance is already
   green — verified by running the preflight — and appears in no gate
   below.)
C. **Timing probe, before any window is booked.** As soon as the gates
   for item 2 permit even a partial run: 10 iterations, extrapolate,
   record the wall-clock budget. The plan gives no runtime estimate;
   at 30–120 s/iteration the 1000-run is 8–33 hours — plan the
   window from measurement, not hope.

## Entry Conditions (Per-Item Gates — Staged, Not All-Or-Nothing)

- **Item 1 (Ms5 host/SDK chain)**: round-1 groundwork complete
  (`phase3-ms5-groundwork-while-blocked/` resolved) + the
  `ext-hyp-input-log-dev-events` flip received.
- **Item 2 (the flagship gate)**: item 1 + the
  `ext-hyp-determinism-replay-linux` flip + a real workload image
  (reference-workload round-1's regenerated artifact preferred; the
  newest real image otherwise — record which; if their round-1 stalls,
  that is the failure route to flag).
- **Item 3 (Ms3 acceptance)**: gates {`ext-hyp-input-log-dev-events`
  flip, prep item A, prep item B settled, image} only — it does NOT
  wait for the determinism-replay handoff or round-1's scaffold, and
  may execute in an earlier window than item 2.
- **Item 4 (CI-lane bead)**: both items 2 and 3.

If a gate fails, the work item is its owner: round-1 (this repo) or
the hypervisor's round-1 item 3 (theirs) or refwork round-1 (theirs).

## What We Need (Behavioral)

1. **The in-VM inject round trip.** `m5-vm-inject-roundtrip` + its
   side-chain (`m5-channel-reattach-checkpoint`,
   `m5-host-mutation-log-audit` — blanket-labeled, unblock on triage),
   plus DHILOG-backed completion of `m5-host-log-fault-plan`.
2. **The flagship gate — with a wall-clock contract.** Fill the
   scaffold's stubs; run `determinism_replay` to the plan bar: 1000
   consecutive iterations, varied fault plans **and seeded, logged
   input bursts**, bit-identical across the plan's four surfaces
   (final RAM hash; drained event stream byte-for-byte; drop
   counters; inject decisions via LogLine digest). Execution must be
   **seeded, chunked, and resumable** (a window interruption resumes
   at iteration k). Wire into the `in_vm` CI lane behind
   `DETGUEST_VM_TESTS`, with an explicit split: the recurring CI
   iteration budget (state it) vs the one-time 1000-iteration
   acceptance (lab lane).
3. **Ms3 acceptance.** `m3-vm-real-workload-e2e` (real workload boots
   under the Linux guest on the Intel lane, SDK events end-to-end)
   and `m3-input-path-acceptance` (pv-pad input through PAD_SET
   records) — in whatever window their subset gates permit; don't
   hold them hostage to item 2's readiness. Never schedule across
   refwork's cutover window (worker restart).
4. **The CI-lane bead.** `m3m5-ci-intel-vm-lanes` closed: the lane
   runs the m3 e2e + m5 replay gates on the documented cadence.
5. **The whole tail, enumerated from bd — not prose.** Also close or
   explicitly disposition: `m3m5-final-quality-gates` (P0),
   `m3m5-handoff-closeout` (P1), `m5-reference-workload-contract-tests`
   (P1), `m3-docs-as-built`, `m5-docs-replay`, and the three M4
   stragglers (`m4-capture-contract-tests` — note Phase 4 consumes
   these captures, so out-scoping it needs a written reason —
   `m4-reverify-churn-test`, `m4-sdk-stats-region-autoreg`). Update
   the `m5-reference-workload-*` beads (they are beads, not files)
   and post the exit-gate-2 citation.

## Acceptance Criteria

1. Prep A/B/C resolved early (the `04-prep-notes.md` channel) — A's
   disposition, B's outcome (i or ii, with the routing evidence), C's
   measured budget.
2. `determinism_replay`: 1000/1000 green on the Intel lane; evidence
   includes per-iteration hash summary, fault-plan **and input-burst**
   census, seeds, resume points if any, revs; the deliberate-mismatch
   negative re-run once against the real path.
3. Ms3: e2e and input-path acceptances green with artifacts (boot log,
   event trail, PAD_SET round-trip record).
4. CI lane demonstrably runs the gates (one linked green run) with the
   recurring budget documented.
5. Tail check, mechanically verifiable: `bd list` filtered to the
   m3/m5/ci/m4-straggler beads named in `01-` shows every one closed
   or carrying a written disposition; exit-gate-2 citation posted.

## Out Of Scope For This Request

- Round-1's staging scope — predecessor.
- The hypervisor handoff verification itself (their round-1 item 3) —
  except the receiving-side diff and the prep-A/B items, which are
  ours.
- reference-workload's M5 stamp — consumed, not owned.
- Ms6 (Phase 8). The Phase-4 capture corpus (refwork round-2).
