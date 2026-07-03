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
   - Size discovery: sequential reads with BAD_REQUEST tail-narrowing within
     the existing device ABI (no hypervisor change). Note for the future: a
     read-only capacity register in dh-devices would simplify this — a
     nicety, not needed now.
   - **Staging requirement — validate at the source, the agent cannot**:
     the device addresses whole sectors only (`capacity = len/512`,
     truncating), and BAD_REQUEST is the only past-the-end signal, so a
     partial tail is **undetectable in-guest** — it silently truncates (our
     Negative 2 demonstrates the downstream symptom). The bridge must
     validate the staged `DH_M9_GAME_IMAGE` before boot: `size % 512 == 0`,
     ideally the full cart rule (power of two, ≥ 32 KiB — refwork
     `Cartridge::from_rom` enforces it in-guest as `BadRomSize`, but that
     fault is attributed to the cart parser, not staging, and costs debug
     time). Positive fact worth stating: capacity truncates and never pads,
     so for a valid ROM the materialized bytes are byte-identical to the
     staged file and the `blake3` cart hash in `meta`/READY evidence is
     unperturbed.
   - Size cap: 32 MiB, loud fault above (the game peaks at two RAM copies —
     file + harness's `Cartridge.rom` — against 128 MiB; the agent unlinks
     the file after the control leg so steady state holds one).
   - The agent never writes the device — pv-blk overlay stays clean, so no
     new dirty-cluster load in hypervisor snapshots. Post-materialization the
     device's SECTOR/BUF_GPA/COUNT/STATUS registers hold the last read's
     values at READY — deterministic (pure function of the image), part of
     device snapshot state, fine but should be known.
   - READY icount will shift (added deterministic pre-Ready work) — expected;
     step-3 READY-snapshot regeneration absorbs it. It is now a pure function
     of WorkloadImage **plus** the content-addressed game image (both pinned
     inputs; ARCHITECTURE.md §4.1 updated accordingly).
3. **reference-workload instructions** (they drive that side, request
   `01-options.md` §Cross-Repo):
   - Add `game_source = "pv-blk"` to `image/boot.toml` `[unit.control]`
     (one line), and make their `xtask` `validate_boot_toml`
     (`xtask/src/image.rs:1619-1672`, an exact-match allowlist validator)
     **require** the key — merely *admitting* it would let a future edit
     drop the line and silently reintroduce the cannot-read-`/dev/vdb` boot
     fault this request exists to fix, with no build-time catch.
   - `refwork-harness` loader: no change (path-agnostic; it will receive
     `/run/detguest/game.img`).
   - Stale `/dev/vdb`-era names to rename at leisure (now logical/stale, not
     load-bearing): `harness.toml`'s `game_image_device = "/dev/vdb"`
     (validated at `image.rs:1690`) and the dist-manifest device block
     `{ kind: virtio-blk, role: game-image, … }` (`image.rs:1051,1210`) —
     both describe a pv-blk MMIO device by a virtio name.
   - Bump `image/guest-sdk.lock` `rev` to our landed SHA and rebuild the
     image (their build refuses on rev mismatch until bumped — known,
     one-line).
   - m9 staged fixture and `boot.toml.m9-refwork-contract`: intentionally
     untouched (no `game_source` ⇒ verbatim `/dev/vdb`, and the fixture's
     `dev_path == "/dev/vdb"` assert still holds).
4. **VM-tier evidence**: test names, the byte-exact checksum value, both
   negative-control fault lines, and the `boot_probe` expectation — against
   the rebuilt package-04 image the probe's last event becomes the agent's
   `pv-blk: no device …` fault (the probe harness has no pv-blk;
   layer-visible success per their caveat). Prefer *observed* over predicted:
   run the request's two-minute probe recipe against a local, uncommitted
   reference-workload lock-bump + `game_source` line before handback
   (00-overview acceptance #3).
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
