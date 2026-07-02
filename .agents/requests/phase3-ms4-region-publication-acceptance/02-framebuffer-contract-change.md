# New Context: The Framebuffer Region Contract Changed On 2026-07-02

If you last synced against determinism-hypervisor at `b973753` (the rev the
refwork audit inspected) or earlier, one contract change landed since that
affects Ms4 work and some existing evidence references.

## What Changed

determinism-hypervisor `5698d7e` ("Derive framebuffer geometry from D7
layout_version contract"; decision record
`../determinism-hypervisor/docs/decisions/framebuffer-region-geometry.md`;
full history in
`../determinism-hypervisor/.agents/requests/rom-bridge-getframebuffer-region-contract/`):

- `GetFramebuffer` and `CaptureSpec.framebuffer` now derive geometry from the
  manifest entry's `layout_version`. **layout_version 1 = raw pixels only:
  XRGB8888, 256×224, stride 1024, exactly 229,376 bytes. No in-region
  descriptor header.**
- The previous 16-byte in-region descriptor parse and the capture-path
  descriptor heuristic are **deleted**. A descriptor-bearing framebuffer
  region no longer has any consumer.
- Any other `layout_version`, or a `layout_version 1` region whose length is
  not 229,376 bytes, gets `FailedPrecondition` naming the offender. An
  all-zero region is a valid black frame.

This matches the reference-workload D7 contract and `RegionEntry` as it
exists in your `detguest-wire` (`layout_version` is the only geometry
channel), so for your real-workload path this is the contract you were
already targeting. The changes to absorb are at the edges:

## What This Invalidates Or Touches In Your Orbit

1. **The staged fixture geometry.** The deployed READY snapshot's staged
   guest publishes a 4,096-byte framebuffer region; the hypervisor now
   rejects it with `layout_version 1 expects 229376 bytes, got 4096`
   (observed live through the bridge, 2026-07-02). If
   `m9_refwork_contract.rs` (or any VM test workload) publishes a
   framebuffer smaller than the D7 length under `layout_version 1`, those
   paths now error where they previously half-worked. Publish the full
   229,376 bytes or the region will be rejected.
2. **Stale evidence references.** The refwork audit cites the hypervisor
   test `descriptor_framebuffer_fixture_feeds_capture_and_get_framebuffer`
   as DH-5 evidence — that test and the descriptor-bearing
   `framebuffer_fixture.asm` were deleted in `5698d7e` (replaced by
   `framebuffer_layout_contract_is_enforced` and a D7-sized capture
   fixture). Don't chase the old names.
3. **Deployment state.** The running `dh-workerd` on this host already has
   the fix (built from `ff1e88c` in the clean worktree
   `~/git/preestablished/.dh-clean-ff1e88c`; do not remove that worktree
   while it is the deployed binary). Note the hypervisor checkout's `main`
   also has two in-flight uncommitted files (`m9_handoff.rs`, `Cargo.lock`)
   from a concurrent session — coordinate before rebuilding from that tree.

## The Good News

Once your Ms4 region publication is real and the workload's framebuffer is
published at the D7 length, the entire host-side read chain
(`GetFramebuffer`, `CaptureSpec`, bridge preview) works with **zero further
changes** — we verified every link of it downstream of the region bytes.
