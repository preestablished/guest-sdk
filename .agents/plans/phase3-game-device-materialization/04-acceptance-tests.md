# 04 — VM-tier acceptance: byte-exact materialization + loud negatives

Depends on 01–03. This is the request's "what green looks like" item 1:
"a test where the agent materializes a known game image via pv-blk and the
unit reads it back byte-exact (checksummed), plus a loud distinct fault when
pv-blk is absent/corrupt", with negative controls per the ecosystem
convention.

## New guest workload: `tests/vm/workloads/src/bin/game_load_check.rs`

A synthetic refwork-ctl unit (fd-3 SEQPACKET, same wire shapes as
`m9_refwork_contract.rs` `drive_control`, lines 264-294) that *actually reads
the game file*:

1. `Hello{v}` → `HelloAck{v}`.
2. `LoadGame{dev_path}`: assert `dev_path == "/run/detguest/game.img"`
   (pins the materialized-path contract the way m9 pins `/dev/vdb`);
   `std::fs::read` it; compute the shared checksum (the `pvblk` module's
   algorithm — seed `0x7062_6c6b_5f69_6f31`, rotate-left-5 fold, stream
   offsets; keep the constants in the workload in sync by comment-reference,
   the crates don't link); print one line to stdout:
   `game bytes=<len> checksum=0x<sum:016x>` (stdout becomes host-visible
   `LogLine`s via the supervise pipes — the `print-lines` precedent). On any
   mismatch/read error reply `Fault{detail}` instead of `GameLoaded` (which
   the agent turns into a loud boot fault, `control.rs:120-124`).
3. `GameLoaded` → `Ready{frame:0}` → wait `Start`, then park (block forever;
   no region registration — this test isolates the game path).

Register the bin in `tests/vm/workloads/Cargo.toml` next to
`m9-refwork-contract`.

## Boot fixture: `image/boot.toml.game-mat`

Clone of `boot.toml.m9-refwork-contract` minus the `[[expected_region]]`
blocks, with:

```toml
exec = "/opt/game-load-check"
[unit.control]
protocol = "refwork-ctl"
proto_version = 1
game_dev = "/dev/vdb"
game_source = "pv-blk"
```

## Test: `tests/vm/tests/game_materialization.rs`

Follow `m4_acceptance.rs` end to end: same env/KVM gating as m2/m4 (not
boot_probe's soft-skip), same `artifacts()` staging recipe
(`m4_acceptance.rs:93-138`): musl-build agent + workloads → stage
`sbin/detguest-agent`, `opt/game-load-check`, `etc/detguest/boot.toml` (the
new fixture) → `./image/build.sh initramfs <stage>` → per-test cpio name.

Game image: a deterministic 32 KiB pattern, **not all-zeros and not
sector-periodic** (e.g. `byte[i] = ((i * 7) ^ (i >> 8)) as u8`) so
sector-swap / truncation / phantom-zero-read bugs shift the checksum.

**Positive path** — attach the pattern via `harness.attach_pv_blk(...)`
(`03-…`), boot, run until `Ready` (drain events like m4 does):

- `Hello` → `WorkloadStarted` → a `LogLine` matching
  `game bytes=32768 checksum=0x…` where the checksum equals the host-side
  computation over the same pattern **by the same function** (import from
  `detguest-agent`'s pvblk module if the test can link it, else reimplement
  against the pinned constants) → `Ready`.
- Negative-control assertions inline (convention:
  `m4_acceptance.rs:402-414`, `m4_snapshot.rs:261-268` — comment names the
  broken implementation each catches):
  - checksum ≠ checksum of 32 KiB of zeros (catches a materializer that
    writes the right length from a phantom device);
  - `bytes=32768` exactly (catches capacity-probe off-by-one);
  - no P0 agent LogLine before Ready (stream AGENT, level 0 — filter as in
    `m4_acceptance.rs:241-254`).

**Negative 1 — device absent** (the request's "absent" fault): boot the same
image with **no** `attach_pv_blk`. Assert: no `Ready`; guest powers off; the
last agent LogLine (stream AGENT, level 0) contains `pv-blk` and the magic
mismatch wording — and does **not** contain the harness's
`cannot read game path` text. This doubles as the recorded expectation for
`boot_probe` against the rebuilt package-04 image (request
`02-verification.md`: under the device-less probe a correct implementation
faults at the pv-blk read — this is that fault, now named).

**Negative 2 — corrupt/truncated device**: attach the pattern truncated to a
non-512 multiple (e.g. 32 KiB − 100). Materialized size is then
`floor/512·512 = 32256` bytes ⇒ the workload's checksum diverges from the
full-pattern expectation ⇒ `Fault` ⇒ boot fault containing the workload's
detail; no `Ready`. Asserts both the sector-truncation semantic (00-overview
risk 2) and the fault loudness. (If `03-…` grew per-sector status injection,
also assert a mid-read `HOST_IO` surfaces as `pv-blk: read status 254 at
sector …`; otherwise the host unit tests in `01-…` cover that split.)

**Guard-reverted check** (request: "shown to fail with the guard reverted"):
once green, temporarily revert one guard locally — e.g. make the workload
skip its checksum comparison — and record in the test's module doc which
assertions caught it. Do not commit the reverted state.

## Regression sweep

- `boot_probe` still compiles and self-skips without env; existing
  m2/m4 suites green (no attached device ⇒ bit-identical harness behavior,
  `03-…`).
- Full quality gates: fmt, clippy `-D warnings`,
  `cargo test --workspace --locked`, musl agent build, then the VM tier on
  this box.

## Done when

Positive + both negatives green on this machine's KVM; evidence (test names,
checksums, fault lines) captured for `05-…`'s resolution doc.
