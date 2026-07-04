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
The delta is what your plan already flagged: **"the probe host drains
ring A continuously while the real worker buffers events until stop —
consumer index frozen mid-run."** So the probe's doorbell-wait always
makes progress; the real worker's does not.

## Why The Caps Didn't Fire (the tell)

Your retuned wedge-to-fault caps (`CONTROL_RECV_POLL_LIMIT` 500 K,
`READY_REGION_POLL_LIMIT`) bound the *poll loops*. But the boot ran the
full 10 B without faulting — so the wedge is **not** in a counted poll
loop. It is in the **unbounded** `channel.rs::emit` critical-full loop
(`:203-212`), reached via:

```
drive_refwork_start (control.rs:149, recv-GameLoaded loop)
  └─ recv() WouldBlock → idle()                  (control.rs:231)
       └─ Supervisor::service_region_ipc          (runtime.rs:196)
            └─ RegionIpc::handle (region_ipc.rs:246)
                 └─ channel.emit_with_doorbell     (region_ipc.rs:296)
                      └─ channel.emit → LOOP on RingFull (critical)  ← spins here
```

Because the spin is inside `idle()`, the outer `recv()` `for _ in
0..CONTROL_RECV_POLL_LIMIT` never advances, so the cap can't fire. That
matches every observation.

## Why The M9 Fixture And The Probe Don't Hit It

- The staged fixture (`m9_refwork_contract.rs`) emits a *minimal* event
  set and never fills ring A before `Ready`.
- The probe drains ring A continuously, so `emit`'s doorbell-wait
  always finds space.
- The real refwork harness emits the full critical burst (Hello,
  WorkloadStarted, 3× {NameIntern, RegionRegister}) into a ring the real
  worker never drains mid-run → overflow → spin.

## Anchors

| What | Location |
|---|---|
| Unbounded critical-full spin | `crates/detguest-agent/src/channel.rs:203-212` |
| `emit_with_doorbell` (region path) | `crates/detguest-agent/src/channel.rs:223`, called `region_ipc.rs:296` |
| `is_critical` (no drop for NameIntern/RegionRegister/Ready) | `crates/detguest-wire/src/record.rs:115` |
| control recv poll cap (didn't fire) | `crates/detguest-agent/src/control.rs:216,247` |
| idle→service wiring | `crates/detguest-agent/src/runtime.rs:194-197` |
| Worker's ring-A consumer behavior (mid-run drain?) | determinism-hypervisor run loop — needs confirming |
