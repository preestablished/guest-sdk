# Resolution: Routing Decision And Fix (Historical Record)

Written 2026-07-07 (phase3-ms5-groundwork-while-blocked, ledger item).
The fix itself landed 2026-07-04 folded into the
`phase3-boot-scheduling-deadlock` work; this file closes the handback
this dir's `02-fix-and-verify.md` promised and was never written.

## The routing decision

**Agent-side (guest-sdk-owned), via the boot-scheduling-deadlock Fix A —
not a determinism-hypervisor change.** `02-fix-and-verify.md` step 1 had
already established the worker *does* drain ring A on doorbell exits
(`detchannel.rs:590`), so the "make the worker drain" routing was a
no-op. What the deadlock reproducer then demonstrated
(`../phase3-boot-scheduling-deadlock/03-resolution.md` §1) reframed the
diagnosis: the observed silent HARD_CAP was not a live ring-A emit spin
at all — no poll cap ever fired, meaning the agent was **parked and
never rescheduled** (cooperative-scheduling starvation in the agent's
pre-Ready waits), the class Fix A eliminates by parking those waits in
the supervisor epoll (`Supervisor::wait_boot_io`, with unconditional
region-IPC service on every wake).

## The commits that carried the fix

Rode the boot-scheduling-deadlock resolution (see that dir's
`03-resolution.md` §"Commits" for the full table):

- `70851a2` — Fix A: epoll-blocking boot waits in the agent
  (control-reply recv + expected-regions gate park in the supervisor
  epoll; region IPC serviced on every wake).
- `487ff56` — handback commit the bridge pinned in
  `reference-workload/image/guest-sdk.lock`.
- `d3ac547` — the no-timer reproducer tiers (incl.
  `refwork_ready_hold.rs`'s no-timer twin) that gate the fix.

## The real-worker verification that covered it

`1f9a123` ("Verify Fix A on the real worker: Phase 3 step 2 clears")
and `../phase3-boot-scheduling-deadlock/04-verification.md`
(rom-operator-bridge, 2026-07-05): the real-worker
`dh-m9-ready-handoff` run booted the exact image that wedged to
**READY**, snapshotted, and restore-verified — the symptom this request
was filed for (silent 10 B hard-cap before Ready) did not recur.

## Residual noted, deliberately not grown here

`02-fix-and-verify.md` step 2's structural hardening (bounding the
`emit` critical-full doorbell loop, `detguest-agent/src/channel.rs`
`emit` — still an unbounded `loop` today) was **not** part of the landed
fix; the reproducer showed the wedge class was starvation, not the emit
spin, and the real-worker verification is green without it. If that
hardening is still wanted it needs its own bead — this file is the
historical record only, per the groundwork request's scope note.
