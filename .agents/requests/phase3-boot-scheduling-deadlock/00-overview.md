# Request: Boot Handshake Deadlocks Under The No-Timer Worker

Filed 2026-07-04 by the rom-operator-bridge session (Phase 3 step 2,
first real boot of the workload image). **Supersedes the messier
`phase3-ring-a-doorbell-drain/` thread** (which records two wrong turns —
ring-drain, guest-timer — before this was understood).

Your symptom-2 fix (`678dc81`, fd-3 retention) is confirmed good. This is
the last-standing step-2 blocker. The diagnosis below was itself reviewed
and **corrected twice** — read the caveats; do not treat any single
mechanism as certain until the reproducer in `02` proves it.

## What's Observed (solid)

The boot reaches kernel → agent-as-PID-1 → pv-blk game materialize →
LoadGame → **all three regions register** (`manifest_generation 6`, at
icount ≈643 M), then **abrupt total silence to the 10 B hard cap** with no
guest-sdk `Ready`. The device-less probe reaches `Ready` on the identical
image. The difference is the run environment, not the image.

## The Environment Delta (corrected)

The probe creates an in-kernel irqchip + PIT (`tests/vm/src/harness/mod.rs:135,144`)
that delivers timer ticks. The deterministic worker's guest has **no armed
interrupt source** — not because "determinism forbids interrupts" (the
hypervisor has a full deterministic interrupt-injection path,
`determinism-hypervisor/crates/dh-vmm/src/inject.rs`, and a paravirt timer
device, `dh-devices/src/clock.rs`) but because **this guest kernel build
has no driver for that timer and the forced cmdline disables TSC/APIC**
(`image/KERNEL.md`, `dh-vmm/src/config.rs:92`
`notsc … clocksource=jiffies noapictimer`). So under the worker, `jiffies`
never advance: scheduling is cooperative-only **and** tick-driven kernel
bookkeeping (RCU, `schedule_timeout`, delayed workqueues) is frozen.

## The Suspected Cause (not fully pinned)

The agent's boot waits are `sched_yield` spins
(`control.rs:214`, `runtime.rs:366`) that make progress only if the kernel
hands the CPU to the workload. In a no-tick, cooperative-only guest that
handoff is unreliable, so the boot deadlocks at the region-3-reply /
`GameLoaded` handshake. **Caveat:** the workload *did* complete region
registration, so cooperative scheduling partly works — the residual stall
may involve a tick-dependent kernel path in the workload, not only the
agent's yield. The fix in `02` must be **proven** by the non-preemptive
probe, not assumed.

## The Ask

Fix the no-timer boot deadlock. Two approaches in `02`:
- **(A) Agent-side:** convert the boot waits from `sched_yield` spins to
  `epoll`-blocking waits over the control fd + region-IPC fds. This is the
  smaller change but requires **new control-fd epoll plumbing** (the
  post-Ready supervise loop epolls the region-IPC fds but **not** the
  control fd today) and a bound redesign (see `02`).
- **(B) Deterministic tick:** give the guest an armed, deterministic timer
  via the hypervisor's existing paravirt-timer path, fixing the whole
  cooperative-scheduling class. Bigger, cross-repo; the bridge would drive
  the determinism-hypervisor half.

Build the **non-preemptive probe reproducer** first (`02`) — it's the fast
inner loop *and* the test of whether (A) actually resolves it.

## Files

| File | Contents |
|---|---|
| `01-root-cause.md` | Observed facts, corrected environment analysis, the two experiments, anchors |
| `02-fix-and-verify.md` | Fix (A) with its real integration work, fix (B), the reproducer + its PIT/TSC caveat, verification |
