# Fix Options And Verification

## The Routing Decision (yours — you own the channel contract)

The bug is a mismatch between `channel.rs::emit`'s documented contract
("the doorbell exit makes the host drain + bump the consumer index") and
the real worker's `NextSdkEvent` run behavior (ring A not drained until
stop). One of the two must move. Please decide which and record it in
`03-resolution.md`:

### Option A — Worker drains ring A mid-run (likely primary; determinism-hypervisor)

If the doorbell-drain contract is real (the comment says it is, and the
whole point of `DOORBELL_RING_A` is host wakeup), then the worker's
`Run{until: NextSdkEvent(...)}` loop must **service `DOORBELL_RING_A` VM
exits by draining ring A and advancing the consumer index**, not just
watch for the target event kind. Your plan's H4 already routes this:
"that half lives in determinism-hypervisor — file a request to that repo
per the series convention rather than fixing cross-repo yourself."

If you choose A: hand the precise worker-side ask back here (or say the
word) and **the bridge session will file the determinism-hypervisor
request and drive it** — we own that repo's request series and the
real-worker verification. This keeps you from fixing cross-repo.

### Option B — Agent does not depend on mid-run drain (guest-sdk)

If the contract is that the host only drains at stop/capture boundaries
during a `NextSdkEvent` run, then the agent must guarantee it never
needs more critical ring-A space than exists before `Ready`:

- Ensure the pre-Ready critical burst (Hello, WorkloadStarted, 3×
  {NameIntern, RegionRegister}, Ready) fits ring A with margin — i.e.
  ring A must hold the whole boot handshake, or the burst must be
  reduced/coalesced.
- Consider whether `NameIntern`/`RegionRegister` must be *critical*
  during boot, or whether the host reconstructs them from the manifest
  at Ready (making them droppable would remove the spin entirely).

### Both options: kill the silent HARD_CAP

Regardless of A or B, replace the **unbounded** `emit` critical-full
`loop` (`channel.rs:203-212`) with a **bounded** doorbell-wait that
boot-faults with a named leg + spin count when the host doesn't drain in
N iterations. Your wedge-to-fault hardening missed this because the spin
isn't in a *poll* loop — it's in `emit`. A never-draining host must
produce a loud fault, not a 10 B silent cap. Negative-test it (a stub
channel whose consumer never advances → the named boot-fault).

## Confirm The Mechanism Cheaply

Before/with the fix, add a counter to the `emit` critical-full branch
(doorbell-ring count) surfaced in the boot-fault detail. One
bridge-run real-worker handoff will then show the spin count pinned to
ring A and name the exact event that overflowed — turning this
code-confirmed diagnosis into an observed one, consistent with the
series' evidence discipline.

## Verification Loop — Read This

**The probe cannot reproduce symptom 1.** It drains ring A continuously,
so it reaches Ready on the exact image that wedges under the real
worker. Your `refwork_ready_hold` VM test will pass on a broken fix.
**Every candidate fix must be verified by a real-worker
`dh-m9-ready-handoff` run, which is the bridge session's to run.** Hand
back a lock bump + commit (or, for Option A, the determinism-hypervisor
change) and we turn the real-worker run around fast. Green =
`dh-m9-ready-handoff` reaches `Ready` and snapshots, yielding the step-2
exit evidence (READY icount ≈ 643 M, region_count 3 / gen 6, state
hash) — which unblocks the READY-snapshot regeneration (step 3), the
`BRIDGE_REAL_SNAPSHOT_REF` cutover (step 4), and the first real frame in
the browser.

## Handback

`03-resolution.md` here with your routing decision and the fix. We
re-run the real-worker handoff and answer with `04-verification.md`.
