# Options (Recommendation: B)

## A — Guest kernel pv-blk block driver (real `/dev/vdb`)

Honest to the `boot.toml` contract, but heavy: a kernel module/driver in
the pinned deterministic kernel, config churn, re-pin, and the
determinism blast radius of block-layer code in every guest. Phase 3
does not need a general block device; the workload reads one image once
at setup.

## B — Agent materializes the game before LoadGame (recommended)

The agent already owns the pv-blk story (your M9 pv-blk IO probe) and
the pre-LoadGame moment. Shape:

1. During unit bring-up, when `[unit.control].game_dev` is configured,
   the agent reads the game image via pv-blk MMIO (the working probe
   code in `tests/vm/workloads/src/bin/m9_refwork_contract.rs` —
   command registers, checksummed readback — is the reference; promote
   it into the agent or a shared crate rather than copying).
2. Writes the bytes to a tmpfs-backed file (e.g. `/run/game.img`), and
   drives `LoadGame { dev_path: <that path> }`.
3. `boot.toml` semantics: either keep `game_dev = "/dev/vdb"` as the
   *logical* device name the agent resolves (no manifest change,
   documents the indirection in API.md §7.1), or add an explicit
   `game_source = "pv-blk"` field — your schema, your call; the
   reference-workload manifest can adapt either way (their boot.toml
   builder is one function).
4. Failure modes stay loud: pv-blk read/checksum failure should be a
   boot fault naming pv-blk, distinct from the harness's
   cannot-read-path fault.

Determinism notes: the read happens at a deterministic boot point
(single-threaded agent, before Ready), byte source is the
content-addressed game image — no new nondeterminism. Memory: game
images are small (32 KiB synthetic now; cartridge-scale later) —
tmpfs is fine at 128 MiB guest RAM, but state the size bound.

## C — Harness reads pv-blk MMIO itself

Pushes `/dev/mem` + MMIO knowledge into reference-workload, duplicates
the probe logic outside your ownership, and gives every future workload
the same problem again. Mentioned for completeness; we'd argue against.

## Cross-Repo Coordination

- If B changes `boot.toml` schema or LoadGame path semantics,
  reference-workload adapts in one place
  (`xtask/src/image.rs` boot.toml staging + `refwork-harness` loader is
  path-agnostic already). Say the shape in your resolution and we'll
  drive that side.
- The dist image embeds the agent at the rev pinned in
  `image/guest-sdk.lock` — landing this means a lock bump + image
  rebuild on the reference-workload side (their build refuses on rev
  mismatch until bumped; known, one-line).
