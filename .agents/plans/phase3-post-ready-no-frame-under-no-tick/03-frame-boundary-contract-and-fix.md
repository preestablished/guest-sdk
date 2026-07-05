# 03 - Frame-Boundary Contract and Fix Decision

This package chooses the fix after package 02 establishes red/green behavior.
Do not start by adding timers or unconditional sleeps.

## Contract to Preserve

The frame boundary is one logical signal with two views:

1. `detguest_sdk::frame_mark()` emits critical `FrameMark { frame_index }` on
   ring W and release-stores ring W producer.
2. It then writes the same frame index to pv-pad `FRAME_COUNTER`.
3. The `FRAME_COUNTER` MMIO write is the frame-boundary VM exit. A host that
   drains ring W inside that exit must see the preceding `FrameMark`.

This is documented in `prompts/docs/guest-sdk/API.md` and
`prompts/docs/guest-sdk/ARCHITECTURE.md`. A normal `FrameMark` does not
currently doorbell unless ring W is full and the critical-event retry path has
to free space.

## Decision Tree

### Case 1: Local Tests Pass, Worker Still Misses Frames

Treat this as a downstream worker or bridge integration bug. Do not change
guest-sdk just to mask it.

Handoff requirements:

- worker must drain ring W at the pv-pad `FRAME_COUNTER` MMIO exit or otherwise
  include ring W in the `NextSdkEvent(FrameMark)` observation path;
- worker must not rely on periodic timer exits for post-Ready event drain;
- READY-snapshot restore must reattach the channel and restore host-side ring
  C/I producer seqs before pushing commands;
- stop predicates must consider ring W events, not only ring A `Ready` and
  agent events;
- wall-clock deadline remains mandatory because a no-tick HLT consumes no
  icount.

File the downstream follow-up with the synthetic and real reproducer evidence.

### Case 2: Ring W Contains FrameMark, Harness/Host Did Not Observe It

Fix the host-side drain boundary, not the SDK producer. In this repo that means
the VM harness if the local reproducer failed; downstream if only the worker
failed.

For the local harness, `tests/vm/src/harness/pio.rs::pvpad_write` already
records `frame_counter_writes` and drains the channel inside the
`FRAME_COUNTER` MMIO exit. If a test proves otherwise, fix that path and add a
unit test around `apply_pvpad_write` / `pvpad_write` plus an integration test
that `FrameMark` is visible after the MMIO write.

### Case 3: Live No-Timer Guest Makes No Frame Progress

First prove whether the workload is alive:

- no `WorkloadExited`;
- no stderr `frame loop failed`;
- no P0 agent fault;
- vCPU not powered off;
- ring W not full with `FrameMark` retry stuck.

If the workload is parked waiting on a guest timer or scheduler mechanism, this
is not an agent boot-wait bug. Either the reference workload must remove that
dependency, or the bridge must reopen the deterministic tick work in the
hypervisor. Do not put a busy loop in the agent to "kick" the workload.

### Case 4: Restore-Only Failure

Focus on snapshot fidelity:

- `VmHarness::from_snapshot()` must be called with `timer_interrupts = false`;
- `VmSnapshot` carries `PioState`, including pv-pad frame counter and input
  schedule;
- channel host state must carry ring C/I producer seqs;
- pending ring W records at the READY boundary must be drainable after restore;
- if first post-restore frame uses `inject_point`, pending-inject host state
  may matter. Track this against `guest-sdk-4bc` if the current accessors are
  insufficient.

Add a minimal synthetic restore regression before changing shared snapshot
code.

## SDK Doorbell Escape Hatch

Only use this if the request explicitly makes guest-sdk responsible for waking
hosts that do not drain at `FRAME_COUNTER`, or if a local red proves there is
no other correct boundary available.

Do not replace `frame_mark()` with `emit_w_event_with_doorbell()`: that helper
doorbells before the `FRAME_COUNTER` MMIO write and can let a host observe
`FrameMark` before the frame-boundary exit.

The least-bad SDK shape is:

1. emit `FrameMark` to ring W;
2. write pv-pad `FRAME_COUNTER`;
3. then issue `doorbell_w()`.

That preserves the documented record-before-frame-counter ordering and gives
hosts that ignore the pv-pad exit a wake immediately after the boundary. If
implemented, update:

- `crates/detguest-sdk/src/lib.rs::SdkState::frame_mark`;
- unit tests around `frame_mark_publishes_record_before_frame_counter_write`;
- API/architecture docs to say a post-boundary ring-W doorbell is emitted;
- VM tests that assert icount or exact event ordering, because every frame now
  has an extra PIO exit.

This change has performance and deterministic-icount cost. Prefer a host-side
drain fix when the contract already assigns the frame-boundary exit to the
host.
