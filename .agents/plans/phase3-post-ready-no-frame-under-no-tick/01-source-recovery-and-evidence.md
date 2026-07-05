# 01 - Source Recovery and Failure Evidence

The named request directory is missing locally. This package prevents the next
agent from implementing against a guessed symptom.

## Step 1: Re-check the Request Source

Start with:

```bash
git pull --rebase
find .agents/requests -maxdepth 2 -type d -name 'phase3-post-ready-no-frame-under-no-tick' -print
find /home/infra-admin/git/preestablished -path '*phase3-post-ready-no-frame-under-no-tick*' -print
bd search "post ready no frame"
bd search "no tick frame"
```

If `.agents/requests/phase3-post-ready-no-frame-under-no-tick/` now exists,
read every file in it before touching code. Treat that request as authoritative
where it conflicts with this inferred plan, and update `00-overview.md` in this
plan directory before implementing.

## Step 2: Pin the Actual Stop Boundary

Collect or reconstruct the bridge-side evidence that led to the request:

- exact command or API operation that timed out: `NextSdkEvent(FrameMark)`,
  `at_frame`, first-frame render, framebuffer read, `Run`, or another stop;
- whether the VM was live from boot or restored from a READY snapshot;
- guest-sdk commit, reference-workload commit, package/image build id, and
  whether `reference-workload/image/guest-sdk.lock` had the no-timer boot fix;
- worker config proving the no-tick shape: no PIT/APIC timer delivery,
  `notsc tsc=unstable clocksource=jiffies noapictimer lpj=4096`;
- timeout class: icount hard cap, wall timeout, HLT/no-instruction progress,
  or VM exit loop;
- event stream after READY, including all ring W events and any
  `WorkloadExited` or P0 `LogLine`;
- ring W producer/consumer indices and drop counters at timeout;
- pv-pad `FRAME_COUNTER` observations, if the worker records them;
- manifest and region reads before/after the failed run, especially the
  reference workload's `meta` frame counter and framebuffer bytes.

Do not skip ring W. `Ready` is on ring A, but frame progress is signaled by
ring W `FrameMark` plus pv-pad `FRAME_COUNTER`.

## Step 3: Classify the Failure Before Fixing

Use the evidence to choose one classification:

1. Guest made no frame progress: no `FrameMark`, no `FRAME_COUNTER`, meta frame
   unchanged, and no workload death. This points to workload starvation or a
   workload-side tick dependency.
2. Guest produced a frame boundary but the host did not stop: `FRAME_COUNTER`
   advanced or ring W contains `FrameMark`, but `NextSdkEvent` / first-frame
   logic did not observe it. This points to downstream frame-boundary drain or
   stop-predicate handling.
3. READY-snapshot restore mismatch: live boot advances frames, restored child
   does not. This points to snapshot state, host channel reattach state, pv-pad
   state, or a snapshot taken at the wrong boundary.
4. Workload exited or faulted: `WorkloadExited`, stderr `frame loop failed`, or
   P0 agent log. Fix the named fault, not the no-tick machinery.
5. Ring W backpressure: ring W full, critical `FrameMark` stuck in
   doorbell-retry, or prod/cons inconsistent. This is a drain/backpressure
   problem, not a scheduler problem.

Only classification 1 with a local red reproducer should reopen the "guest
needs a tick" line of thought. Classification 2 is probably not a guest-sdk
runtime bug unless the request explicitly changes the SDK wakeup contract.

## Deliverable

Add a short evidence note before code work begins. If the original request
directory remains absent, create a local resolution/evidence file under the
request path or reference the Beads issue used for the implementation. Include
the classification above and the raw command lines used to reproduce it.
