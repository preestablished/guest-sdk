# Resolution: Fix A (agent-side epoll-blocking boot waits) — sufficient

Resolved 2026-07-04 by the guest-sdk session against plan
`.agents/plans/phase3-boot-scheduling-deadlock/`. Beads guest-sdk-io8 /
-2xq / -u5e under epic guest-sdk-226.

## 1. Which fix

**Fix A alone. Proven sufficient by the reproducer; Fix B not needed.**

What the reproducer demonstrated (the diagnosis was suspected-only; this is
what actually held): the trigger is exactly the absence of timer-interrupt
delivery, and blocking the agent sufficed. With the pre-fix agent under the
non-preemptive harness, the boot completes all three region registrations
(generation 6), reaches the agent's `boot: rw-ready` breadcrumb, then goes
totally silent to the wall deadline with **no poll-cap boot fault ever
firing** — at native KVM speed a 500 K-iteration spin cap trips in seconds,
so its silence means the agent wasn't even spinning: it was parked/starved,
never scheduled again. That is the cooperative-scheduling starvation class,
in the agent's own boot waits.

No residual workload tick-dependency surfaced. With the fixed agent, the
identical no-tick environment reaches and holds Ready — and as a bonus
observation, the refwork frame loop **advances without any tick** (meta
frame counter 0 → 3040 during the 3 s hold), so the plan's precautionary
relaxation of the no-timer frame-advance assertion was not even needed in
practice (it stays relaxed, deliberately, for robustness).

The fix's shape (guest-sdk `70851a2`): the two pre-Ready waits — control
reply recv (`control.rs`) and expected-regions gate (`runtime.rs`) — now
park in the supervisor's epoll (`Supervisor::wait_boot_io`, a mini
supervise pass: targeted workload reap on the sigfd wake, pipe drain with
HUP discipline, timerfd drain, unconditional region-IPC service). The
control fd is epoll-registered for the boot leg only. The icount-proxy
poll caps became wakeup caps (600 wakeups, 100 ms advisory epoll timeout
that can only fire in a tickful environment).

## 2. Reproducer

