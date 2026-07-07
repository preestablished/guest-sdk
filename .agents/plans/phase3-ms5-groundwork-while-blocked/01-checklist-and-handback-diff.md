# Package 01 — Checklist Into The Beads, Diff The Arrived Handback, Flip, Acknowledge

Covers request items 2 and 6, merged because the handback
(`.agents/requests/phase3-ext-hyp-input-log-and-replay-handoff/00-handback.md`,
dh rev `0831f92`) arrived before this plan executed. The request's own
fallback governs: "diff the evidence against the checklist on receipt
instead — same rigor, opposite order."

Rigor note: write the checklist **from the bead contracts and
IMPLEMENTATION-PLAN §Ms5 first, then open the handback matrix**. The
checklist's authority comes from being derived from our contracts, not
from the evidence it will judge. (The author has necessarily read the
handback already — the discipline is that every checklist item must
trace to a contract line, not to a matrix row.)

## Step 1 — Mint the checklist, with stable IDs

Sources of truth for the items:

- Bead `guest-sdk-ext-hyp-input-log-dev-events` DESCRIPTION: "PAD_SET
  landing, channel mutation DEV_EVENT encodings for ring C and I
  pushes, ring A and W consumer bumps, and pio_answer records."
- Bead `guest-sdk-ext-hyp-determinism-replay-linux` DESCRIPTION:
  "bit-identical determinism_replay Linux guest gate and replay-mode
  input-log application with synthesizer absent."
- `docs/prompts/guest-sdk-in-guest-chain-milestones-3-5.md:58-59`
  (the Ms5 acceptance lines: input log captures every deterministic
  host mutation; replay returns the same decisions at the same inject
  sequence points; 1000 seeded iterations on the Intel in-VM lane).
- Both beads' unblock condition: "shipped **and available to the
  Intel VM lane**."

Checklist shape — one line per item, each phrased so the counterparty
can cite a test or evidence file against it:

`ILDE-*` (input-log-dev-events):

