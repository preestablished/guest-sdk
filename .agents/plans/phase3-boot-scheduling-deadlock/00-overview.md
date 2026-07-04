# Plan: Boot Handshake Deadlock Under The No-Timer Worker

Answers `.agents/requests/phase3-boot-scheduling-deadlock/` (filed by
rom-operator-bridge, 2026-07-04). Read that directory first — this plan does
not repeat its context, and the request's caveats (the mechanism is
*suspected*, not pinned) shape the package order below.

## Goal (behavioral)

Under a guest with **no armed interrupt source** (the deterministic worker's
environment: no PIT/APIC-timer delivery, `jiffies` frozen), the agent's boot
handshake (`Hello → HelloAck → LoadGame → GameLoaded → Ready → Start` plus the
expected-regions gate) reaches and **holds** guest-sdk `Ready` instead of
wedging silently to the host's 10 B-instruction HARD_CAP. A genuinely dead
workload still produces a loud, named boot fault wherever a wake source
exists, and the silent-hang failure mode is bounded by the **host** wall-clock
deadline everywhere else.

## Approach: Fix A first, reproducer-gated; Fix B is a contingency

The request offers (A) agent-side epoll-blocking boot waits and (B) a
deterministic paravirt tick (cross-repo). This plan implements **A**, but only
*after* building the request's §3 non-preemptive reproducer, because the
diagnosis is explicit that A is not guaranteed sufficient (the workload
completed region registration under cooperative scheduling, so the residual
stall may involve a tick-dependent path in the *workload*). The reproducer is
the arbiter:

- Reproducer red (wedges) on the current agent → confirms the environment
  trigger.
- Reproducer green after Fix A → A is sufficient; ship it.
- Reproducer still red after Fix A → **stop, do not start Fix B here** — it
  is cross-repo (determinism-hypervisor + a guest kernel driver) and the
  bridge drives that half. Record the evidence in the resolution file
  (package 03) and hand back.

## Load-bearing design decisions (argued in the packages)

1. **The blocking wait lives on `Supervisor`, not in `control.rs`**
   (`wait_boot_io`, package 02). `control::drive_refwork_start` keeps its
   `&ControlSocket` + progress-closure shape; the `Idle` callback simply
   *becomes blocking*. This resolves the request's "epfd-access boundary"
   question (its point 2) with the smallest interface change: the epfd never
   crosses into `control.rs`.
2. **The control fd is registered in the epoll set for the boot leg only**
   (added before `drive_refwork_start`, removed after it returns on both the
   success and error paths). Post-Ready the agent neither reads nor writes
   the control socket today, and a boot-only registration means the supervise
   loop needs no new token handling and no HUP/busy-spin analysis. Revisit
   only when host-driven HashRequest/Shutdown relays land.
3. **`wait_boot_io` is a mini supervise pass, not a bare `epoll_wait`.** It
   must drain the timerfd (level-triggered; otherwise a pending tick turns
   the "block" back into a spin) and must **reap on sigfd readability**
   rather than merely draining it — consuming a SIGCHLD datum during boot
   without reaping would let a workload that dies mid-gate become an
   unreported zombie after Ready. Reaping during boot is correct and an
   improvement: workload death now *wakes* the boot wait, the next control
   recv sees EOF, and the boot faults immediately instead of spinning out a
   poll cap.
4. **Guest-side timeout bounds are advisory; the host deadline is
   authoritative.** In the no-tick guest, `epoll_wait(timeout)` can never
   time out — timer expiry is itself interrupt-driven. So the icount-proxy
   poll caps are replaced by *wakeup* caps with an epoll timeout that fires
   only in tickful environments (host unit tests, the PIT-ful probe, any
   future ticked worker). A dead-block in the no-tick guest parks the vCPU in
   HLT; the harness's `run_until` wall deadline (and the worker's own
   wall-clock budget — flag this to the bridge in the resolution) is the
   backstop. This is stated loudly in package 02 because the request warns
   the failure mode must not silently become "host CI hangs forever": the
   harness deadline + serial dump means it cannot.
5. **The non-preemptive harness variant keeps irqchip + PIT and suppresses
   delivery via GSI routing** (package 01), preserving the load-bearing
   `KVM_PIT_SPEAKER_DUMMY` port-0x61 behavior so the kernel's PIT-polled TSC
   calibration cannot hang — exactly the trap the request's §3 caveat names.
6. **Two reproducer tiers**: a cheap in-repo one (game-mat image, full
   control leg, `DETGUEST_VM_TESTS=1`-gated) and the authoritative
   real-refwork twin of `refwork_ready_hold.rs` (env-gated on
   `REFWORK_READY_INITRAMFS`). If the in-repo workload happens not to wedge
   pre-fix (the tick dependency may be refwork-specific), the refwork twin is
   the red→green criterion and the in-repo test remains as a guard.

## Invariants that must not break

- The boot-leg breadcrumb sequence (`boot: helloack` … `boot: evidence-done`)
  and its ordering assertions in `refwork_ready_hold.rs:158-170`.
- fd-3 retention for the workload's lifetime (symptom-2 fix `678dc81`;
  guarded by `control_leg_retains_workload_socket_and_names_its_legs`).
- `unit_control_faults_before_ready_when_workload_does_not_reply` stays fast
  (test-mode budget, package 02 step 5).
- The preemptive (timer-ful) probe, m2, m4, and game-materialization suites
  stay green.
- READY icount / state hash **will** shift (spin→block changes the syscall
  stream). Expected, not a regression; `m2_acceptance` icount is
  self-consistency-only unless `DETGUEST_STRICT_ICOUNT=1`. Note the shift in
  the resolution for the bridge's snapshot regeneration (its step 3).

## Packages (implement in order)

| File | Contents | Depends on |
|---|---|---|
| `01-no-timer-harness-and-reproducer.md` | Harness no-timer-delivery mode + probe hook + the two reproducer tests (expected red) | — |
| `02-agent-epoll-boot-waits.md` | Fix A: `wait_boot_io`, control-fd epoll lifecycle, bound redesign, test-mode budgets | 01 red confirmed |
| `03-verification-and-handback.md` | Full test matrix, guard-reversion checks, `03-resolution.md` handback, lock-bump note, Fix-B contingency | 01 + 02 |

## Tracking

Create beads before starting (`bd create`, one per package, `bd dep add`
02→01 and 03→02), claim as you go, close on completion. Do not use markdown
TODO lists.
