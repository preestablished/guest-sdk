# Choreography And Handback

## The Convergence, Round 2 Shape

- **hypervisor round-1 item 3** delivers the DHILOG/replay handoff;
  this request's prep item A dispositions the third bead
  (`ext-hyp-m9-linux-guest`) **ourselves** against their M9 evidence.
  The note in their request dir is an FYI — "we are dispositioning
  the m9 bead against your evidence; no action needed unless we
  report a gap" — not a scope extension to their in-flight item.
- **reference-workload round-1** supplies the image and runs its own
  M5 stamp; the two suites are the two halves of Phase 3 exit gates
  1–2 and should share the Intel lab calendar where possible.
- **The hugepage reconciliation (prep item B)** is the one item with
  no other home — this request is its tracker of record. Kernel
  provenance is already green (verified by running the preflight);
  only the hugepage question remains, and its fix — if real — routes
  through the hypervisor's host-config regime with the operator, not
  through this repo acting alone.

## Phases-Track Verification

On your resolution we will:

1. re-run the CI-lane invocation from a clean checkout (one iteration
   budget) and confirm the full-gate evidence's internal consistency
   (1000 iterations, fault-plan census, four-surface hashes);
2. re-run the deliberate-mismatch negative;
3. check every Ms3/Ms5-tail bead is closed-or-triaged and the
   exit-gate-2 citation resolves (paths exist, revs match).

## Handback Shape

Entry-condition prep (items 3–4) may be resolved early in a short
`04-prep-notes.md` — the third-bead disposition and the host-preflight
record are valuable the moment they exist. Full resolution follows
(continue numbering) with SHAs, evidence roots, bead table, CI link;
we respond with a verification file.

## Contact / Tracking

- The Ms3/Ms5 tail beads (`01-` lists them with edge types).
- Predecessors: this repo's round-1; hypervisor round-1 item 3;
  refwork round-1.
- Plan authority: IMPLEMENTATION-PLAN §Ms3/§Ms5;
  `phase-3-workload-in-the-box.md` exit gate 2.