- ILDE-1: PAD_SET record kind exists with a pinned encoding test.
- ILDE-2: DEV_EVENT for ring C pushes — ring id distinctly pinned.
- ILDE-3: DEV_EVENT for ring I pushes — ring id distinctly pinned
  (not inferred from C's test).
- ILDE-4: consumer-bump DEV_EVENT for ring A — ring id distinctly
  pinned.
- ILDE-5: consumer-bump DEV_EVENT for ring W — ring id distinctly
  pinned.
- ILDE-6: `pio_answer` record with single-emitter discipline (exactly
  one record per IN, never doubled) and an encoding test.
- ILDE-7: all of the above available to the Intel VM lane (evidence
  produced on, or demonstrably runnable on, the lane host).

`DRL-*` (determinism-replay-linux):

- DRL-1: replay-mode input-log application with the synthesizer absent
  (replay answers injects from the recorded log only), test-cited.
- DRL-2: divergence detection when the log lacks or mismatches an
  expected answer (missing/mismatch both classified), test-cited.
- DRL-3: channel-mutation drift detection covers consumer bumps and
  ring pushes, test-cited.
- DRL-4: bit-identical Linux replay evidenced on a **real** workload
  image (not only the fixture-era corpus), with run count and rev.
- DRL-5: available to the Intel VM lane.

Adjust wording during execution if a contract line demands it, but do
not drop items; add `ILDE-n+1`-style items if the bead contracts imply
something this list missed.

## Step 2 — Write the checklist into the two bead descriptions

Per the request, the bead descriptions are the one location the
hypervisor's text already points at — not a side doc.

```bash
bd update guest-sdk-ext-hyp-input-log-dev-events --description="<original description>

ACCEPTANCE CHECKLIST (cite a test or evidence file per item):
ILDE-1: ... (one line each)"
```

Same for `guest-sdk-ext-hyp-determinism-replay-linux` with `DRL-*`.
Preserve the original description text verbatim above the checklist;
`bd update --description` replaces the whole field, so re-read it with
`bd show` first and re-include it. Do not touch the NOTES field here —
it carries the hypervisor's handback annotations.

## Step 3 — Diff the handback matrix against the checklist

For each item, record: checklist ID → handback evidence cite (their
file:line / test name) → verdict (satisfied / gap). Expected mapping
from the matrix as filed (verify, don't trust this table):

| Item | Handback evidence |
|---|---|
| ILDE-1 | `dhilog.rs:44,:171`; `record_framing_and_padding` (`dhilog.rs:506`) |
| ILDE-2 | `detchannel.rs:789-798`; `push_command_logs_ring_push_with_ring_c_id` |
| ILDE-3 | `push_workload_ctrl_logs_ring_push_with_ring_i_id` (added at `0831f92`) |
| ILDE-4 | A-then-W drain test (`detchannel.rs:1766`, asserts A=2) |
| ILDE-5 | `doorbell_drains_logs_cons_bump_and_sdk_digests` (asserts W=3) |
| ILDE-6 | `dhilog.rs:218-232`; `pio_answer_dev_event_encoding`; single-emitter doc `detchannel.rs:809-813` |
| ILDE-7 | evidence produced on `infra-control`, the lane host |
| DRL-1 | `replay_engine.rs:82`; `log_backed_replay_detchannel_answers_nonzero_inject` |
| DRL-2 | `pio_answer_missing`/`pio_answer_mismatch` (`replay_engine.rs:174,:185`) + reseal classifier test |
| DRL-3 | `EVENT_CONS_BUMP` verification (`replay_engine.rs:611,:913`), drift classifier tests |
| DRL-4 | `VerifyReplay` ×2/run on `workload-image-0.1.0`, green 3 consecutive runs, dh `0831f92` |
| DRL-5 | runs executed on `infra-control` |

Known disclosed caveat to weigh for DRL-4: dh's fixture-era Linux
corpus gate is stale against the real image (their bead
`determinism-hypervisor-jyo7`, P1). The *capability* is freshly
evidenced via the VerifyReplay legs on the real image; the stale corpus
is their regression-suite debt, tracked in their repo. Recommended
verdict: DRL-4 satisfied, caveat recorded verbatim in the flip
annotation so the 1000-iteration gate's eventual evidence doesn't
inherit an unstated assumption.

## Step 4 — Flip or annotate

If every item is satisfied: flip both beads —

```bash
bd close guest-sdk-ext-hyp-input-log-dev-events \
  --reason="Handback 00-handback.md (dh 0831f92) satisfies ILDE-1..7; diff in .agents/plans/phase3-ms5-groundwork-while-blocked/ execution notes; caveat none"
bd close guest-sdk-ext-hyp-determinism-replay-linux \
  --reason="Handback satisfies DRL-1..5; DRL-4 caveat: dh fixture-era corpus stale (determinism-hypervisor-jyo7), capability evidenced on real image via VerifyReplay x2, 3 consecutive runs"
```

Closing these unblocks the live dep edges into
`m5-host-log-fault-plan` and `m5-determinism-replay-ci-gate` (the
latter still *stays* blocked on `m5-vm-inject-roundtrip` — that is
correct and expected; package 03 annotates it).

If any item falls short: leave that bead blocked, add a NOTES line
naming the failed item, and respond in the handback dir as `01-gap.md`
with the specific gap — the handback promises P0 treatment.

## Step 5 — Acknowledge in the hypervisor's request dir

Their acceptance criterion 3 waits on our acknowledgment. Write a short
note into
`../determinism-hypervisor/.agents/requests/phase3-frame-cap-retune-and-run-wallclock-backstop/`
(follow that dir's numbering; likely `0N-guest-sdk-ack.md`): checklist
now live in the two bead descriptions, diff done in the fallback order,
both beads flipped (or the specific gap), one line on the DRL-4 caveat
disposition. Also drop the one-line "checklist is live" pointer the
request's item 2 asks for — the ack note satisfies both if it names
where the checklist lives.

That directory is a sibling repo (`../determinism-hypervisor` relative
to this repo's parent). Verify repo context before committing there
(`pwd`, `git remote -v`), commit only that note, and use their existing
commit-message style.

## Done when

- Both bead descriptions carry the ID'd checklist (verify with
  `bd show`).
- The diff table (item → evidence → verdict) is recorded in this plan
  dir's execution notes or the resolution file.
- Both beads flipped, or gap responses filed.
- Ack note committed in the hypervisor request dir.
