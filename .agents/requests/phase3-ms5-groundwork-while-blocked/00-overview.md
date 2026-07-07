# Request: Stage Ms5 So The Determinism-Replay Gate Lights Up The Day The Hypervisor Hands Off

## Who Is Asking

The phases track. Filed 2026-07-07, alongside a determinism-hypervisor
request whose item 3 is the other half of this one.

## Why guest-sdk, Why Now

Your Phase 3 star is delivered: Ms4 done and independently verified
(2026-07-02), the boot-scheduling deadlock fixed and verified on the real
worker, game-device materialization landed — all five request dirs in this
repo are resolved. What remains of your Phase 3 obligation is **Ms5** — Phase 3 exit gate 2
requires "Ms5 `determinism_replay` CI gate green," and the phase work
list spells out its content: `inject_point` + input-log round trip +
the bit-identical determinism proof.

Ms5's critical items sit behind two external beads
(`guest-sdk-ext-hyp-input-log-dev-events`,
`guest-sdk-ext-hyp-determinism-replay-linux`, both P0, last updated
2026-06-18 — before the hypervisor's M9 acceptance). Here is the news:
**the hypervisor capabilities appear to already exist** (DHILOG
PAD_SET/DEV_EVENT incl. `pio_answer`; replay-engine application; the
Linux M5 record-replay corpus gate in their M9 evidence), and we have
filed a request in their repo
(`../determinism-hypervisor/.agents/requests/phase3-frame-cap-retune-and-run-wallclock-backstop/`,
item 3) to verify coverage against your two bead contracts on the Intel
VM lane and hand the evidence to you.

So the bottleneck is about to move. The question is whether guest-sdk
spends the handoff-wait idle — 27 blocked beads, 1 ready — or staged so
that when the handoff lands, Ms5 is a short execution rather than a cold
start. (And the blocked count itself deserves scrutiny: some of it is
blanket labeling, not live dependency edges — see the re-triage item.)

## The Ask In One Paragraph

Do the one genuinely ready bead (`guest-sdk-4bc`, host Channel
intern-map / pending-inject re-seed accessors — recorded Ms5 prep debt
from the Ms4 resolution); sharpen the receiving end of the handoff by
writing the per-item acceptance checklist for the two ext-hyp bead
contracts (so the hypervisor verifies against your list, not their
memory); re-triage the Ms5 bead chain to split what is *truly* blocked on
hypervisor records from what is host-side work mislabeled as blocked
(`m5-host-log-fault-plan` — replacing your own `LogFaultPlan` skeleton —
looks like the test case); build the `determinism_replay` test scaffold
to the point where the only missing pieces are the external records; and
close the disclosed residual from the no-frame investigation by running
the strengthened no-timer assertion against a real reference-workload
artifact once their regenerated image exists (`REFWORK_READY_INITRAMFS`).

## Files In This Request

| File | Contents |
|---|---|
| `01-current-state.md` | Evidence: what's done, the blocked graph, what the handoff will unblock |
| `02-requested-work.md` | The ask, sequencing, acceptance criteria, out of scope |
| `03-verification-offer.md` | Cross-request choreography and handback shape |
