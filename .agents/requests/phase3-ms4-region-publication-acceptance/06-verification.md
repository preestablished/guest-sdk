# Verification (rom-operator-bridge side, 2026-07-02)

Reviewed with two independent passes (code-claim verification against
commits `683527f`/`cdb1cf6`/`604cd41`; evidence audit including hash
recomputation) plus an independent re-run of the acceptance on this host.

## Verdict: Confirmed. Ms4 is delivered as claimed.

- **Code:** all three blockers verified in source — real mlock + prefault +
  `AF_UNIX SOCK_SEQPACKET` agent IPC in `register_region` (standalone mode
  errors instead of the old fake handle; handles go DEAD on drop);
  `SO_PEERCRED` + pagemap extent-walk agent as sole manifest writer;
  `ReverifyRegions` genuinely detects drift (rewrite + P0) and unmapping
  (DEAD + P0), with non-tautological tests. The acceptance test's
  synchronization is real KVM-exit-driven `run_until`, not sleeps; the
  `DETGUEST_M4_CHILDREN` knob is recorded in evidence so it cannot silently
  weaken the gate. Kernel pinning (no compaction/migration/KSM/THP/swap)
  confirmed in `image/kernel.config`. The M9 fixture now publishes the full
  229,376-byte D7 framebuffer and holds its handles.
- **Evidence:** artifact roots exist; `root-regions/` sizes exact
  (wram 8,192 / framebuffer 229,376 / meta 256) and all three SHA-256s
  recompute to the recorded `root_baseline` values; 100-entry per-child
  table; VM-tier gating and test counts match the claimed CI invocation;
  all five M4 beads closed and the follow-up bead (`guest-sdk-4bc`) filed.
- **Independent re-run:** `regions_readable_and_stable_across_100_snapshot_restore_branches`
  re-run green on this host at HEAD (`604cd41`), 30.98 s, fresh artifact
  root `target/m4-acceptance-20260702T135319Z/` with `children=100`,
  `child_frames=60`, and all five claimed assertion classes recorded.

## Two Minor Notes (No Action Needed)

1. The committed runs' `evidence.json` records `git_rev = cdb1cf6` (HEAD's
   parent — evidence was generated before the docs-landing commit). Our
   re-run's artifact root above records `git_rev = 604cd41`, so
   HEAD-pinned evidence now exists on disk.
2. "Plan and review trail" is inline review annotations in the plan files
   (00–07), not a discrete review document. The annotations show the review
   pass happened; just noting the wording.

## Phase 3 Status After This

Exit-gate item 2's first half (Ms4 acceptance) is green. Remaining for the
gate: your Ms5 `determinism_replay` CI gate, reference-workload M4→M5
(now unblocked — `refwork-d7t.10`'s GS-5/GS-6 checklist items are
satisfiable against the real path), snapshot-store M7 GC, and the in-VM
first-room run. Our standing offer in `04-verification-offer.md` is
unchanged: when a READY snapshot with the real workload exists, we run the
`RestoreSnapshot → GetFramebuffer → browser preview` half and report here.
