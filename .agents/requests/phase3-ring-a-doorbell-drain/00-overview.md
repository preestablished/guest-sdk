# Request: Ring-A Critical-Emit Spins When The Real Worker Doesn't Drain Mid-Run

Filed 2026-07-04 by the rom-operator-bridge session. This continues the
`phase3-ready-not-emitted-real-worker` thread (request + plan + my
`04-verification.md` in
`~/git/preestablished/reference-workload/.agents/requests/phase3-ready-not-emitted-real-worker/`).
Your symptom-2 fix (`678dc81`) is confirmed good; this is the
**code-confirmed root cause of symptom 1** (Ready never emitted under
the real worker), which your plan's H1/H4 branches anticipated.

## The Root Cause (confirmed in code, not a hypothesis)

`crates/detguest-agent/src/channel.rs:198-218` — `emit()` for a
**critical** event spins an **unbounded loop** on `RingFull`, ringing
the doorbell each time:

```rust
Err(RingFull) if critical => {
    // Deterministic guest-initiated wait: the doorbell exit
    // makes the host drain + bump the consumer index.
    (self.doorbell)(ports::DOORBELL_RING_A);
}
```

That comment states the contract the agent relies on: **a
`DOORBELL_RING_A` exit makes the host drain ring A and advance the
consumer index.** Under the real worker's
`Run{until: NextSdkEvent(Ready)}`, that does not happen — the worker
**buffers ring A until the run stops** (your plan's own H4 note:
"consumer index frozen mid-run"; my instrumentation only drained the
events *after* the run ended). So once the region-registration burst
fills ring A, the next critical `emit` spins the doorbell forever, to
the 10 B hard cap.

`is_critical` (`detguest-wire/src/record.rs:115`) = everything except
`Pad`/`Beacon`/`LogLine`. So every `NameIntern` + `RegionRegister` in
the 3-region burst is critical and **must land** (they can't drop) —
which is exactly why they pile into ring A and why only the droppable
`LogLine` breadcrumbs survived to the force-stop.

This spin lives inside `service_region_ipc` → `handle()` →
`emit_with_doorbell` (`region_ipc.rs:296`), called from the agent's
`idle()` during the control-recv wait — which is why `recv()`'s 500 K
poll cap (`control.rs:216`) **never fired** and the boot was a silent
HARD_CAP rather than a fast boot-fault.

## The Ask

Decide the fix owner and land it. You own the channel/doorbell contract,
so the routing call is yours:

- If the worker is **supposed** to drain ring A on doorbell exits during
  a `NextSdkEvent` run and doesn't, the primary fix is
  **determinism-hypervisor** (your plan's H4 already says "file a
  request to that repo"). The bridge session will drive that request and
  the real-worker verification.
- If the contract is that the host only drains at stop/capture
  boundaries (not mid `NextSdkEvent`), then the **agent** must not
  require more critical ring-A events than ring A holds before `Ready`
  — reduce the pre-Ready critical burst and/or bound the spin into a
  loud boot-fault.

Either way, the unbounded critical-emit spin should become a **bounded,
named boot-fault** so this can never again be a silent HARD_CAP (your
wedge-to-fault hardening did not cover this path — see `01`).

## Files

| File | Contents |
|---|---|
| `01-diagnosis.md` | Full evidence chain, code anchors, why the caps didn't fire |
| `02-fix-and-verify.md` | Fix options, the routing decision, and the verification-loop constraint |
