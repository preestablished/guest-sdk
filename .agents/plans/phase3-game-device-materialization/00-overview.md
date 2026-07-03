# Plan: Game Device Materialization (Phase 3 — the last gap before READY)

Answers `.agents/requests/phase3-game-device-materialization/` (filed by
rom-operator-bridge, 2026-07-03). Read that directory first; this plan does not
repeat its context.

## Goal (behavioral)

When `[unit.control]` opts in, the agent — before sending `LoadGame` — reads
the game image out of the pv-blk MMIO device, writes it to
`/run/detguest/game.img` (RAM-backed initramfs rootfs), and sends **that** path
as `LoadGame.dev_path`. The reference workload's harness then does an ordinary
filesystem read and boots to `Ready`. Failures anywhere in the pv-blk path are
loud, pv-blk-named boot faults (API.md §7.3), distinct from the harness's
cannot-read-path fault.

This is the request's **Option B**, as recommended. Options A (guest kernel
block driver) and C (harness reads MMIO itself) are rejected for the reasons in
the request's `01-options.md`.

## Decisions made by this plan (the load-bearing ones)

Each is argued in the package that implements it; summarized here so the
implementer sees the whole shape first.

1. **Opt-in via a new optional `game_source = "pv-blk"` field in
   `[unit.control]`** (not magic interpretation of the `/dev/vdb` string).
   Absent ⇒ today's behavior exactly (pass `game_dev` verbatim), so the staged
   M9 fixture — whose workload hard-asserts `dev_path == "/dev/vdb"` at
   `tests/vm/workloads/src/bin/m9_refwork_contract.rs:268` — keeps working
   untouched, and the migration is one added line in reference-workload's
   `image/boot.toml`. `game_dev` stays required for `refwork-ctl` (§7.2
   unchanged) and documents the logical device. See `02-…`.

2. **The pv-blk client is promoted into the agent as
   `crates/detguest-agent/src/pvblk.rs`** (not a new shared crate). The agent
   already owns the two ingredients — `/dev/mem`-style GVA→GPA translation
   (`translate.rs`) and MMIO/PIO discipline (`pio.rs`) — and no other crate
   needs to *drive* pv-blk after this lands (the new test workload reads a
   plain file; the m9 fixture keeps its own probe copy as a synthetic
   stand-in). Register logic is written against an injectable register-access
   trait so the probe/read/verify logic is host-unit-testable, same pattern as
   the injectable translator in `region_ipc`/`translate`. See `01-…`.

3. **Size discovery: capacity probing within the existing device ABI.** The
   pv-blk guest ABI (determinism-hypervisor `crates/dh-devices/src/blk.rs`)
   exposes **no capacity register** — registers are SECTOR/BUF_GPA/COUNT/CMD/
   STATUS only, and out-of-range reads fail with `STATUS_BAD_REQUEST`
   (`blk.rs:137-147`). Rather than asking the hypervisor for an ABI addition
   (re-pin, snapshot-section churn, mid-phase), the agent finds the capacity
   deterministically with a doubling-then-binary-search of 1-sector reads
   (~2·log₂(capacity) commands; capacity is fixed per run so the sequence is
   bit-reproducible). Consequence to state loudly in the resolution: the device
   only addresses whole sectors (`capacity = len_bytes / 512`, `blk.rs:114-117`),
   so **staged game images must be a multiple of 512 bytes** or the tail is
   silently unaddressable. See `01-…` §Capacity and `05-…`.

4. **Device presence check via the bus MAGIC register.** The MMIO bus serves
   `0x00 MAGIC` = device id (u32 read; pv-blk is `DEVICE_ID_PV_BLK = 0x0005`,
   dh-devices `bus.rs:89-97`, `blk.rs:26`). The agent reads it first; mismatch
   ⇒ loud "pv-blk device not present" fault. Under the device-less probe
   harness (reads return 0) this is exactly the layer-visible fault the
   request's `02-verification.md` predicts.

