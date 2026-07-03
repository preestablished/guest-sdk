# 02 — `game_source` schema + runtime wiring + docs

Depends on `01-…` (`pvblk::materialize`).

## Schema decision (recap from 00-overview decision 1)

New **optional** `[unit.control]` key:

```toml
[unit.control]
protocol = "refwork-ctl"
proto_version = 1
game_dev = "/dev/vdb"        # unchanged: required for refwork-ctl; the
                             # logical game device name
game_source = "pv-blk"       # NEW, optional. Present => the agent
                             # materializes the game from the pv-blk MMIO
                             # device to /run/detguest/game.img and sends
                             # THAT path as LoadGame.dev_path. Absent =>
                             # game_dev is sent verbatim (v1 behavior).
```

Why explicit-field and not magic `/dev/vdb` interpretation: absent-field
preserves today's semantics bit-for-bit, so `image/boot.toml.m2`,
`.m4-regions`, and `.m9-refwork-contract` (whose workload asserts
`dev_path == "/dev/vdb"`, `m9_refwork_contract.rs:268`) need no change, all
existing goldens stand, and adopting the new path is a single reviewable line
in reference-workload's `image/boot.toml`. The schema is ours (API.md §7:
"this repo owns the format; the agent is its only parser").

Materialized path constant: `/run/detguest/game.img` — alongside
`agent.sock` in the agent-owned runtime dir (`detguest-wire`
`regionipc.rs:19`). Define it next to the socket path or in `pvblk.rs`;
either way one constant, no string dupes.

## Parser: `crates/detguest-agent/src/boot.rs`

- `UnitControl` (`boot.rs:29-37`) gains `pub game_source: Option<GameSource>`
  with `enum GameSource { PvBlk }` (an enum, not a String — unknown values
  must die in the parser, not deep in runtime).
- Parse in the `[unit.control]` block (`boot.rs:154-189`): accept exactly
  `"pv-blk"`; any other value ⇒ boot fault
  `unit[{i}].control: unknown game_source {v:?} (v1 knows "pv-blk")` — same
  loud-parse discipline as the existing
  `game_dev required for refwork-ctl (§7.2)` check at `boot.rs:179-183`.
- `game_dev` remains required for `refwork-ctl` (unchanged rule).

Parser tests (extend the existing in-module suite, cf. `boot.rs:655-707`):
valid `game_source = "pv-blk"` round-trips to `Some(GameSource::PvBlk)`;
absent ⇒ `None`; unknown value ⇒ error naming the field and value; existing
fixture-shaped documents still parse with `game_source: None`.

## Runtime wiring: `crates/detguest-agent/src/runtime.rs`

In `autostart_and_ready` (`runtime.rs:162-198`), in the `Some(control)` branch
**before** the socketpair + `start_unit_with_control` (so a pv-blk fault never
leaves an orphan unit — 00-overview decision 6):

```rust
let load_path: &str = match control.game_source {
    Some(GameSource::PvBlk) => {
        pvblk::materialize(GAME_IMG_PATH)
            .map_err(|e| format!("materialize game from pv-blk: {e}"))?;
        GAME_IMG_PATH
    }
    None => control.game_dev.as_deref().expect_or_fault(...), // existing §7.2 guarantee
};
```

Then pass `load_path` into the control leg. Two options for the plumbing;
take the first unless it fights the code:

- Preferred: give `control::drive_refwork_start` (`control.rs:70`) an explicit
  `game_path: &str` parameter and hoist the `game_dev` resolution (currently
  `control.rs:87-90`) into the caller. Send site `control.rs:114`
  (`encode_load_game`) is otherwise untouched.
- Alternative: pass a resolved `UnitControl` clone with `game_dev` swapped —
  rejected: it lies about what boot.toml said.

Error propagation is already right: any `Err(String)` out of
`autostart_and_ready` becomes `boot_fault` (LogLine stream AGENT level 0 +
power-off) at `runtime.rs:416-418`, per §7.3. The `pv-blk:`-prefixed messages
from `01-…` make the fault name pv-blk — distinct from the harness's
"cannot read game path" fault, as the request demands.

Note on `runtime.rs:657` (test fixture constructing
`UnitControl { game_dev: Some("/dev/vdb"...), .. }`): gains
`game_source: None`; existing control-leg tests must stay green untouched
otherwise.

## Control-leg golden

Add one golden alongside `control.rs:325-331`
(`encode_load_game("/dev/vdb") == [0x01, 0x08, b"/dev/vdb"]`):
`encode_load_game("/run/detguest/game.img")` with its length prefix — pins
the exact bytes the reference workload will see after adoption.

## Docs (same commit as the code)

`prompts/docs/guest-sdk/API.md`:

- §7.1 schema block (lines 766-804): add the `game_source` line with the
  comment shape above; fix the stale `(the virtio-blk game image device)`
  aside on `game_dev` (line 791) — there is no virtio-blk; it is the logical
  name for the pv-blk-backed game device.
- §7.2 field rules (lines 811-825): `game_source` optional, v1 value set
  `{"pv-blk"}`, unknown value ⇒ boot fault; semantics: present ⇒ the agent
  materializes the game bytes from the pv-blk MMIO device (capacity-probed,
  sector-granular — **images must be 512-aligned**) to
  `/run/detguest/game.img` before `LoadGame` and sends that path; absent ⇒
  `game_dev` sent verbatim.
- §7.3: add pv-blk materialization failure (absent device, read status,
  checksum drift, size cap) to the enumerated boot-fault causes.

`prompts/docs/guest-sdk/ARCHITECTURE.md` §4.2 (control leg, line ~305):
insert the materialization step between unit-config resolution and the
`LoadGame` send, with the determinism note (pre-Ready, single-threaded, pure
MMIO, no retry — cf. §7 rules at lines 525-566 and the no-retry rule at
line 359).

## Done when

Host tier green (`cargo test -p detguest-agent`, fmt, clippy, musl build);
parser + golden tests as above; docs updated in the same commit; the m2/m4/m9
boot fixtures parse unchanged.
