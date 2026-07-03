# Resolution: The Agent Materializes The Game From pv-blk

guest-sdk session, 2026-07-03. Option B as you recommended, landed and
verified. Plan trail: `.agents/plans/phase3-game-device-materialization/`
(reviewed by two independent passes before implementation; the plan files
record what changed and why).

## Commits (this repo, on `main`)

| SHA | What |
|---|---|
| `6b7fd33` | `crates/detguest-agent/src/pvblk.rs` — the pv-blk client (presence check, size discovery, materialize, checksum verify) |
| `d20db85` | `game_source = "pv-blk"` schema + LoadGame wiring + API.md §7 / ARCHITECTURE.md §4.1–4.2 |
| `91f86ad` | tests/vm harness: read-only pv-blk device model at `0xD000_4000` |
| `a33e370` | VM acceptance: byte-exact positive + device-absent + truncated negatives |

**Bump `image/guest-sdk.lock` to `a33e3704d14acd19da04402205b03fcabee9fdfa`**
(or later).

## The boot.toml / LoadGame semantics we chose

New **optional** key in `[unit.control]` (API.md §7.1/§7.2; schema is ours):

```toml
[unit.control]
protocol = "refwork-ctl"
proto_version = 1
game_dev = "/dev/vdb"        # unchanged; still required for refwork-ctl —
                             #   now documented as the LOGICAL device name
game_source = "pv-blk"       # NEW. Present => before LoadGame the agent
                             #   reads the whole image out of the pv-blk
                             #   MMIO device into /run/detguest/game.img
                             #   and sends THAT path as LoadGame.dev_path.
                             #   Absent => game_dev sent verbatim (bit-for-
                             #   bit the old behavior; every existing
                             #   fixture/golden unchanged).
```

Mechanics (ARCHITECTURE.md §4.2 step 0, normative):

- Materialization happens **before the unit is spawned** — a pv-blk fault
  never leaves an orphan workload; it is a plain §7.3 boot fault.
- **Size discovery**: the device ABI has no capacity register, so the agent
  reads forward from sector 0 in 4 KiB chunks and treats the first
  `STATUS_BAD_REQUEST` (the only past-the-end signal) as the tail, narrowed
  exactly with ≤ 3 shrinking reads. Deterministic: the command sequence is a
  pure function of the image. (Future nicety for the hypervisor, not needed
  now: a read-only capacity register would remove the tail probe.)
- The written file is **verified**: re-read and checksummed against the
  device stream (rotate-left-5 fold, seed `0x7062_6c6b_5f69_6f31` — your
  fixture's readback-checksum precedent, non-panicking).
- **Reads only** — the agent never issues CMD_WRITE/CMD_FLUSH, so the pv-blk
  overlay stays clean: no new dirty-cluster load in snapshots. After the last
  read, SECTOR/BUF_GPA/COUNT/STATUS retain that command's values at READY —
  deterministic device snapshot state, fine but should be known.
- Size cap **32 MiB** (loud fault above): the game peaks at two RAM copies —
  the /run file plus `Cartridge.rom` — against 128 MiB guest RAM. The agent
  **unlinks `/run/detguest/game.img` after the control leg** (your harness
  holds its own copy by `GameLoaded`), so steady state holds one copy.
- Failure taxonomy, all `pv-blk:`-prefixed agent LogLines (level 0), distinct
  from your harness's cannot-read-path fault: device absent (bus MAGIC
  mismatch), read status error (names status + sector), materialized-file
  checksum drift, size cap, empty device.
- **READY icount**: shifts (added deterministic pre-Ready work) and is now a
  pure function of WorkloadImage **plus the content-addressed game image**
  (both pinned inputs; ARCHITECTURE.md §4.1 updated). Step-3 READY-snapshot
  regeneration absorbs the shift.

## ⚠ Staging requirement — validate at the source, the guest cannot

The device addresses whole 512-byte sectors (`capacity = len_bytes / 512`,
truncating). A non-512-multiple staged image **silently loses its tail, and
the agent cannot detect that** — `BAD_REQUEST` is the only past-the-end
signal, so the lost bytes are invisible to any guest-side probe. Please
validate the staged `DH_M9_GAME_IMAGE` before boot: `size % 512 == 0`,
ideally your full cart rule (power of two ≥ 32 KiB — `Cartridge::from_rom`
does enforce it in-guest, but that fault reads as a cart-parser error, not a
staging error, and will cost debugging time). Positive fact: capacity
truncates and never pads, so for a valid ROM the materialized bytes are
byte-identical to the staged file — your `blake3` cart hash in `meta`/READY
evidence is unperturbed.

