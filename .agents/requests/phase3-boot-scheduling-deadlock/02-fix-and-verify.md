# Fix And Verification

Build the reproducer (§3) first — it is the fast inner loop and the only
thing that proves a fix actually works, since neither approach below is
guaranteed by the (reviewed, still-partial) diagnosis.

## Fix A — Agent-side: epoll-block the boot waits

Convert the two boot waits (`control.rs::recv` `:214`,
`wait_for_expected_regions` `runtime.rs:366`) from the `sched_yield` spin
to an `epoll`-blocking wait over the control fd + the region-IPC fds, so
the agent deschedules until real I/O readiness. This is the smaller change,
but it is **not** "reuse the supervise loop" — real integration work:

1. **New control-fd epoll registration.** The post-Ready supervise loop
   epolls the region-IPC listener + accepted conns (`supervise.rs:285,314,375`)
   but the workload control fd is **never** added to any epoll set
   (`workload_control` is merely retained, `runtime.rs:203`). You are
   adding a new registration, with its lifecycle (add on socketpair
   create; remove/replace on workload exit — mirror the pipe-fd dance at
   `supervise.rs:368-377,477-491`). `ControlSocket` (`control.rs:36`) also
   has no public raw-fd accessor yet.
2. **Resolve the epfd-access boundary.** `control::drive_refwork_start`
   only sees `&ControlSocket` + a `progress` closure and is deliberately
   decoupled from `Supervisor`/its epfd (`control.rs:1-7`). Decide: pass
   the epfd/`&mut Supervisor` in, or move the recv-loop driver into
   `runtime.rs`/`supervise.rs` where the epfd lives. This is a design call
   the spec can't make for you — pick one and note it.
3. **Bound redesign: icount → wall-clock/wakeup.** The current caps are
   sized as *guest-instruction* proxies against the host icount HARD_CAP
   (`control.rs:18-29`, `runtime.rs:19-27`). `epoll_wait` blocks **without
   burning guest instructions**, so a hang there will never trip the
   HARD_CAP — the fallback must become a wall-clock or bounded-wakeup
   budget that boot-faults with the named leg. Get this wrong and you
   move the silent-hang failure mode from "guest HARD_CAP" to "host CI
   hangs forever" — the exact class this request exists to kill.
4. **Preserve the fast-fail test mode.** The caps are shrunk under
   `#[cfg(test)]` (`control.rs:30-33`, `runtime.rs:28-31`) so
   `unit_control_faults_before_ready_when_workload_does_not_reply`
   (`runtime.rs:857`) fails fast. The epoll version needs an equivalent
   short test-mode budget or that test goes slow/flaky.
5. **Expect the READY icount to change.** Switching spin→block changes the
   guest syscall stream, so the READY-point icount and its state hash will
   shift. That is expected, not a regression — do not chase the old value;
   the deployed READY snapshot is regenerated downstream (step 3) anyway.
   (No hard-coded local golden breaks: `m2_acceptance.rs`'s icount check is
   self-consistency-only unless `DETGUEST_STRICT_ICOUNT=1`.)

## Fix B — Deterministic tick (alternative, more complete)

The hypervisor already supports deterministic interrupt injection and has
a paravirt timer device (`inject.rs`, `dh-devices/src/clock.rs`) — they're
just never armed because the guest kernel has no driver and the cmdline
disables the HW timers. Giving the guest an **armed deterministic tick**
(a guest driver for DH's pv-timer + the worker arming it) would restore
preemptive scheduling for the *whole* guest, fixing both the agent's wait
and any workload tick-dependency in one place. Bigger and cross-repo (the
bridge drives the determinism-hypervisor half); worth stating in your
resolution whether you judge A sufficient or B necessary — which the
reproducer answers.

## §3 — The Non-Preemptive Probe Reproducer (build this first)

The probe can't reproduce symptom 1 because its PIT gives preemption.
A probe variant with **no timer-interrupt delivery** reproduces the
deadlock in ~30 s with no real-worker handoff — and, crucially, tells you
whether Fix A is *sufficient* or you need Fix B.

**Caveat (not a clean toggle):** `harness/mod.rs:118-132` creates
`create_irq_chip` + `create_pit2` back-to-back, and the load-bearing
comment there notes `KVM_PIT_SPEAKER_DUMMY` is required or *"the kernel's
PIT-polled TSC calibration spins forever."* Naively deleting the PIT risks
trading the scheduling deadlock for a TSC-calibration hang. So the
non-preemptive variant must suppress timer *interrupt delivery* while
keeping whatever the boot needs to not TSC-hang — closer to a second
harness configuration than a flag. Validate it wedges *at the region /
GameLoaded point* (matching the real-worker trail), not at TSC calibration.

Assert: current agent → wedges before Ready; after Fix A → reaches and
**holds** Ready; the preemptive probe and m2/m4 suites stay green
(guard-reversion-proven, ecosystem style).

## Verification Loop

1. **You:** the non-preemptive-probe test goes red → green with the fix.
   If it stays red with Fix A, that's the signal Fix B (or more) is needed.
2. **Us (bridge, final gate):** a real-worker `dh-m9-ready-handoff` reaches
   `Ready` and snapshots. The probe is a model; the real worker is truth.

Green = the handoff yields the step-2 exit evidence (READY icount, region
count 3 / gen 6, state hash), unblocking READY-snapshot regeneration
(step 3), the `BRIDGE_REAL_SNAPSHOT_REF` cutover (step 4), and the first
real frame in the browser.

## Handback

`03-resolution.md` here: which fix (A/B/both), whether the non-preemptive
reproducer was built and its result, the commit(s), and the
reference-workload lock bump — that is the `rev = "<full guest-sdk sha>"`
line in `reference-workload/image/guest-sdk.lock` (its build refuses on
mismatch). We re-run the real worker and answer with `04-verification.md`.
