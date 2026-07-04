# Root Cause Analysis (2026-07-04, reviewed + corrected)

## Observed On The Real Worker

`dh-m9-ready-handoff`, instrumented (determinism-hypervisor `44c44f5`
dumps buffered guest events on a non-Ready stop). Boot of the rebuilt
package-04 image (guest-sdk `914dbde`, reference-workload `aa69558`):

```text
stop reason 4 (HARD_CAP); icount=10000000000 frames=0
  Hello (640M) · WorkloadStarted (642M) · "boot: helloack" (642M)
  wram/framebuffer/meta: NameIntern + RegionRegister, gen 6 — all at 642–643M
  ...then abrupt total silence to 10B. No Ready (stream 8): 0.
  "boot: gameloaded"/"boot: rw-ready" appear only at icount=10B (force-stop
   artifacts) — the agent never received GameLoaded during the run.
```

Region registration **completes** in ~1 M instructions (all at ≈643 M),
then nothing. Fast progress then an abrupt full stop — a deadlock at the
post-registration `GameLoaded` handshake, not a slow spin. **Note this
proves the workload runs and makes progress up to that point** — see the
mechanism caveat below.

## Why It's The Environment, Not The Image

The device-less probe (`tests/vm/tests/boot_probe.rs` + `BOOT_PROBE_GAME`)
on the **identical** image reaches `Ready { region_count: 3, gen 6 }` and
the workload stays alive to the deadline.

## The Environment Delta (corrected from the first draft)

1. **The probe delivers timer ticks; the worker's guest gets none.** The
   probe builds an in-kernel irqchip + PIT
   (`tests/vm/src/harness/mod.rs:135,144`). The deterministic worker's
   guest has no armed interrupt source. **Correction:** this is a
   *configuration* fact, not "determinism forbids interrupts." The
   hypervisor supports deterministic interrupt injection
   (`dh-vmm/src/inject.rs`: `queue_interrupt` / `KVM_INTERRUPT`, timed by
   icount) and has a paravirt timer device (`dh-devices/src/clock.rs`).
   They are simply never armed here: the guest kernel build has no driver
   for that timer (`image/KERNEL.md` "No paravirt clock, bare TSC") and the
   forced cmdline (`dh-vmm/src/config.rs:92`,
   `notsc tsc=unstable clocksource=jiffies noapictimer`) disables the
   TSC/APIC timer. Result: **`jiffies` never advance** during this boot.

2. **Timerless cmdline on the probe still reaches Ready.** Adding those
   same flags to the probe via the new `BOOT_PROBE_CMDLINE` override did
   **not** reproduce the wedge — disabling the guest's *use* of the
   TSC/APIC timer doesn't stop the probe's PIT from delivering ticks. So
   the trigger is the *absence of interrupt delivery*, not the cmdline.

## Mechanism (suspected, not fully pinned)

`jiffies`-never-advancing has two effects, and it is not established which
dominates:

- **Cooperative-only scheduling.** The agent's boot waits are spins:
  `MSG_DONTWAIT recv → idle() (service_region_ipc) → sched_yield()`
  (`control.rs:216-234`; `wait_for_expected_regions` `runtime.rs:366-405`
  is the same shape). `sched_yield` is cooperative — it can hand off
  without a timer — but with only the agent and workload runnable and no
  tick, the handoff to the workload is unreliable.
- **Frozen tick-driven kernel bookkeeping.** With `jiffies` stuck, RCU
  quiescent-state advance, `schedule_timeout`, delayed workqueues, and
  load-balancing don't fire — any of which the *workload* might depend on
  to complete `register_region #3`'s reply-recv or the `GameLoaded` send.

**Caveat that matters for the fix:** the workload completed all three
region registrations, so cooperative scheduling demonstrably works for
those handoffs. The stall is specifically at region-3-reply / GameLoaded.
So making the *agent* stop spinning may not be sufficient if the *workload*
has its own tick-shaped dependency. `02`'s reproducer is the test.

Why the poll caps didn't fire: `CONTROL_RECV_POLL_LIMIT` bounds *poll
iterations* as a guest-instruction proxy; a genuine block that doesn't
advance the counter never trips it — hence the silent 10 B HARD_CAP.

## Anchors (verified)

| What | Location |
|---|---|
| Boot control-recv sched_yield spin | `crates/detguest-agent/src/control.rs:214-254` |
| idle → service_region_ipc wiring | `crates/detguest-agent/src/runtime.rs:194-197` |
| expected-regions wait (same spin) | `crates/detguest-agent/src/runtime.rs:366-405` |
| region-IPC listener fd / non-blocking service | `crates/detguest-agent/src/region_ipc.rs:129,143,160,216` |
| supervise loop epoll — **region-IPC fds only, NOT the control fd** | `crates/detguest-agent/src/supervise.rs:237-242,285,314,375,517` |
| control fd retained but never epolled | `runtime.rs:203`, `supervise.rs:229` |
| probe irqchip+PIT (the masker) | `tests/vm/src/harness/mod.rs:135,144`; watchdog note `:349` (host-side) |
| hypervisor deterministic interrupt injection / pv-timer | `determinism-hypervisor/crates/dh-vmm/src/inject.rs`, `dh-devices/src/clock.rs` |
| forced timerless cmdline | `determinism-hypervisor/crates/dh-vmm/src/config.rs:92` |
