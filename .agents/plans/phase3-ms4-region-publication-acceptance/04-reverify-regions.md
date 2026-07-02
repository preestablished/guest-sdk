# 04 — Real `ReverifyRegions`

Request blocker #2. Semantics are already specced: API.md §6 (~line 626):
"re-walk pagemap for all live regions; emit RegionUpdate per region whose
extents changed (P0 alarm) or unchanged (generation echo)". ARCHITECTURE.md §5
calls it the P0 pinning canary. The host sends it after restore/fork.

## Implementation

Replace the stub at `crates/detguest-agent/src/commands.rs:77-82` with a call
into the region ledger from `02-…`:

```rust
Command::ReverifyRegions => {
    region_ipc::reverify(sup)?;   // same borrow-split shape as service()
}
```

`reverify` for each **live** `RegionRecord`:

1. Re-walk: `open_pagemap_for(record.pid)` + `build_extents(record.gva,
   record.len)` — same injectable-translator structure as registration so
   tests can simulate drift without real pagemap access.
2. Compare against `record.extents`.
   - **Unchanged**: emit `RegionUpdate(RegionEvent{region_id, name_id,
     layout_version, manifest_generation})` with the *current* (even)
     generation — the "generation echo".
   - **Changed** (different extents): P0 alarm — emit an agent `LogLine`
     (stream AGENT, level 0) naming the region and the first differing
     extent, rewrite the region's extents in the manifest under the seqlock
     (new pool slots if the count grew and the pool has room, else
     `TooManyExtents` → treat as unmappable below), update `record.extents`,
     then emit `RegionUpdate` with the new generation.
   - **Unmappable** (translate error: NotPresent/Swapped/PfnHidden/Io, e.g.
     workload died or pages were reclaimed): P0 alarm LogLine; mark the
     manifest entry DEAD under the seqlock; mark the record dead; emit
     `RegionUpdate` with the new generation. The host observes the DEAD flag
     on its next `read_manifest` and the RegionUpdate as the signal.
3. One doorbell after the sweep (`emit_with_doorbell` on the last event, or
   explicit doorbell) so the host drains a complete batch.
4. Workload pid gone (record.pid no longer supervised / process dead): treat
   as unmappable per-region; do not skip silently.

Determinism: iterate records in region_id order; no clocks, no allocation
patterns dependent on external state beyond pagemap contents.

Also delete/replace the stub-asserting test
`reverify_regions_is_currently_noop_without_regions` (`commands.rs:99-118`) —
its replacement below asserts the no-regions case emits nothing (still true:
empty ledger → no events, no doorbell).

## Restore/fork correctness note

`RegionRecord`s live in agent process memory, which is guest RAM — they
survive snapshot/restore/fork by construction, and `pid` remains valid inside
the restored guest. This is exactly why the ledger must be agent-side. Add
this note to ARCHITECTURE.md §5 (in `07-…`).

## Tests (request acceptance #2: corruption is detected)

Host unit tests in `detguest-agent` with injected translators:

- Empty ledger → no events, no doorbell (replaces the stub test).
- Two live regions, translator returns identical extents → exactly two
  `RegionUpdate` echoes, generations unchanged, no LogLine, one doorbell
  batch.
- Translator returns shifted GPA for one region → P0 LogLine + manifest
  extents rewritten (verify via `copy_manifest_stable`) + `RegionUpdate` with
  bumped generation; the other region still echoes.
- Translator returns `NotPresent` (the "deliberately corrupted/unmapped
  region") → P0 LogLine + entry DEAD in manifest + `RegionUpdate`; a
  subsequent `read_manifest` on the host side (use `detguest-host` against
  the manifest bytes) no longer resolves the name.
- Dead records are skipped.

VM tier: the `06-…` acceptance sends `ReverifyRegions` after every restore and
asserts the echo path (RegionUpdate count == live regions, no P0 LogLine) —
proving the command is exercised on the real restore path, per the request
("the restore/fork path exercises it").

## Done when

Unit tests above green; `CAP_REVERIFY_REGIONS` capability bit already
advertised in Hello (`runtime.rs:150`) is now truthful.
