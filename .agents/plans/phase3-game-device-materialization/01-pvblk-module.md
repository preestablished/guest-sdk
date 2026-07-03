# 01 — `pvblk.rs`: the agent's pv-blk client

New module `crates/detguest-agent/src/pvblk.rs` (+ `mod pvblk;` in `lib.rs`).
This is the "promote it into the agent" of the request's Option B step 1. The
reference implementation is `tests/vm/workloads/src/bin/m9_refwork_contract.rs`
(constants lines 33-43, `PvBlkClient` lines 118-206, `gva_to_gpa` lines
242-254) — **do not copy its panics**: every failure returns `Err(String)` so
`runtime.rs` can route it through `boot_fault` (a panic in PID 1 is exit 101,
not a §7.3 loud fault).

## Device contract (source of truth: determinism-hypervisor `crates/dh-devices/src/blk.rs` + `bus.rs`)

```rust
pub(crate) const PV_BLK_BASE: u64 = 0xD000_4000; // 4 KiB MMIO window GPA
const REG_MAGIC: usize = 0x00;   // u32 RO, served by the bus: device id
const REG_SECTOR: usize = 0x08;  // u64 RW
const REG_BUF_GPA: usize = 0x10; // u64 RW (guest-physical DMA target)
const REG_COUNT: usize = 0x18;   // u32 RW (sectors)
const REG_CMD: usize = 0x1C;     // u32 WO: write triggers synchronously
const REG_STATUS: usize = 0x20;  // u32 RO
const CMD_READ: u32 = 1;         // never issue CMD_WRITE (2) / CMD_FLUSH (3):
                                 // writes dirty the hypervisor overlay
const DEVICE_ID_PV_BLK: u32 = 0x0005;
const STATUS_OK: u32 = 0;
const STATUS_BAD_REQUEST: u32 = 1; // out-of-range sector/count or bad CMD
const STATUS_MEM_FAULT: u32 = 2;   // BUF_GPA not mapped guest RAM
const STATUS_HOST_IO: u32 = 0xFE;  // host-side base image read failure
pub(crate) const SECTOR_SIZE: usize = 512;
const SECTORS_PER_PAGE: u32 = 8;   // 4096 / 512
pub(crate) const MAX_GAME_BYTES: u64 = 64 << 20; // loud fault above this
```

Access discipline (bus §6.1): 4- or 8-byte naturally aligned
`read_volatile`/`write_volatile` only. MAGIC must be read as a **4-byte**
access (an 8-byte read at 0x00 is a bus guest-fault, `bus.rs:90-95`).

Partial-completion contract (`blk.rs:42-49`): on nonzero STATUS the buffer is
undefined — treat any nonzero status as fatal for that request; never retry
(ARCHITECTURE.md:359 — no in-guest retry, failures must be loud and
reproducible).

## Structure: injectable registers, testable core

```rust
/// Register-level access to one pv-blk window. Two impls: the real
/// /dev/mem mapping, and a test fake backed by a Vec<u8> image.
pub(crate) trait PvBlkRegs {
    fn read_u32(&mut self, off: usize) -> u32;
    fn write_u32(&mut self, off: usize, v: u32);
    fn write_u64(&mut self, off: usize, v: u64);
    /// CMD_READ `count` sectors at `sector` into the DMA page; on OK the
    /// impl exposes the page contents via `dma_page()`.
    // (shape it however reads cleanest — the point is that probe/read/
    //  checksum logic below never touches /dev/mem directly)
}
```

Split the module into:

- **`MappedPvBlk`** — the real impl. Opens `/dev/mem` with `O_SYNC`, mmaps 4 KiB
  at `PV_BLK_BASE` (`MAP_SHARED`, `PROT_READ|PROT_WRITE`) — same shape as the
  probe's `PvBlkClient::new` (m9 lines 124-161) and the SDK's `map_pv_pad`
  (`crates/detguest-sdk/src/pio.rs:100-125`). Errors → `Err`, not panic.
  `CONFIG_DEVMEM=y` / STRICT_DEVMEM off are already pinned
  (`image/kernel.config:55-61`).
