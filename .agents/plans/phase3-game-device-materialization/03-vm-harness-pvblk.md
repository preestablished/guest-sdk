# 03 — Read-only pv-blk device model in the VM harness

Independent of 01/02; needed by `04-…`. Today the tests/vm harness stubs
**only** the pv-pad latch: all MMIO dispatches to `pio::pvpad_read`/
`pvpad_write` (`tests/vm/src/harness/mod.rs:377-387`), which return 0 / drop
for any address outside `PVPAD_BASE = 0xD000_1000` (`pio.rs:32,197-232`).
There is no pv-blk at `0xD000_4000` — which is why the m9 workload's status
checks "pass" against a phantom device and why `boot_probe` shows the gap.

## What to build

A minimal, read-only pv-blk model faithful to dh-devices semantics
(determinism-hypervisor `crates/dh-devices/src/blk.rs` + `bus.rs`), sitting in
`tests/vm/src/harness/` (new `pvblk.rs` module next to `pio.rs`):

- 4 KiB window at `0xD000_4000`.
- Registers: `0x00` MAGIC (RO, u32 `0x0005` — the bus serves device id,
  `bus.rs:89-97`), `0x04` VERSION (RO, u32 1), `0x08` SECTOR (u64),
  `0x10` BUF_GPA (u64), `0x18` COUNT (u32), `0x1C` CMD (u32, write
  triggers), `0x20` STATUS (u32 RO).
- Backing: `Vec<u8>` supplied by the test; `capacity_sectors = len / 512`
  (truncating, like `blk.rs:114-117`).
- `CMD_READ`: validate like `blk.rs:137-147` — count 0, overflow, or
  `sector + count > capacity` ⇒ `STATUS_BAD_REQUEST` (1); copy from backing
  into guest memory at BUF_GPA via the harness's `GuestMemoryMmap`; an
  unmapped BUF_GPA range ⇒ `STATUS_MEM_FAULT` (2).
- `CMD_WRITE` / `CMD_FLUSH`: return `STATUS_BAD_REQUEST` with a comment
  stating the deliberate divergence (the real device supports writes; the
  agent must never issue them — a write reaching this stub is a bug we want
  loud). Nothing in this tier writes: the m9 workload (which does write) has
  no test here (`tests/vm/tests/` has only boot_probe/m2/m4 files).
- Unknown CMD ⇒ `STATUS_BAD_REQUEST`; STATUS is read-only; misaligned or
  non-4/8-byte accesses can just be ignored/return 0 (bus-fault fidelity is
  not needed at this tier — the agent never issues them).

## Plumbing

- Dispatch: in the run loop's MMIO arm (`mod.rs:377-387`), route addresses in
  `[0xD000_4000, 0xD000_5000)` to the pv-blk model **only when the test
  attached one**; otherwise preserve today's behavior exactly (read 0 /
  drop) — `boot_probe` and the existing m2/m4 tests must be bit-identical
  with no device attached (their goldens, e.g. m4's
  `0x3b0d3ebc93e4ba51`, must not shift).
- Attachment API: keep it in the harness style — e.g. a
  `VmHarness::attach_pv_blk(backing: Vec<u8>)` (or a field on the params
  struct `VmHarness::new` takes, whichever exists — follow `mod.rs:182-269`),
  plus an accessor for test assertions mirroring `pvpad()` (`mod.rs:434`).
- Guest-memory copy: same access path `attach_channel`/pv-pad use
  (`pio.rs:179` neighborhood) — do not add a second mapping of guest RAM.

## Tests (host-side, in tests/vm — no KVM needed for the model itself)

Unit-test the model directly (plain struct + fake/borrowed guest memory if
the harness structure allows; otherwise fold into `04-…`'s VM test):

- MAGIC/VERSION reads; MAGIC is 4-byte only.
- Read at last valid sector OK; one past ⇒ BAD_REQUEST; count 0 ⇒
  BAD_REQUEST (this is what the agent's capacity probe keys on — fidelity
  here **is** the test of the probe's assumptions).
- Multi-sector read crossing nothing special returns exact backing bytes.
- Non-512-multiple backing: trailing partial sector not addressable.
- WRITE ⇒ BAD_REQUEST.

## Done when

Model + plumbing green; **all existing VM-tier tests pass unchanged with no
device attached** (run `m2_acceptance`, `m4_acceptance`, `m4_snapshot` on
this box); `04-…` can attach a patterned image.
