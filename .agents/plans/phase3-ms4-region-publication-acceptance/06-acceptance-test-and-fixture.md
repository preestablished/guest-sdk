# 06 — D7 fixture bump, M4 workload, the 100× acceptance, evidence

Closes bead `guest-sdk-m4-platform-readability-vm` (request blocker #3) and
absorbs the framebuffer contract change (request `02-framebuffer-contract-change.md`).

## A. Staged fixture bump (do first — independent, unblocks staged flows)

`tests/vm/workloads/src/bin/m9_refwork_contract.rs`:
- `FRAMEBUFFER_LEN: usize = 229_376` (was 4,096). D7 layout_version 1 =
  XRGB8888, 256×224, stride 1024, exactly 229,376 bytes, no in-region
  descriptor. The frame-loop's index mask `& (FRAMEBUFFER_LEN - 1)` still
  works (229,376 is NOT a power of two — replace the mask with
  `% FRAMEBUFFER_LEN` for the framebuffer index only; keep masks for the
  power-of-two WRAM/META).
- **Hold the region handles for process lifetime (review BLOCKER).**
  `publish_regions()` currently binds handles to `let _wram = …` locals that
  drop on return; with `03-…`'s drop-unregisters semantics that would DEAD
  all three regions before the control-`Ready` datagram. `std::mem::forget`
  each handle with a comment (the regions live until power-off by design).
- Leave everything else alone. The region flags/layout_version are already
  correct. RLIMIT_MEMLOCK is a non-issue: the agent's `spawn` already sets
  it to ∞ pre-exec (`supervise.rs:172-177`), and a 224 KiB `.bss` grows
  neither the binary file nor the initramfs.

## B. M4 acceptance workload: `tests/vm/workloads/src/bin/m4_regions.rs`

A dedicated deterministic workload (don't overload the m9 fixture, which is
contract-frozen against the hypervisor):

- Regions: `wram` 8,192 B, `framebuffer` 229,376 B
  (`RegionFlags::FRAMEBUFFER`), `meta` 256 B; all `layout_version 1`;
  registered via `sdk::register_region` (exercises the full new IPC path);
  handles held via `mem::forget` (see §A — drop would DEAD them).
- Per frame: read pad input via `sdk::poll_input()`; mix input into a
  deterministic PRNG-ish accumulator (same rotate/xor style as m9); scatter
  writes into wram + framebuffer; write `meta[0..4] = frame_le`,
  `meta[8..16] = acc_le`, `meta[16..24] = input_history_hash_le` (FNV-1a of
  all inputs consumed so far); `sdk::frame_mark()`.
- No `[unit.control]` (plain autostart), no pv-blk — keep the surface minimal.
- boot.toml fixture: autostart the unit and gate Ready on publication through
  the new path (making every boot of this fixture a regression test for
  01–03), using the real schema (`boot.rs:233-262` — array-of-tables, not
  the `name@version` shorthand):

  ```toml
  [[expected_region]]
  name = "wram"
  layout_version = 1

  [[expected_region]]
  name = "framebuffer"
  layout_version = 1

  [[expected_region]]
  name = "meta"
  layout_version = 1
  ```

  Follow how m2's `artifacts()` builds initramfs variants
  (`m2_acceptance.rs:70-153`, `image/build.sh`) and add an `m4` variant;
  register the new binary in `tests/vm/workloads/Cargo.toml`.

## C. The acceptance test: `tests/vm/tests/m4_acceptance.rs`

Gating: identical to m2 (`#[ignore = "KVM tier"]`, `DETGUEST_VM_TESTS=1`,
`/dev/kvm` assert, `--test-threads=1`). Child count: `DETGUEST_M4_CHILDREN`
env override, **default 100** (CI runs the real count; override is for local
iteration only and the evidence file records the actual count).

Host-side region reads go through the real host path: `detguest-host`
`Channel`/`read_manifest`/`read_region` over the harness `MemSlot` GuestMem
(the harness learns the channel GPA from CHANNEL_INIT; children re-attach
from the snapshot per `05-…`). **No guest round trip for reads** — reads must
not send any command or run the vCPU; that is the point of the milestone.
Structure the helper so this is self-evident (takes a `&VmHarness` memory
view only). Note `mem()` is `pub(crate)` and integration tests compile as a
separate crate — add a public read-only accessor on `VmHarness` as part of
this package.

Flow:

1. Boot the m4 fixture to Ready (fresh `VmHarness`); run to a warm-up
   boundary (e.g. frame 8).
2. **Root snapshot** via `05-…`.
3. Root baseline: read all three regions; record SHA-256 of each; assert
   framebuffer read returns exactly 229,376 bytes and meta frame counter ==
   warm-up frame.
4. For `i in 0..N` (N=100), sequentially:
   a. `VmHarness::from_snapshot(root)`.
   b. **Restore fidelity**: immediately re-read all regions; must equal root
      baseline hashes bit-exactly.
   c. **Reverify on restore** (`04-…`): `push_command(ReverifyRegions)`
      (works in children because `05-…` restores the host `Channel` +
      producer seqs), then `run_until` with predicate "RegionUpdate count ==
      live regions" and a generous deadline (the agent's ring-C poll rides a
      100 ms epoll cadence — never a fixed sleep); assert echoes and zero P0
      LogLines. Drain counts are **deltas since child start** (children start
      with fresh `Observed`), never absolute totals.
   d. Schedule child-specific inputs via the `05-…` PvPad schedule queue:
      seed = `i / 2` (children `2k` and `2k+1` get identical schedules;
      different `k` differ). 60 frames of inputs derived from the seed.
   e. Run 60 frames (predicate on FrameMark delta or meta frame counter
      reaching warm-up+60).
   f. Read regions again: meta frame counter == warm-up + 60; meta
      input-history hash matches the host-side recomputation for seed `i/2`
      (the recomputation must include the warm-up frames' inputs — zeros
      before any schedule — since the workload hashes every poll);
      record SHA-256 of wram/framebuffer/meta.
   g. Drop the child.
