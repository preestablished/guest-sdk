# Current State (Evidence-Based)

Primary source: the reference-workload readiness audit at
`../reference-workload/.agents/plans/guest-sdk-unblock-reference-workload/m4-in-vm-first-room-evidence.md`
(inspected guest-sdk rev `08abbbc`, which is still your `main` tip as of this
filing), plus our own findings from operating the deployed runtime.

## What Already Works

- **GS-5 control handoff (partial):** `detguest-agent/src/control.rs` drives
  `Hello -> LoadGame -> Ready{frame=0} -> Start`; `runtime.rs` starts the
  autostart unit, waits for expected manifest regions, emits `RegionRegister`
  then `Ready`. Host/unit tests pass
  (`cargo test -p detguest-agent -p detguest-sdk -p detguest-host --locked`).
- **Manifest + host reads:** `detguest-host/src/manifest.rs` stable manifest
  reads and `read_region` are implemented and tested; the hypervisor consumes
  them in production (we exercise that path daily through the bridge).
- **Staged fixture:** `tests/vm/workloads/src/bin/m9_refwork_contract.rs`
  publishes staged `wram`/`framebuffer`/`meta` and speaks the refwork control
  protocol. The deployed rom-bridge-o73 READY snapshot is built on the staged
  M9 fixture — its framebuffer region is a 4 KiB stub, which is exactly why
  the operator bridge still renders "No Frame Yet" (see `02-…`).

## The Three Recorded Blockers

1. **`detguest-sdk/src/regions.rs` — `register_region` (~line 133)** is a
   validate-and-return-no-op-handle path. Ms4's substance — mlock, prefault,
   pagemap GVA→GPA translation, agent IPC registration, kernel-config
   pinning assumptions (no compaction/migration/KSM/THP/swap) — is not proven
   through it. Tracked by your `guest-sdk-m4-agent-ipc-protocol` /
   `guest-sdk-m4-agent-ipc-server` beads.
2. **`detguest-agent/src/commands.rs` — `ReverifyRegions` (~line 77)** is a
   no-op. Restore/fork readability re-verification is part of what the 100×
   acceptance below has to lean on.
3. **`guest-sdk-m4-platform-readability-vm` (P0, BLOCKED):** the Intel VM
   acceptance test proving published regions are readable from the host and
   stable across **100 snapshot/restore branches** — this is the phase exit
   gate's wording for Ms4, and no fixture-level evidence substitutes for it.

## What Downstream Is Waiting

- reference-workload M4 (`refwork-d7t.10`) is BLOCKED explicitly on GS-6;
  its unblock checklist items 2–3 are your GS-5/GS-6 for the *real* workload
  path.
- The Phase 3 exit gate items 2 (your Ms4/Ms5) and 3 (first-room in-VM via
  worker gRPC, `GetFramebuffer` shows the room) cannot start until this
  lands.
- The operator bridge (browser preview, input, capture) is deployed, healthy,
  and blocked from showing a single real pixel by the absence of a READY
  snapshot whose guest publishes a conformant D7 framebuffer.
