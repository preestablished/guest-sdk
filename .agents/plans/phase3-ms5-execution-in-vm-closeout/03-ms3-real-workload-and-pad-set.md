# Package 03 — Ms3 Real-Workload And PAD_SET Acceptance

This package is independently executable once package 01 clears M9, image,
and preflight gates. It must not wait for the Ms5 replay implementation.

## A. Real-workload lifecycle acceptance

Implement a focused ignored test (prefer a new
`tests/vm/tests/m3_acceptance.rs` rather than hiding the gate in a broad test)
that boots the package-01 real image under Linux, runs it through
detguest-agent, drains through `detguest-host`, and asserts the documented Ms3
lifecycle and golden event stream. Reuse the real image selection/config from
`refwork_ready_hold` rather than inventing another environment contract.

The oracle must cover agent/workload readiness, SDK attach, expected event
types/order, exact golden stream hash where the bead requires it, zero
unexpected critical drops, and clean workload progress. On failure persist
serial, decoded events, channel/drop counters, image identity, and runner
revisions in the common evidence root.

## B. PAD_SET record-to-latch acceptance

The current `PvPad::schedule` is only an in-process stand-in. Add a neutral
type such as `DecodedPadSet { at_frame, port, buttons }` and an adapter to the
existing latch. Contract-test synthetic decoded fixtures, then feed it through
the exact upstream boundary found in package 01. If none exists, report the
upstream gap and do not claim real PAD_SET acceptance from direct schedule
calls. Do not add a second DHILOG parser to guest-sdk.

Add `Observed::pvpad_reads` (frame counter at read, port, value) in the MMIO
read-exit path and pair it with a canonical workload LogLine per poll. This
makes once-per-frame observable; the latch itself does not enforce call count.

Drive multiple ports, multiple updates in one frame, sparse frames, and a
seeded burst. Assert end to end:

1. each decoded PAD_SET lands at the requested frame and port;
2. workload `poll_input` observes the held latch value exactly once per
   simulated frame according to the SDK contract;
3. ring I contains control records only—input data never appears there;
4. each `FrameMark(F)` is drained before the matching FRAME_COUNTER write
   opens the next work period; and
5. repeating the seed yields the same frame/input/event trace.

Record a human-readable PAD_SET → latch → poll observation table plus the raw
decoded event trail. Add negatives for an invalid port and deliberately shifted
`at_frame` so the ordering oracle is proven sensitive.

## C. Beads and docs inputs

Close `guest-sdk-m3-vm-real-workload-e2e` after A passes on the pinned real
image. Close `guest-sdk-m3-input-path-acceptance` after B passes through the
record boundary, not merely direct `PvPad::schedule`. Preserve exact commands
and artifact paths for package 06. Defer final prose edits to package 05, but
collect the as-built facts needed by `guest-sdk-m3-docs-as-built`.
