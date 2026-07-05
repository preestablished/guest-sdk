# Verification (rom-operator-bridge side, 2026-07-05)

## Verdict: Fix A confirmed on the real worker. Phase 3 step 2 CLEARS.

Bumped `reference-workload/image/guest-sdk.lock` to `487ff56`
(reference-workload `667ca8b`, pushed), rebuilt the package-04 image, and
ran the real-worker `dh-m9-ready-handoff` — the final gate the probe
could not stand in for.

**Result: the workload-in-the-box booted to READY and snapshotted.**

```text
READY TakeSnapshot succeeded: yes
RestoreSnapshot verification succeeded: yes
source/restored leases destroyed: yes
worker slots before/after: 4/4
```

This is the first time the real emulator+game image has booted to READY
under the deterministic worker — through kernel → agent-as-PID-1 → pv-blk
game materialize → LoadGame → region registration → **Ready** → snapshot.
Before your fix this exact path silently hard-capped at 10 B; after it, it
reaches READY and the snapshot restore-verifies. The READY snapshot ref is
in the private handoff env for the step-3/4 cutover.

Deployed worker was healthy after the run (the handoff destroyed its
leases); one stale PAUSED_S slot at the *old fixture* icount (641343512,
pre-fix) was reclaimed by a routine worker restart — not from this run.

## On Your Five Action Items

1. **Real-worker re-run — done, green** (above).
2. **Shifted READY icount — acknowledged.** We took the new READY point
   from this run; the deployed READY snapshot gets regenerated in step 3
   rather than matched to any old value.
3. **Worker wall-clock budget — filing it.** You're right that with the
   agent parked in `epoll_wait` and no tick, a genuinely dead workload
   HLTs and burns no instructions, so the icount HARD_CAP can't backstop
   it — a Run could hang. Our handoff run was wrapped in a host
   `timeout(1)` as a stopgap and completed well within it. We're filing a
   determinism-hypervisor request for a proper per-Run wall-clock deadline
   on `NextSdkEvent` runs. Not a step-2 blocker (the happy path reaches
   READY), but real robustness debt.
4. **New pre-Ready event shapes — noted, no worker-side Ready-first
   assumption found.** The bridge's frame/session handling keys off
   `Ready`/state, not event ordering; the handoff's `NextSdkEvent(Ready)`
   run stops on the Ready-kind event and tolerates preceding LogLine /
   WorkloadExited events. We'll keep an eye out during step 3.
5. **Frames advance with no tick — great to know**; it retires the last
   "is Fix A sufficient" doubt from our side too. Fix B not needed.

## What's Next (bridge/operator)

Step 2 is closed. Remaining Phase 3: step 3 (regenerate the *deployed*
READY snapshot through the deployed snapstore + `BRIDGE_REAL_SNAPSHOT_REF`
cutover — operator-coordinated), then the browser renders the first real
frame. Thanks — clean fix, thorough reproducer, honest handback.
