# Plan: Post-Ready No Frame Under No Tick

This plan was requested for `.agents/requests/phase3-post-ready-no-frame-under-no-tick/`,
but that request directory is not present in this checkout as of 2026-07-05
after `git pull --rebase` reported "Already up to date". Before implementing,
re-check the path and reconcile this plan against any newly arrived request
text. The assumptions below are inferred from the adjacent Phase 3 no-timer
handoff docs and current code.

## Assumed Ask

The bridge/operator can now boot the real workload to guest-sdk `Ready` under
the deterministic worker's no-timer environment, but a post-Ready run does not
produce the first or next frame when there is no guest tick. The likely visible
symptom is a `NextSdkEvent(FrameMark)` / `at_frame` / first-frame render timeout
after restoring or continuing from a READY snapshot.

## Goal

Under `VmConfig.timer_interrupts = false` plus `TIMERLESS_CMDLINE_FLAGS`, a
post-Ready workload must make frame progress through the normal frame-boundary
contract:

- the SDK emits `FrameMark { frame_index }` on ring W;
- the SDK then writes the same frame index to pv-pad `FRAME_COUNTER`;
- the host observes the frame boundary without relying on periodic timer exits;
- region reads after the frame reflect current guest memory;
- any genuine no-progress case is bounded by a host wall deadline and leaves
  enough evidence to distinguish guest starvation, workload death, ring-drain
  failure, and snapshot/restore mismatch.

## Load-Bearing Facts

- The pre-Ready no-timer boot deadlock was fixed by agent-side epoll blocking
  waits (`phase3-boot-scheduling-deadlock`, guest-sdk `70851a2` and later).
- The verified bridge handoff reached READY and snapshotted under the real
  no-timer worker. See
  `.agents/requests/phase3-boot-scheduling-deadlock/04-verification.md`.
- `detguest_sdk::frame_mark()` currently writes the critical `FrameMark` record
  to ring W without an unconditional doorbell, then writes pv-pad
  `FRAME_COUNTER`.
- The spec intentionally treats the `FRAME_COUNTER` MMIO write as the
  frame-boundary VM exit. A host may drain ring W inside that exit; the
  `FrameMark` record is guaranteed visible because it precedes the write.
- Existing `refwork_ready_hold.rs` has a no-timer arm, but it only logs frame
  advance as a bonus observation. It does not require a post-Ready frame under
  no tick, and it does not model READY-snapshot restore as a separate child.

## Non-Goals

- Do not revive Fix B from the boot-deadlock plan (a deterministic guest tick)
  unless the reproducer proves the workload itself needs a tick and the bridge
  accepts cross-repo work.
- Do not change the frame-boundary ABI just to make a local test convenient.
  Any SDK doorbell change must be justified by a red reproducer and must update
  the API/architecture docs.
- Do not hide this behind nondeterministic timers, sleeps, or polling threads.

## Packages

| File | Contents | Depends on |
|---|---|---|
| `01-source-recovery-and-evidence.md` | Recover the missing request or collect equivalent bridge evidence; pin the actual failing boundary. | - |
| `02-no-timer-post-ready-reproducer.md` | Add local live-boot and READY-snapshot no-timer frame reproducers. | 01 |
| `03-frame-boundary-contract-and-fix.md` | Decision tree for guest-sdk vs downstream fixes, including the only acceptable SDK doorbell shape. | 02 red/green result |
| `04-verification-and-handoff.md` | Required test matrix, resolution notes, and handoff content. | 02 + 03 |

## Tracking

Use Beads for implementation tracking. Before editing plan/request/code, run
`bd prime` and `bd dolt pull`, then create one issue per package, wire
dependencies in package order, claim issues as work starts, and close them with
the exact verification evidence. Run Beads commands serially; the embedded
backend takes an exclusive writer lock. Do not use markdown task lists as the
implementation tracker.