## reference-workload instructions (your side; say the word and we'll re-verify)

1. `image/boot.toml`: add `game_source = "pv-blk"` to `[unit.control]` (one
   line — we build-tested exactly this locally, see Evidence).
2. `xtask` `validate_boot_toml` (`xtask/src/image.rs:1619`): make it
   **require** `game_source = "pv-blk"`, exact-match style like the
   neighboring checks. Merely admitting the key would let a future edit drop
   the line and silently reintroduce the cannot-read-`/dev/vdb` boot fault
   this request exists to fix, with no build-time catch.
3. `image/guest-sdk.lock`: bump `rev` to `a33e370…` (build refuses on
   mismatch until bumped — known, one line) and rebuild the image.
4. `refwork-harness` loader: **no change** (path-agnostic; it will receive
   `/run/detguest/game.img`).
5. Stale `/dev/vdb`-era names to rename at leisure (logical/stale now, not
   load-bearing): `harness.toml` `game_image_device = "/dev/vdb"`
   (`image.rs:1690`) and the dist-manifest device block
   `{ kind: virtio-blk, role: game-image, … }` (`image.rs:1051,1210`) — both
   describe a pv-blk MMIO device by a virtio name.
6. The m9 staged fixture (`boot.toml.m9-refwork-contract` + its
   `dev_path == "/dev/vdb"` assert) is intentionally untouched: no
   `game_source` ⇒ verbatim pass-through.

## VM-tier evidence (all on infra-control KVM, `DETGUEST_VM_TESTS=1`)

`tests/vm/tests/game_materialization.rs` — 3/3 green in ~8 s (plus the m2/m4
suites unchanged and green, m2 golden event hash `0x3b0d3ebc93e4ba51`
unshifted):

1. `materialized_game_reaches_ready_byte_exact` — 32 KiB pattern on the
   harness's new pv-blk model; boot reaches `Ready` through the production
   shape (materialize → control leg → region gate); the workload's `meta`
   region carries the byte-exact checksum `0x59ac17a52dffda9c` (a golden
   pinned identically in the agent's host unit tests) read through the real
   host manifest path; stdout LogLine
   `game bytes=32768 checksum=0x59ac17a52dffda9c`.
2. `absent_device_is_a_loud_pv_blk_fault_not_a_path_fault` — no device: the
   agent faults `pv-blk: no device at GPA 0xd0004000 (magic 0x0, want 0x5)`
   before the unit spawns; asserted NOT to contain your old
   `cannot read game path` text; no `Ready`.
3. `truncated_backing_faults_loudly_before_ready` — backing truncated to
   32 668 bytes ⇒ materialized 32 256 (63 sectors); the workload's embedded
   expectation catches it; boot fault carries
   `refwork fault after LoadGame: game image is 32256 bytes, want 32768`.

Negative tests shown to fail with the guard reverted, per the convention:
with the workload's self-check disabled, test 3 fails (guest reaches
`Ready`); the agent-side verify-pass guard is covered by the unit tier's
drift test (`pvblk::tests::verify_pass_catches_a_corrupted_file`).

**Observed (not predicted) probe evidence**: we locally (uncommitted, then
reverted) bumped your lock to `a33e370`, added the `game_source` line, ran
`xtask image build` (your validator accepted the new key), and ran your
two-minute probe recipe. The probe's last event is now:

```text
GuestEvent Hello { proto_version: 1, agent_version: 256, capabilities: 3 }
GuestEvent LogLine (AGENT, level 0):
  "materialize game from pv-blk: pv-blk: no device at GPA 0xd0004000
   (magic 0x0, want 0x5)"
→ power off
```

— exactly the layer-visible fault your `02-verification.md` caveat predicts
for the device-less probe harness (it stubs only pv-pad; reads return 0).
The old `refwork fault after LoadGame: cannot read game path `/dev/vdb`` is
gone. Under the real worker (pv-blk present, game staged), the same boot
proceeds through materialization to `LoadGame` → regions → `Ready`.

## Handback

Over to you for step-2: adopt per the instructions above, run
`dh-m9-ready-handoff` with `DH_M9_GAME_IMAGE` staged (512-aligned!), and
record the READY icount / region manifest / state hash. We'll answer
`04-verification.md` questions as they come.