5. Cross-child assertions: for every pair `(2k, 2k+1)`: all three region
   hashes identical (**determinism**); for `k != k'`: wram hashes differ
   (sanity that inputs actually steer state — assert over the full set, allow
   0 collisions since inputs feed the accumulator every frame).
6. **Fork-of-fork**: from the last child (before dropping it), take a
   second-level snapshot, build a grandchild, verify restore fidelity + 10
   frames of progress + a `ReverifyRegions` echo — "readability holds after
   restore and after fork".

Also add a small negative control asserting the test would catch a no-op
regression: after Ready, read the manifest and assert `wram`'s entry has
`extent_n >= 1` and a nonzero GPA that is NOT the channel GPA, and that
`read_region` bytes change between frame 0 and frame 60 (a fake handle / empty
manifest cannot pass).

## D. Evidence discipline (request acceptance #3)

Same shape as the hypervisor's M9 acceptance
(`../determinism-hypervisor/target/m9-final-acceptance-20260621T004402Z/` —
inspect it and mirror the format):

- Artifact root: `target/m4-acceptance-<UTC timestamp>Z/` created by the test.
- Contents: `evidence.json` (git rev, host, kernel, child count, warm-up
  frame, per-child region SHA-256 table, pass/fail per assertion group,
  wall-clock), `root-regions/` (raw root baseline region dumps), the test's
  stdout log. Hash the dumps and record the hashes in `evidence.json`.
- Print the artifact root path in the test output; record it in the bead
  close reason and the handback note (see `07-…`).

## E. Runtime budget

100 × (128 MiB memcpy + 60 frames × 4,096 work units) — the memcpy dominates
(~100 × ~30 ms) plus guest execution; estimate well under 10 minutes total.
If a child hangs, `run_until`'s deadline fires — fail fast with the child
index in the message.

## Done when

- Fixture bump compiles into the initramfs; existing staged flows (m2 tests)
  still green.
- `m4_acceptance` green on this machine (`infra-control` == the
  `infra-control-kvm-intel` runner host) with N=100; artifact root recorded.
- If host provisioning blocks the run (see 00 risk #3), the plan is NOT done —
  fix the provisioning or escalate in the handoff with exact failing check.
