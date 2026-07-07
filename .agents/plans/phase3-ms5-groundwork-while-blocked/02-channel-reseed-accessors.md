# Package 02 — `guest-sdk-4bc`: Channel Intern-Map / Pending-Inject Re-Seed Accessors

Closes the ready bead `guest-sdk-4bc` (P2), the debt the Ms4 resolution
filed. Claim it first (`bd update guest-sdk-4bc --claim`). Re-read the
Ms4 resolution's "Notes for anyone touching this next"
(`.agents/requests/phase3-ms4-region-publication-acceptance/05-resolution.md`)
before starting — it is the provenance for this bead and for the
register-path deadlock constraint anyone in this area must know.

## Current state

- `crates/detguest-host/src/channel.rs:106-108`: `interns:
  BTreeMap<u32, InternEntry>` and `pending_injects: BTreeMap<u32, u32>`
  are `pub(crate)`, no re-seed accessors. `InternEntry` (`:68-73`) is
  `{ name: String, reachable_decl: bool }`, also `pub(crate)`.
- Existing precedent for exactly this shape:
  `producer_seqs()`/`restore_producer_seqs()`
  (`channel.rs:206-217`) with the public `ProducerSeqs` carrier struct.
  Follow it.
- `tests/vm/src/harness/snapshot.rs`: `HostChannelState.interns:
  Vec<InternRecord>` already carries the drained intern records
  (`InternRecord { name_id: u32, name: Vec<u8>, reachable_decl: bool }`)
  "so a child can re-seed name resolution once detguest-host grows a
  `Channel::restore_interns`". `from_snapshot` re-attaches and restores
  producer seqs but leaves a comment: "The intern map cannot be
  re-seeded yet."

## Work

### detguest-host

1. Add a public carrier for intern entries (mirroring `ProducerSeqs`),
   e.g. `InternSnapshotEntry { name_id: u32, name: String,
   reachable_decl: bool }` — or accept `(u32, &str, bool)` tuples;
   pick whichever reads best against the existing API style.
   Encoding note: `Channel` stores names as `String` (lossy UTF-8 at
   drain time, per the `intern_name` doc, `channel.rs:256-258`), while
   the harness `InternRecord` carries raw `Vec<u8>`. The accessor API
   takes/returns the host-side `String` form; the harness converts
   with the same lossy rule at the call site so a re-seeded child
   matches what a root that drained the events would hold. State this
   in the accessor doc comment.
2. `Channel::interns(&self)` — iterator or Vec of the carrier, for
   symmetric snapshot capture (today the harness reconstructs interns
   from drained events; capture-from-channel is the cleaner source
   once the accessor exists — switch the harness capture over).
3. `Channel::restore_interns(&mut self, ...)` — replaces the map.
   Decide and document collision semantics (restore into a
   freshly-attached child: the map must be empty; `debug_assert` or
   return error on non-empty — match how `restore_producer_seqs`
   handles being called late, and be consistent).
4. `Channel::pending_injects(&self)` / `restore_pending_injects(&mut
   self, ...)` — same pattern over `BTreeMap<u32, u32>` (iseq →
   name_id).
5. Unit tests in detguest-host: re-seeded channel resolves
   `intern_name(id)` and `take_pending_inject(iseq)` without any
   drain having occurred.

### tests/vm harness

6. `snapshot.rs::from_snapshot`: call `restore_interns` from the
   carried `hc.interns`; delete the "cannot be re-seeded yet" comment.
7. Extend `HostChannelState` with `pending_injects: Vec<(u32, u32)>`
   captured via the new accessor, restored in `from_snapshot`. Keep —
   but soften — the "snapshot at a boundary with no outstanding inject
   queries" doc note: it changes from a correctness constraint to a
   description of what the field preserves. Snapshot-format
   compatibility is not a concern (snapshots are in-process, never
   serialized to disk — confirm, and say so in the commit message).
8. Update the `HostChannelState`/`InternRecord` doc comments that
   currently document the limitation (the bead's acceptance names this
   explicitly).

### The proving test (acceptance criterion 1)

A test where a child `Channel` re-seeded via the new accessors
resolves a ring-event `name_id` **without falling back to manifest
name bytes**. Two tiers:

- Host-only unit test (ungated, runs everywhere): construct a channel
  over test guest-mem, re-seed interns, assert `intern_name` resolves —
  this is the substance.
- Harness tier: in the existing gated snapshot tests
  (`tests/vm/tests/m4_snapshot.rs` family), assert post-restore that
  the child's channel resolves a known intern id directly (not via
  manifest). Add the assertion to an existing test rather than a new
  boot — cheap, and it exercises the `from_snapshot` wiring. This repo
  runs on the lane host (`infra-control`), so **execute this tier
  before closing the bead** (`DETGUEST_VM_TESTS=1 cargo test -p
  detguest-vmtest --test m4_snapshot -- --ignored --test-threads=1`)
  rather than shipping it unexercised; record the run in the
  resolution.

## Done when

- Accessors landed with unit tests; harness re-seeds both maps in
  `from_snapshot`; docs updated at both sites.
- `cargo test -p detguest-host` and the workspace host-only suite
  green.
- `bd close guest-sdk-4bc --reason="..."` citing the commit SHA.
