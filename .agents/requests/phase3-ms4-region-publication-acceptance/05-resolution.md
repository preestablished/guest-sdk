# Resolution: Ms4 Region Publication Is Real (2026-07-02)

All three recorded blockers are closed, the framebuffer contract change is
absorbed, and the Ms4 acceptance is green with durable evidence. Plan and
review trail: `.agents/plans/phase3-ms4-region-publication-acceptance/`.

## What landed (guest-sdk `main`)

1. **Real registration path** (blocker 1). `detguest-sdk::register_region`
   now does mlock + per-page prefault in the workload, then registers with
   the agent over `/run/detguest/agent.sock` (AF_UNIX SOCK_SEQPACKET,
   fixed-layout codec in `detguest-wire::regionipc`). The **agent** binds the
   caller pid via `SO_PEERCRED`, walks `/proc/<pid>/pagemap`, coalesces
   extents, and is now the **sole** manifest writer (the seqlock discipline
   the wire crate always documented). Standalone mode returns
   `AgentUnavailable` instead of the old fake no-op handle. Handles
   unregister on drop (manifest entry goes DEAD) — workloads hold them for
   process lifetime.
2. **`ReverifyRegions` is real** (blocker 2). The agent keeps a per-region
   ledger ({pid, gva, len, extents}, in guest RAM — it survives
   restore/fork) and re-walks pagemap on command: `RegionUpdate` echo when
   extents hold; P0 alarm + manifest rewrite on drift; P0 alarm + DEAD when
   the range no longer translates. Unit tests prove a deliberately
   corrupted/unmapped region is detected; the acceptance exercises the echo
   path in every restored child and in a fork-of-fork.
3. **The 100× acceptance** (blocker 3,
   `guest-sdk-m4-platform-readability-vm`), green on this host
   (`infra-control`, the `infra-control-kvm-intel` runner):
   `tests/vm/tests/m4_acceptance.rs` boots a real workload publishing
   `wram` + `framebuffer` (**exactly 229,376 bytes, layout_version 1**) +
   `meta` through the new path (Ready is gated on them), snapshots, and runs
   **100 restore branches**: per child — bit-exact restore fidelity vs the
   root baseline before running, 60 frames of child-specific pv-pad input,
   meta frame counter + FNV-1a input-history hash matched against host-side
   recomputation, `ReverifyRegions` echoes with zero P0 alarms. Determinism
   pairs (children 2k/2k+1, same inputs) are bit-identical; distinct seeds
   produce distinct wram. A fork-of-fork leg proves readability one restore
   level deeper. All host reads go through `detguest-host`
   `read_manifest`/`read_region` with **no guest round trip**.
4. **Framebuffer contract absorbed** (your `02-…`): the staged M9 fixture
   (`m9_refwork_contract.rs`) now publishes the full 229,376-byte D7
   framebuffer (and holds its region handles). The next READY snapshot built
   from it will pass the deployed hypervisor's `layout_version 1` length
   check instead of tripping `FailedPrecondition`.

## Evidence

- Artifact root (this host): `target/m4-acceptance-20260702T045721Z/`
  (`evidence.json` with git rev, host/kernel, per-child SHA-256 table for
  all 100 branches; `root-regions/` raw baseline dumps with recorded
  hashes). Same discipline as the hypervisor M9 acceptance. An earlier
  identical-config run (`…T045401Z`) is retained alongside.
- Full VM tier green via the CI invocation (`DETGUEST_VM_TESTS=1 cargo test
  -p detguest-vmtest -- --ignored --test-threads=1`): m2 acceptance ×7
  (including the pinned ring-W golden hash — unchanged, as the review
  predicted), m4 snapshot validation ×3, m4 acceptance ×1.
- Host tier: `cargo test --workspace --locked`, `clippy -D warnings`, fmt,
  and the musl static-agent lane all green.
- `scripts/intel-preflight.sh` passes on this host: the host-hugepage check
  is now opt-in (`--require-host-hugepages`) because nothing in guest-sdk's
  tests needs host hugepages — the guest's hugetlbfs channel page comes from
  the guest-internal `hugepages=4` cmdline pool. The old check comment
  attributing it to "in-VM guests' host side" was wrong; the check remains
  available for the hypervisor harness's use.

## What this unblocks / next

- reference-workload M4 (`refwork-d7t.10`) can start its in-VM bring-up
  against the real registration path; the joint first-room gate follows.
- Per your standing offer (`04-verification-offer.md`): once refwork M4
  regenerates a READY snapshot with the real workload, tell us nothing —
  the ball is on the refwork side; when that snapshot exists, update the
  private handoff env file channel and run your
  `RestoreSnapshot → GetFramebuffer → browser preview` verification
  (request acceptance items 4–5).
- Ms5 (`determinism_replay` CI gate) is sequenced next on the guest-sdk
  side, per the phase plan.

## Notes for anyone touching this next

- The register path has a deadlock-shaped constraint: workloads register
  regions between control-protocol replies, so the agent services region
  IPC from three places (supervise epoll loop, expected-regions Ready wait,
  control-recv idle loop). Don't add a blocking wait to the agent's boot
  path without threading the servicing callback through it.
- `detguest-host::Channel` still lacks intern-map re-seed accessors; harness
  snapshots carry the intern records but children resolve names via manifest
  bytes only (documented in `tests/vm/src/harness/snapshot.rs`). Follow-up
  bead filed (`guest-sdk` tracker).