5. **Reads only — the agent never writes the game device.** Materialization
   must not dirty the pv-blk overlay (dirty clusters enlarge hypervisor
   snapshot sections). No CMD_WRITE, no CMD_FLUSH. Integrity is a streaming
   checksum over the read plus a second full read pass compared against the
   first (the fixture's readback-checksum precedent, made non-panicking).

6. **Materialize before the unit is spawned** (in `autostart_and_ready`,
   `crates/detguest-agent/src/runtime.rs:162-198`, before the socketpair/
   `start_unit_with_control` branch) so a pv-blk fault never leaves an orphan
   workload running, and the §7.3 fault path stays simple.

## Determinism envelope (why this is safe)

ARCHITECTURE.md §7 (`prompts/docs/guest-sdk/ARCHITECTURE.md:525-566`) binds
the agent. The materialization step is: single-threaded, pre-Ready, pure
guest↔device MMIO (same category as the pv-pad latch), no entropy, no clocks,
no host-injected input before READY (§4.1), and every failure branch is a
deterministic function of guest+device state, taken once with no retry
(ARCHITECTURE.md:359). The READY icount changes (more pre-Ready work) — that is
expected and absorbed by the bridge's step-3 READY-snapshot regeneration; it
remains a pure function of the WorkloadImage + the content-addressed game
image, both pinned inputs.

## Work packages (files in this plan)

| File | Package | Depends on |
|---|---|---|
| `01-pvblk-module.md` | `pvblk.rs` in the agent: register trait, presence check, capacity probe, DMA read loop, checksum, error taxonomy, host unit tests | — |
| `02-boot-toml-and-wiring.md` | `game_source` parsing (`boot.rs`), runtime wiring before `drive_refwork_start`, fault wording, API.md §7 + ARCHITECTURE.md updates | 01 |
| `03-vm-harness-pvblk.md` | Read-only pv-blk device model in `tests/vm/src/harness/` (MAGIC/VERSION + real status semantics), opt-in per test | — (parallel with 01/02) |
| `04-acceptance-tests.md` | New guest workload + boot.toml fixture + `game_materialization.rs` VM test: byte-exact positive path, device-absent and corrupt negative controls | 01–03 |
| `05-resolution-and-handoff.md` | `03-resolution.md` in the request dir, reference-workload lock-bump/boot.toml instructions, bridge coordination, beads + session close | 01–04 |

Suggested order: 01 → 02 (host `cargo test` green after each), 03 in
parallel, then 04, then 05. Everything lands in this repo only; the
reference-workload side is instructions in the handback, not edits by us.

## External contracts that must NOT change

- The refwork-ctl wire encoding (`control.rs` `encode_hello`/`encode_load_game`
  framing and goldens — a new *path string* is fine; the framing is not).
- Existing boot.toml fixtures' behavior: `image/boot.toml.m2`,
  `.m4-regions`, `.m9-refwork-contract` parse and run exactly as today
  (no `game_source` ⇒ verbatim pass-through).
- The pv-blk device ABI (we consume it; we do not change dh-devices).
- Manifest layout, ring event codecs, `Ready` semantics, agent.sock protocol.

## Acceptance (verified before handback)

1. Host tier: `cargo fmt --check`, `clippy -D warnings`,
   `cargo test --workspace --locked` green; musl agent build green
   (`cargo build --release --target x86_64-unknown-linux-musl -p detguest-agent`).
2. VM tier: `game_materialization` acceptance green on this box (KVM
   available); byte-exact readback via checksum; both negative controls
   observed (pv-blk-named fault, no `Ready`).
3. Probe expectation documented: under `boot_probe` with the rebuilt
   package-04 image the last event becomes the agent's pv-blk-named fault
   (device-less harness) — layer-visible progress per the request.
4. `03-resolution.md` filed with commits, semantics, lock-bump instructions.

## Top risks and mitigations

1. **Unmapped-MMIO behavior differs across harnesses.** Under our tests/vm
   harness, non-pv-pad MMIO reads return 0 (`tests/vm/src/harness/pio.rs:197-232`)
   — the MAGIC check turns that into a clean fault. Under the real hypervisor
   a missing device is `BusError::Unmapped` → injected guest fault, but in
   production the device is always present; do not try to make the agent
   survive that path. Documented in `02-…`.
2. **Sector-granular truncation** of non-512-aligned images — a staging
   requirement on the bridge/reference-workload side, stated in the
   resolution (`05-…`); the 32 KiB synthetic image is aligned.
3. **DMA buffer physical contiguity**: multi-sector DMA needs GPA-contiguous
   memory; the plan reads at most one 4 KiB page (8 sectors) per command into
   a page-aligned, mlocked static — one page is always GPA-contiguous.
   (`01-…` §DMA.)
4. **`game_source` under the hypervisor's staged m9 image** — untouched by
   design (decision 1); only reference-workload's package-04 boot.toml adopts
   it.
5. **RAM budget**: game bytes exist twice transiently (file + workload's own
   copy); cap materialization at `MAX_GAME_BYTES = 64 MiB` (loud fault above)
   against 128 MiB guest RAM; today's images are 32 KiB.

## Non-goals

- Building a real `/dev/vdb` (Option A) or moving MMIO into workloads
  (Option C).
- Any determinism-hypervisor change (incl. a capacity register — note it as a
  possible future ABI nicety in the resolution, nothing more).
- Running the real-worker end-to-end handoff ourselves (bridge does it on our
  word, per the request's `02-verification.md`).
- reference-workload edits (that repo adapts per our resolution).
