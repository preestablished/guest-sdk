# 02 - No-Timer Post-Ready Reproducer

Build the reproducer before changing runtime code. Current local evidence says
the real reference-workload frame loop advanced without a tick during the
boot-deadlock fix. This package turns that observation into a required,
snapshot-shaped test.

## A. In-Repo Synthetic Reproducer

Add the cheap no-timer post-Ready frame test using the existing M4 workload:

- workload: `tests/vm/workloads/src/bin/m4_regions.rs`;
- boot config: `image/boot.toml.m4-regions`;
- VM config: `timer_interrupts = false` and `cmdline = timerless_cmdline()`;
- no pv-blk and no refwork control leg. This is deliberate: the guard targets
  the SDK frame-boundary path (`FrameMark` + pv-pad `FRAME_COUNTER`) under no
  tick, not the game-device/control handshake already covered elsewhere.

Do not use the current `m9_refwork_contract` as the cheap reproducer unless
you first change the fixture. It performs pv-blk `WRITE`/`READ`/`FLUSH` on
frame 0, while `tests/vm/src/harness/pvblk.rs` is intentionally read-only and
returns `STATUS_BAD_REQUEST` for write/flush. A frame-0 pv-blk
`WorkloadExited` from that fixture is a fixture mismatch, not proof of a
no-tick frame failure. If an M9-shaped synthetic guard is required, either add
a no-write fixture/mode or implement a writable pv-blk overlay with focused
unit coverage.

Test shape:

1. Build/stage agent plus `m4-regions` into a dedicated initramfs
   output so it does not clobber other suites.
2. Boot to guest-sdk `Ready`.
3. Drain events and assert `Ready { region_count: 3, manifest_generation: 6 }`.
4. Capture the current pv-pad frame counter and `meta` frame value.
5. Run until at least one new `FRAME_COUNTER` write is observed, counting a
   delta from the current `Observed` baseline. Restored children start with
   fresh `Observed`, so child predicates should usually use
   `frame_counter_writes.len() >= N`; live-root predicates can compare the last
   observed value to the captured pv-pad frame counter.
6. Drain and assert a matching `OwnedPayload::FrameMark { frame_index }` exists
   on ring W.
7. Read `wram`, `framebuffer`, and `meta`; assert exact continuity between
   `FrameMark.frame_index` and the pv-pad `FRAME_COUNTER` write. For M4-style
   `meta`, remember the off-by-one contract: the workload writes completed
   frame metadata before `sdk::frame_mark()`, so `meta[0..4]` normally equals
   `FRAME_COUNTER - 1`. Also assert at least one mutable region changed.
8. Fail loudly on `WorkloadExited`, P0 agent logs, or timeout, including serial
   text and the drained events.

This is the cheap guard. It proves the repo's own SDK/agent/harness frame path
does not need timer interrupts after Ready.

## B. READY-Snapshot Restore Reproducer

Add a second leg to the same test or a separate test file that models the
deployed READY snapshot:

1. Boot the no-timer synthetic VM to `Ready`.
2. Drain events and take `VmHarness::snapshot()` immediately after the
   Ready-stop boundary.
3. Restore a child with `VmHarness::from_snapshot()` using a config whose
   `timer_interrupts` is still `false`.
4. Before running the child, read all regions and record only observable state:
   pv-pad frame counter, region bytes, and drained events. Do not require
   private channel internals; `from_snapshot()` restores host C/I producer seqs
   internally, and missing intern/pending-inject accessors are separate work
   tracked by `guest-sdk-4bc`.
5. Run the child until one new `FRAME_COUNTER` write and one ring W
   `FrameMark` are observed.
6. Assert region mutation and exact frame-index continuity. If this leg pushes
   a command such as `ReverifyRegions`, use it as a public command-push
   continuity check rather than inspecting private channel state.

Important: `from_snapshot()` creates a fresh VM using `cfg.timer_interrupts`.
Passing a default timerful config would invalidate the test.

## C. Real Reference-Workload Gate

Strengthen `tests/vm/tests/refwork_ready_hold.rs` for the env-gated real
artifact path:

- keep the timerful arm's existing frame-advance assertion;
- change the no-timer arm from "log if frames advanced" to a required
  post-Ready frame assertion for this request;
- add a READY-snapshot child leg if the bridge failure involved restoring the
  READY snapshot;
- keep the existing `REFWORK_READY_INITRAMFS` / `REFWORK_READY_BZIMAGE` gating.

If the real artifact is not available, the synthetic reproducer is still the
in-repo regression guard, but the implementation is not fully verified against
the original symptom.

## D. Expected Outcomes

- If both synthetic and real no-timer tests pass before any fix, do not modify
  guest-sdk runtime code. The failure is likely downstream worker drain,
  stop-predicate, or snapshot-store behavior. Move to package 03's downstream
  handoff path.
- If live boot fails locally before the first post-Ready frame, classify with
  package 01 and fix only the failing layer.
- If live boot passes but READY-snapshot restore fails, focus on snapshot
  state: pv-pad state, in-kernel irq routing, `Channel::producer_seqs()`,
  ring W consumer index, pending inject state, and snapshot boundary.

Record red output before fixing whenever a local red exists. The red evidence
is part of the acceptance criteria.
