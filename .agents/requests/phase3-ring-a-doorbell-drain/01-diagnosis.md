# Diagnosis (Real-Worker Run, 2026-07-04)

Image under test: reference-workload `aa69558` (lock at guest-sdk
`914dbde`), rebuilt locally; the build's rev-check guarantees the
`914dbde` agent.

## The Real-Worker Event Trail

`dh-m9-ready-handoff`, instrumented (determinism-hypervisor `44c44f5`
dumps buffered ring-A events on a non-Ready stop):

```text
stop reason 4 (HARD_CAP); icount=10000000000 frames=0
  stream=1  icount=640981471  Hello              (critical)
  stream=9  icount=642810314  WorkloadStarted    (critical)
  stream=11 icount=642810314  LogLine "boot: helloack"   (DROPPABLE)
  stream=2/7 642810314..643049118  wram, framebuffer, meta
             NameIntern (critical) + RegionRegister (critical) — SIX pairs
  stream=11 icount=10000000000 LogLine "boot: gameloaded"  (force-stop artifact)
  stream=11 icount=10000000000 LogLine "boot: rw-ready"    (force-stop artifact)
```

- Region registration **completes** (6 critical pairs, `gen 6`).
- The last breadcrumb with a *real* icount is `boot: helloack`. The
  `gameloaded`/`rw-ready` LogLines carry `icount == 10_000_000_000`
  exactly — emitted/flushed only at the force-stop, not mid-run. The
  agent never actually received `GameLoaded` during the run.
- No `Ready` (stream 8, EventKind::Ready): **0**.

## Probe vs Real Worker — Same Image, The Discriminator

The device-less probe (`tests/vm/tests/boot_probe.rs` with
`BOOT_PROBE_GAME`) on the **identical** image reaches
`Ready { region_count: 3, gen 6 }` and the workload is **alive at the
30 s deadline** (Timeout, not WorkloadExited). Symptom 2 fully fixed.
**ROOT CAUSE CONFIRMED (2026-07-04): preemption.** The probe creates an
in-kernel irqchip + PIT (`tests/vm/src/harness/mod.rs:135`; the run loop
notes "idle HLT ... wakes on timer", :348), so it delivers periodic
timer interrupts and the guest gets **preemptive** scheduling. The
deterministic worker delivers **no** interrupts (determinism forbids
them), so scheduling is **cooperative only**. The agent's boot
control-recv wait is a spin — `MSG_DONTWAIT recv → idle()
(service_region_ipc) → sched_yield()` (`control.rs:214-247`) — that
relies on preemption to hand the CPU to the workload. Under the worker's
no-interrupt execution, `sched_yield` doesn't get the workload scheduled,
so after region registration the handshake deadlocks (the workload never
sends `GameLoaded`, the agent spins) → the 10 B silent HARD_CAP.

Two experiments nailed it:
- **Timerless cmdline on the probe still reaches `Ready`.** Adding the
  worker's `notsc tsc=unstable clocksource=jiffies noapictimer` to the
  probe cmdline (new `BOOT_PROBE_CMDLINE`) did NOT reproduce the wedge —
  because disabling the guest's *use* of the TSC/APIC timer doesn't stop
  the probe's PIT from delivering ticks. So it is not the guest cmdline;
  it is the **absence of interrupt delivery** under the deterministic
  worker. This is also exactly why "the probe can't reproduce symptom 1":
  the PIT masks it.
- **Fast-then-dead, not slow.** Region registration completes in ~1 M
  instructions (all at icount ≈ 643 M), then abrupt total silence to
  10 B. A slow cooperative spin would dribble progress; an abrupt full
  stop after fast progress is a deadlock at the post-registration
  `GameLoaded` transition.

## The Fix (plan H1, now with a confirmed why)

Replace the `sched_yield` spin in the agent's boot waits with a
**`poll(2)` blocking wait on BOTH the fd-3 control socket AND the
region-IPC fds** (`control.rs::recv` + the `idle`/`service_region_ipc`
fds; the epoll infra already exists for the supervise loop). `poll(2)`
deschedules the agent until real I/O readiness, which forces the kernel
to run the workload — no dependency on preemption. Keep a bounded
fallback that boot-faults with a named leg if `poll` returns nothing
serviceable N times, so a future wedge is loud, not a 10 B silent cap.

## Fast Local Reproducer (unblocks your verify loop)

Because the masker is the PIT, a **non-preemptive probe variant** — same
`boot_probe` but with interrupt delivery suppressed (no irqchip/PIT tick,
or a run mode that doesn't inject timer IRQs) — should reproduce the
deadlock in ~30 s, WITHOUT a real-worker handoff. Build that and you can
iterate the `poll(2)` fix locally; the bridge session does the final
real-worker confirmation. (If a faithful non-preemptive probe is hard,
hand back the commit + lock bump and the bridge runs the real worker.)

## Anchors

| What | Location |
|---|---|
| Unbounded critical-full spin | `crates/detguest-agent/src/channel.rs:203-212` |
| `emit_with_doorbell` (region path) | `crates/detguest-agent/src/channel.rs:223`, called `region_ipc.rs:296` |
| `is_critical` (no drop for NameIntern/RegionRegister/Ready) | `crates/detguest-wire/src/record.rs:115` |
| control recv poll cap (didn't fire) | `crates/detguest-agent/src/control.rs:216,247` |
| idle→service wiring | `crates/detguest-agent/src/runtime.rs:194-197` |
| Worker's ring-A consumer behavior (mid-run drain?) | determinism-hypervisor run loop — needs confirming |
