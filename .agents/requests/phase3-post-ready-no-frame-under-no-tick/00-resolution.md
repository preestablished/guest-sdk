# Resolution: Post-Ready No Frame Under No Tick

Resolved 2026-07-05 by implementing
`.agents/plans/phase3-post-ready-no-frame-under-no-tick/`.

## Source Recovery

The requested source directory was absent when implementation began.
`git pull --rebase` was up to date, `find` located only the plan directory,
and `bd search "post ready no frame"` / `bd search "no tick frame"` found no
matching request issue. `bd dolt pull` could not run in this checkout because
the embedded Dolt remote reports that a branch must be specified, while the
`bd dolt pull` wrapper exposes no branch flag. Evidence was recorded on
`guest-sdk-6jd`.

## Classification

Local evidence does not show a guest-sdk runtime frame-boundary bug.

The new no-timer post-Ready VM guard proves that, with
`VmConfig.timer_interrupts = false` and the timerless cmdline, the M4
frame-loop fixture:

- reaches `Ready`;
- emits a ring-W `FrameMark` after Ready;
- writes the matching pv-pad `FRAME_COUNTER`;
- mutates readable regions after that frame;
- repeats the same behavior after `VmHarness::from_snapshot()` when the child
  is restored with `timer_interrupts = false`.

Therefore no SDK post-frame doorbell or deterministic guest tick was added.
The existing contract remains: `frame_mark()` publishes `FrameMark` before
writing `FRAME_COUNTER`, and the harness drains ring W at the frame-counter
MMIO exit.

## Changes

- Added `tests/vm/tests/no_timer_post_ready.rs`.
  - `no_timer_live_boot_produces_post_ready_frame`
  - `no_timer_ready_snapshot_restore_produces_next_frame`
- Tightened `tests/vm/tests/refwork_ready_hold.rs` so both timerful and
  no-timer arms require the reference workload frame counter to advance after
  Ready, instead of treating no-timer frame progress as a bonus observation.

## Verification

Completed:

```text
cargo test -p detguest-wire -p detguest-host -p detguest-sdk -p detguest-agent
  => detguest-agent 56 passed; detguest-host 20 passed + loopback 1 passed;
     detguest-sdk 29 passed; detguest-wire 47 unit + 9 golden + 8 proptest passed

DETGUEST_VM_TESTS=1 cargo test -p detguest-vmtest --test no_timer_boot -- --nocapture
  => 1 passed

DETGUEST_VM_TESTS=1 cargo test -p detguest-vmtest --test no_timer_post_ready -- --nocapture
  => 2 passed

cargo test -p detguest-vmtest --test refwork_ready_hold -- --nocapture
  => 2 passed; both tests skipped their bodies because REFWORK_READY_INITRAMFS was unset
```

## Residual Risk

The real reference-workload artifact was not available locally
(`REFWORK_READY_INITRAMFS` unset), so the strengthened real-artifact no-timer
assertion is compile-checked but not exercised here. A downstream worker that
still misses post-Ready frames should first verify its ring-W drain and
`NextSdkEvent(FrameMark)` stop path at the pv-pad `FRAME_COUNTER` exit; the
local guest-sdk harness path is green.