- **DMA page** — one `#[repr(align(4096))] static` 4096-byte buffer (mirrors the
  probe's `DISK_BUF`, m9 lines 53-60): zero it, `mlock` it, touch it, then
  translate GVA→GPA **once** with the agent's own
  `translate::open_pagemap()` + `translate::gva_to_gpa` (`translate.rs:48,75` —
  the exact code the agent already runs for its channel GPA,
  `runtime.rs:136-138`). One page is always GPA-contiguous, which is why the
  read loop never asks for more than `SECTORS_PER_PAGE` per command.
  A `mlock` failure is a fault (the agent is root/PID 1; children get
  `RLIM_INFINITY` memlock at `supervise.rs:172-177` — the agent itself needs no
  raise, but check the return anyway).
- **Pure logic** (generic over `PvBlkRegs`): presence check, capacity probe,
  `materialize()` read loop, checksum. All host-unit-testable.

## Presence check

Read u32 at `REG_MAGIC`; require `DEVICE_ID_PV_BLK`. Mismatch ⇒
`Err("pv-blk: no device at GPA 0xd0004000 (magic 0x{got:x}, want 0x5)")`.
This is the fault the device-less `boot_probe` harness will show (its
non-pv-pad MMIO reads return 0 — `tests/vm/src/harness/pio.rs:197-232`), giving
the bridge the layer-visible failure their `02-verification.md` expects.

## Capacity probe (no capacity register exists — see 00-overview decision 3)

`fn probe_capacity(regs) -> Result<u64, String>` returns capacity in sectors:

1. `read_ok(sector) := issue CMD_READ{sector, count: 1, buf: dma_gpa}` and
   inspect STATUS: `OK` ⇒ true; `BAD_REQUEST` ⇒ false; anything else ⇒
   hard `Err` naming the status and sector (BAD_REQUEST is the *only* status
   that encodes "past the end", `blk.rs:137-147`; MEM_FAULT/HOST_IO during a
   probe are real faults, not size signals).
2. `read_ok(0)` false ⇒ `Err("pv-blk: game device is empty (0 sectors)")`.
3. Doubling: find smallest `k` with `read_ok(2^k - 1)` false (bail with a
   loud fault past `MAX_GAME_BYTES / 512` — don't probe forever).
4. Binary search in `(2^(k-1) - 1, 2^k - 1)` for the largest readable sector
   `s`; capacity = `s + 1`.

Deterministic: capacity is fixed for the run, so the exact command sequence —
and therefore the READY icount — is a pure function of the image size.
~2·log₂(capacity) single-sector commands; trivial even at cartridge scale.

Enforce `capacity * 512 <= MAX_GAME_BYTES` ⇒ else
`Err("pv-blk: game image {n} bytes exceeds {MAX_GAME_BYTES} cap")`.

## Materialize

`pub(crate) fn materialize(dest: &str) -> Result<(), String>` (the real-impl
entry point `runtime.rs` calls; internally generic over `PvBlkRegs`):

1. Map device, set up DMA page, presence check, probe capacity.
2. `create_dir_all("/run/detguest")` (already exists by this point —
   `region_ipc.rs:97` bound agent.sock at `runtime.rs:414` — but don't depend
   on ordering), create `dest` (truncate).
3. Loop: `CMD_READ min(SECTORS_PER_PAGE, remaining)` sectors into the DMA
   page; nonzero status ⇒
   `Err("pv-blk: read status {s} at sector {sector} (count {c})")`;
   `write_all` the bytes to the file; fold them into a streaming checksum.
4. Second pass: repeat the same reads recomputing the checksum (do **not**
   re-read the file); mismatch ⇒ `Err("pv-blk: readback checksum drift
   (0x{a:x} != 0x{b:x})")`. This is the fixture's checksummed-readback
   precedent (m9 `write_read_once`, lines 163-172) as a non-panicking
   tripwire; images are small and the cost is deterministic.
5. Return; the file is RAM-backed (initramfs rootfs), no sync needed.

Checksum: reuse the fixture's algorithm so numbers are comparable across
tiers — seed `0x7062_6c6b_5f69_6f31`, per byte
`sum = sum.rotate_left(5) ^ (byte as u64).wrapping_add(i)` with `i` the
**stream** offset (m9 `checksum_disk_buffer`, lines 208-240, generalized from
per-buffer to whole-stream). Expose it `pub(crate)` — the VM test asserts the
same function host-side (`04-…`).

No writes to the device, ever (00-overview decision 5).

## Unit tests (host tier, in-module)

Fake `PvBlkRegs` backed by `Vec<u8>` implementing real semantics:
`BAD_REQUEST` past capacity or on count 0, data served from the vec,
injectable per-sector status overrides.

- Capacity probe exact at boundaries: 1, 7, 8, 9, 63, 64, 65, 4096 sectors
  (powers of two and neighbors — off-by-one country).
- Empty device ⇒ the "0 sectors" error.
- Probe hitting `MAX_GAME_BYTES` bound ⇒ loud cap error, terminates.
- Presence check: wrong magic ⇒ error text names pv-blk + both magics.
- Mid-read `MEM_FAULT`/`HOST_IO` (injected at sector N) ⇒ error names status
  and sector; file logic not consulted after failure.
- Full materialize over a patterned 32 KiB image ⇒ file bytes == source
  bytes (byte-exact), checksum matches an independently computed value.
- Second-pass drift (fake returns different bytes on pass 2) ⇒ the
  checksum-drift error. **Negative control per the ecosystem convention**
  (cf. `tests/vm/tests/m4_snapshot.rs:261-268`): comment names the broken
  implementation this catches (a materializer that skips verification).
- Non-page-multiple image (e.g. 9 sectors) exercises the short final read.

`/dev/mem` mapping and pagemap translation are deliberately not unit-tested
(no fake would prove anything); the VM tier (`04-…`) covers them.

## Done when

`cargo test -p detguest-agent` green with the above; `clippy -D warnings`
clean; no `unwrap`/`assert!` on device-reachable paths; musl build green.