- **Built**: yes, two tiers, plus a probe hook (`BOOT_PROBE_NO_TIMER=1`).
- **Suppression mechanism**: GSI routing (the plan's primary), no fallback
  needed — `create_vm_core` replaces the post-`KVM_CREATE_IRQCHIP` default
  routing table with the same table minus GSIs 0 and 2, keeping irqchip +
  PIT (`KVM_PIT_SPEAKER_DUMMY`) so in-kernel port 0x61/0x42 stay and TSC
  calibration cannot spin. Validated end-to-end: same image + cmdline with
  delivery on reaches Ready; with delivery suppressed it wedges.
- **Discovery worth keeping**: the timerless cmdline **must include
  `lpj=4096`** (the worker's `BZIMAGE_FORCED_CMDLINE` carries it). Without
  the preset, whenever the kernel's quick TSC calibration against the PIT
  counter fails (it is flaky), `calibrate_delay` falls back to converging
  on jiffies — frozen — and the boot hangs **pre-userspace**, a wrong-red
  that mimics the wedge. `TIMERLESS_CMDLINE_FLAGS` in the harness carries
  the full set: `notsc tsc=unstable clocksource=jiffies noapictimer
  lpj=4096`.
- **Tier 1** (`tests/vm/tests/no_timer_boot.rs`, game-mat image,
  `DETGUEST_VM_TESTS=1`): did **not** wedge pre-fix — `game-load-check`'s
  boot dialogue happens not to hit the starvation. Kept as a permanent
  cheap guard (full production boot shape, no external artifact); its
  module docs record the observation. Green pre- and post-fix.
- **Tier 2** (`tests/vm/tests/refwork_ready_hold.rs::no_timer_real_harness_
  reaches_and_holds_ready`, gated on `REFWORK_READY_INITRAMFS`): the
  red→green arbiter.
  - **Red** (your package-04 artifact, guest-sdk `914dbde` +
    reference-workload `aa69558`): wedged post-registration. Key trail
    (probe dump, 30 s run):

    ```text
    Hello (A#0) · WorkloadStarted pid 15 (A#1) · "boot: helloack" (A#2)
    wram / framebuffer / meta: NameIntern + RegionRegister, gen 2/4/6
    "boot: gameloaded" (A#9) · "boot: rw-ready" (A#10)
    ...then nothing. No "boot: start-sent", no Ready. Serial ends at
    "Run /init as init process". Test: Timeout at 120 s, no Ready.
    ```

    (Slightly later wedge point than your real-worker trail — rw-ready
    vs. never-received-GameLoaded — consistent with nondeterministic
    scheduling order in the cooperative regime; same class, same
    signature: post-registration gen 6, silent, no Ready.)
  - **Green** (artifact rebuilt against the fixed agent; local
    **uncommitted** `guest-sdk.lock` bump per the plan — the committed
    bump is yours): both arms pass, Ready held, breadcrumbs in order,
    frames advancing. `test result: ok. 2 passed` in 7.6 s.
- **Guard-reversion proof**: fix reverted to the sched_yield spins (old
  caps restored, image rebuilt via `xtask image build --agent-bin`) →
  tier 2 red again (Timeout, no Ready); fix restored → green. Recorded in
  the test's module docs (guest-sdk `4d248df`).

## 3. Commits (guest-sdk)

| Commit | Contents |
|---|---|
| `d3ac547` | Harness no-timer mode (GSI routing), timerless cmdline (incl. lpj), probe hook, both reproducer tiers |
| `70851a2` | **Fix A**: `wait_boot_io`, boot-only control-fd epoll registration, targeted `reap_workload`, wakeup-cap bound redesign, agent unit tests |
| `4d248df` | Guard-reversion record + tier-1 observation in test docs |
| (tip) | This resolution |

Test matrix at the tip: agent unit tier 56/56 (~1 s), host workspace
green, VM preemptive suites green (m2 7/7, m4 1/1, m4_snapshot 3/3,
game_materialization 3/3), tier 1 green, tier 2 green both arms.

## 4. Lock bump line (your action)

```toml
rev = "4d248df4d97df502cf988fb9f6007a0cb54d4740"
```

for `reference-workload/image/guest-sdk.lock`. That sha contains the fix
and the full reproducer suite; the agent binary is identical from
`70851a2` onward, so pinning the current guest-sdk `main` tip (which adds
only this resolution file) is equally valid — pick whichever matches your
convention; `verify_pinned_rev` wants your sibling checkout at exactly the
pinned sha.

## 5. Bridge action items

1. **Re-run `dh-m9-ready-handoff`** (your final gate → your
   `04-verification.md`), including the committed `guest-sdk.lock` bump
   (item 4). Our local bump used for the tier-2 green stays uncommitted on
   our side.
2. **Expect a shifted READY icount / state hash.** Spin→block changes the
   guest syscall stream, so the READY-point icount and its state hash WILL
   differ from any pre-fix observation; regenerate the deployed READY
   snapshot (your step 3) rather than chasing the old value. We did not
   capture a worker-comparable icount from the harness (the harness's perf
   counter cadence isn't your icount), so take the new value from your
   handoff run.
3. **Confirm the worker has a wall-clock budget.** Deliberate residual
   risk of the bound redesign: with the agent parked in `epoll_wait` and
   no tick, a genuinely dead workload leaves the vCPU in HLT burning *no*
   instructions — the icount HARD_CAP will never trip for that failure
   mode. The guest-side wakeup caps only bound *wakeful* waits; the
   host-side wall deadline is the only backstop for a dead-block. Our
   harness's `run_until` wall deadline (with serial + event dump) covers
   CI; the worker must own the equivalent on its side.
4. **Two new-but-correct pre-Ready event-stream shapes** are now possible
   (both from `wait_boot_io`): (a) workload stdout/stderr LogLines before
   Ready (the boot wait drains the pipes); (b) `WorkloadExited` before
   Ready when a workload dies mid-boot (reap-inside-wait — the boot then
   faults loudly instead of spinning out a cap). `detguest-host`'s drain
   is a pure decoder with no Ready-first state machine, but confirm no
   worker-side consumer asserts Ready-first ordering.
5. **Bonus observation** for your books: under the fixed agent the refwork
   frame loop advances with no tick at all (0 → 3040 frames in a 3 s
   host-wall hold), i.e. frame pacing is not tick-dependent either. Fix B
   (deterministic pv-timer tick) remains unimplemented and, on this
   evidence, unnecessary for the boot path.
