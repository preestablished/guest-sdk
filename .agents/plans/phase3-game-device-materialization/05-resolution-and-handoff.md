# 05 — Resolution handback, cross-repo instructions, session close

Depends on 01–04 all green.

## `03-resolution.md` (in `.agents/requests/phase3-game-device-materialization/`)

Per the series convention (request `02-verification.md` §Handback). Contents:

1. **Commits** — list SHAs (this repo only).
2. **Semantics chosen** (the request explicitly asks for "the
   boot.toml/LoadGame semantics you chose"):
   - New optional `[unit.control] game_source = "pv-blk"`; absent ⇒
     `game_dev` sent verbatim (unchanged v1); present ⇒ agent materializes
     from pv-blk MMIO to `/run/detguest/game.img` pre-LoadGame and sends that
     path. API.md §7.1/§7.2/§7.3 updated.
   - Capacity discovery: BAD_REQUEST probing within the existing device ABI
     (no hypervisor change). Note for the future: a read-only capacity
     register in dh-devices would let the agent drop the probe — a nicety,
     not needed now.
   - **Staging requirement**: game images must be a multiple of 512 bytes
     (the device addresses whole sectors only; a partial tail is
     unaddressable and gets truncated — our Negative 2 demonstrates it).
     The 32 KiB synthetic image is fine.
   - Size cap: 64 MiB, loud fault above.
   - The agent never writes the device — pv-blk overlay stays clean, so no
     new dirty-cluster load in hypervisor snapshots.
   - READY icount will shift (added deterministic pre-Ready work) — expected;
     step-3 READY-snapshot regeneration absorbs it.
3. **reference-workload instructions** (they drive that side, request
   `01-options.md` §Cross-Repo):
   - Add `game_source = "pv-blk"` to `image/boot.toml` `[unit.control]`
     (one line; their `xtask` `validate_boot_toml` may need to admit the
     key).
   - `refwork-harness` loader: no change (path-agnostic; it will receive
     `/run/detguest/game.img`).
   - Bump `image/guest-sdk.lock` `rev` to our landed SHA and rebuild the
     image (their build refuses on rev mismatch until bumped — known,
     one-line).
   - m9 staged fixture and `boot.toml.m9-refwork-contract`: intentionally
     untouched (no `game_source` ⇒ verbatim `/dev/vdb`, and the fixture's
     `dev_path == "/dev/vdb"` assert still holds).
4. **VM-tier evidence**: test names, the byte-exact checksum value, both
   negative-control fault lines, and the recorded `boot_probe` expectation —
   against the rebuilt package-04 image the probe's last event becomes the
   agent's `pv-blk: no device …` fault (the probe harness has no pv-blk;
   layer-visible success per their caveat).
5. Invitation to re-verify (their `04-verification.md`) + run the real-worker
   step-2 handoff.

## Beads

Per CLAUDE.md this repo tracks work in bd. Before implementing, create one
bead per plan package (title <100 chars, details in `-d`, `-t feature` for
01–04 / `-t task` for 05, `-p 1`), chained with `bd dep add <next> <prev>` per
the dependency table in `00-overview.md`; claim each with
`bd update <id> --claim` when starting, close on completion. Suggested titles:

- `Agent pv-blk client module (presence, capacity probe, materialize, checksum)`
- `boot.toml game_source=pv-blk + LoadGame path wiring + API.md §7 docs`
- `tests/vm harness: read-only pv-blk device model`
- `VM acceptance: game materialization byte-exact + absent/corrupt negatives`
- `Resolution handback + reference-workload lock-bump instructions`

Record durable gotchas discovered along the way with `bd remember` (e.g. the
no-capacity-register fact is worth remembering if it bit you).

## Session close (mandatory, CLAUDE.md)

1. File beads for any follow-ups (e.g. the future capacity-register nicety if
   the hypervisor folks want a bead on their side — ours records the pointer).
2. Quality gates: fmt, clippy `-D warnings`, `cargo test --workspace
   --locked`, musl agent build, VM tier suites.
3. Close/update beads.
4. `git pull --rebase && bd dolt push && git push` — verify
   `git status` shows up to date with origin. Work is not done until push
   succeeds.
5. Hand off: tell the bridge session the resolution is filed (they re-verify
   with the probe + the real-worker boot and answer with
   `04-verification.md`).

## Explicitly out of scope

- Editing reference-workload or determinism-hypervisor.
- Running `dh-m9-ready-handoff` / the step-2 real-worker invocation (bridge
  does this; scratch recipe lives with them).
- Rebuilding/restarting anything under `~/git/preestablished/.dh-clean-*` or
  the deployed worker.
