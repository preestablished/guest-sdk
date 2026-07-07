# Execution Notes

## Package 01 — Checklist diff against the handback (2026-07-07)

Checklist minted from the two bead contracts +
`docs/prompts/guest-sdk-in-guest-chain-milestones-3-5.md:58-59`, written
into both bead DESCRIPTIONs, then diffed against
`.agents/requests/phase3-ext-hyp-input-log-and-replay-handoff/00-handback.md`
(dh rev `0831f92`). Order note: evidence arrived before the checklist
landed, so the request's sanctioned fallback governs — checklist first
from contracts, diff on receipt.

Spot-check: every cited symbol/test below was independently verified to
exist in `../determinism-hypervisor` (contains `0831f92`; HEAD `fac52a6`):
`KIND_PAD_SET` (dhilog.rs:44,:171), `record_framing_and_padding`
(dhilog.rs:506), `pio_answer_dev_event_encoding` (dhilog.rs:537),
`push_command_logs_ring_push_with_ring_c_id` (detchannel.rs:1577),
`push_workload_ctrl_logs_ring_push_with_ring_i_id` (detchannel.rs:1613),
`doorbell_drains_logs_cons_bump_and_sdk_digests` (detchannel.rs:1342),
A-then-W cons-bump assertion (detchannel.rs:1766, asserts `[2, 3]`),
`log_backed_replay_detchannel_answers_nonzero_inject`
(replay_engine.rs:2560),
`replay_detchannel_reports_missing_inject_answer_as_divergence` (:2594),
`reseal_classifier_labels_pio_answer_mismatch` (:2633),
`reseal_classifier_labels_channel_mutation_drift` (:2644).

| Item | Handback evidence | Verdict |
|---|---|---|
| ILDE-1 | `dhilog.rs:44,:171`; test `record_framing_and_padding` (`dhilog.rs:506`) | satisfied |
| ILDE-2 | `detchannel.rs:789-798`; test `push_command_logs_ring_push_with_ring_c_id` (asserts C=0) | satisfied |
| ILDE-3 | test `push_workload_ctrl_logs_ring_push_with_ring_i_id` (asserts I=1; added at `0831f92`) | satisfied |
| ILDE-4 | A-then-W drain test (`detchannel.rs:1766`, asserts A=2) | satisfied |
| ILDE-5 | test `doorbell_drains_logs_cons_bump_and_sdk_digests` (asserts W=3) | satisfied |
| ILDE-6 | `dhilog.rs:218-232`; test `pio_answer_dev_event_encoding` (`dhilog.rs:537`); single-emitter doc `detchannel.rs:809-813` | satisfied |
| ILDE-7 | all evidence produced 2026-07-07 on `infra-control`, the Intel lane host | satisfied |
| DRL-1 | `replay_engine.rs:82`; test `log_backed_replay_detchannel_answers_nonzero_inject` (:2560) | satisfied |
| DRL-2 | `pio_answer_missing`/`pio_answer_mismatch` (`replay_engine.rs:174,:185`); `replay_detchannel_reports_missing_inject_answer_as_divergence` (:2594); `reseal_classifier_labels_pio_answer_mismatch` (:2633) | satisfied |
| DRL-3 | `EVENT_CONS_BUMP` verification (`replay_engine.rs:611,:913`); `reseal_classifier_labels_channel_mutation_drift` (:2644); ring-push drift (:2655/:2666) | satisfied |
| DRL-4 | `VerifyReplay` ×2 per run on real dist `workload-image-0.1.0` (initramfs blake3 `36f50484…`), green 3 consecutive runs, dh `0831f92` | satisfied, with caveat |
| DRL-5 | runs executed on `infra-control` | satisfied |

DRL-4 caveat (recorded, not glossed): dh's fixture-era Linux corpus gate
is stale against the real image (their bead `determinism-hypervisor-jyo7`,
P1 — old `PVBLKIO1` meta-proof, old fixture initramfs pin, `epoch_len`
overshoot). The bit-identical replay *capability* is freshly evidenced on
the real image via the VerifyReplay legs; the stale corpus is dh's
regression-suite debt, tracked in their repo. Verdict: DRL-4 satisfied
with the caveat carried into the flip annotation so the 1000-iteration
gate's eventual evidence doesn't inherit an unstated assumption.

Result: every item satisfied → both beads flipped (closed). No gap
response needed.
