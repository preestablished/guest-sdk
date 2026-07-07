# Cross-Request Choreography And Handback

## The Three-Repo Convergence

Three requests filed 2026-07-07 interlock here:

- **determinism-hypervisor**
  (`phase3-frame-cap-retune-and-run-wallclock-backstop`, item 3): verifies
  their DHILOG/replay surfaces against *your* two bead contracts and files
  evidence to you. Your item 2 checklist is what they verify against —
  publish it early and tell them where it lives.
- **reference-workload** (`phase3-m4-first-room-gate-and-m5-stamp`):
  regenerates the image/READY snapshot. Their artifact is your item 5
  input, and their M5 suite plus your Ms5 gate are the two halves of
  Phase 3 exit gates 1–2.
- **This one**: the receiving-side staging.

Deadlock check: nothing here waits on more than one of those — items 1–4
need neither request to land; item 5 needs only the refwork artifact.

## Phases-Track Verification

On your resolution we will:

1. re-run the `determinism_replay` scaffold's self-test legs from a clean
   checkout, including the deliberate-mismatch negative;
2. read the re-triaged bead graph against the checklist (every blocked
   bead cites a checklist item; every unblocked one has landed work);
3. confirm the checklist/evidence loop closed in one of its two valid
   orders — they verified against your checklist, or you diffed their
   earlier-arriving evidence against it on receipt (item 2's fallback) —
   with the acknowledgment note (item 6) present in their request dir.

## Handback Shape

Same convention as this repo's five resolved requests: append
`04-resolution.md` (or `0N-` continuing your numbering) with git SHAs,
bead dispositions, the checklist location, scaffold test output, and —
when it happens — the real-artifact `no_timer_post_ready` record; we
respond with a verification file.

## Contact / Tracking

- Beads covered: `guest-sdk-4bc`; the Ms5 chain
  (`m5-host-log-fault-plan`, `m5-sdk-inject-point`,
  `m5-vm-inject-roundtrip`, `m5-determinism-replay-ci-gate`,
  `m3m5-ci-intel-vm-lanes`) for re-triage; the two `ext-hyp-*` beads for
  the checklist.
- The Ms4 resolution's "Notes for anyone touching this next" section is
  the provenance for item 1 and for the register-path deadlock constraint
  anyone in this area should re-read first.
