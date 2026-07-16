# Suggestions (non-blocking)

### S1 — Duplicate `NameIntern` with a *different* name is silently first-wins and uncounted

**File:** `crates/detguest-host/src/drain.rs:269-275`

```rust
self.interns
    .entry(name_id)
    .and_modify(|e| e.reachable_decl |= reachable_decl)   // name never updated
    .or_insert_with(|| InternEntry { name: …, reachable_decl });
```

If the same `name_id` is interned twice with **different** name bytes, the first name wins,
the second is dropped silently, and nothing is counted or warned. The raw `GuestEvent` still
carries the second name, so a consumer comparing `event.name` against `intern_name(id)` would
see a mismatch with no signal. Per the crate's own pattern for the other "shouldn't happen but
tolerate" cases (`unknown_kind_records`, `unmatched_injects`), consider a
`intern_id_redefined: u64` metric and keeping first-wins:

```rust
.and_modify(|e| {
    e.reachable_decl |= reachable_decl;
    if e.name.as_bytes() != name.as_slice() { self.intern_redefined += 1; } // count, keep first
})
```

(Note the borrow: `self.intern_redefined` can't be touched inside `and_modify` if it borrows
`self.interns`; restructure to a `match self.interns.entry(...)` so the counter bump lives
outside the closure.)

### S2 — Lossy-UTF-8 divergence between `intern_name()` and `GuestEvent.name` is undocumented

**File:** `crates/detguest-host/src/drain.rs:273` (`String::from_utf8_lossy`)

`intern_name()` returns the lossy-converted string (invalid bytes → U+FFFD), while the
drained `OwnedPayload::NameIntern.name` preserves the raw bytes. The `OwnedPayload` doc
(drain.rs:32-33) notes string fields stay raw, but does not call out that the *intern table*
applies lossy conversion, so the two views of the same name can differ byte-for-byte. One
sentence on `intern_name()` documenting "names are interned via lossy UTF-8; the raw bytes are
on the `GuestEvent`" would close the gap.

### S3 — `read_region` empty-buffer / over-coverage semantics deserve a test + a doc line

**File:** `crates/detguest-host/src/manifest.rs:113-153`

The current behavior is reasonable but undocumented and untested:
- `offset == region.len` with an empty `buf` → `Ok(())` (end == len, not `>`), no reads.
- `offset > region.len` with an empty `buf` → `OutOfBounds`.
- Extents summing **greater** than `region.len` are *allowed* (only under-coverage is
  rejected); the walk clamps to `region.len`, so over-coverage is harmless.

Add a one-line doc note ("offset may equal len for an empty read; extent tables may
over-cover the logical length") and a small unit test pinning the empty-buf and over-cover
cases so a future refactor doesn't silently change them.

### S4 — Loopback never asserts a ring wrap occurred / pads were consumed

**File:** `crates/detguest-host/tests/loopback.rs`

Ring A is 64 KiB and takes ~15k events of ~30 B, so the free-running index passes 64 KiB many
times and the producer *must* emit `Pad` records at the wrap boundaries — and the drain *must*
silently consume them. The test recovers events correctly through those wraps (so pad handling
is implicitly exercised), but **nothing explicitly asserts** that any wrap/pad happened. A
regression that broke pad insertion or pad-consumption could still pass if the event sequence
happened to avoid the boundary. Consider tracking producer `prod` crossing `size` (or counting
`Pad`s the simulator induced) and asserting `> 0`, mirroring the existing
`assert!(sim.drops.w_records > 0)` belt-and-suspenders check.

### S5 — Loopback's doorbell-retry path is likely never exercised

**File:** `crates/detguest-host/tests/loopback.rs:170-174`

Critical-event pushes on a full ring take the doorbell branch (`drain_and_collect` then
`continue`). With a drain every 4096 iterations the rings rarely fill on the critical path, so
this branch may execute **zero** times across the run — a documented "critical path"
(ARCHITECTURE.md §3 doorbell + retry) with no actual coverage. Add a counter on the doorbell
branch and `assert!(doorbells > 0)`, or construct a dedicated burst of critical events with no
intervening drain (mirroring the i==70_000 droppable burst) so the retry loop is guaranteed to
run at least once.

### S6 — `MockGuestMem::add_segment` end computation is unchecked (test-only, minor)

**File:** `crates/detguest-host/src/guestmem.rs:74`

```rust
let s_end = s.base + s.data.len() as u64;   // unchecked
```

`add_segment` correctly `checked_add`s the *new* segment's end (line 70) but the existing
segments' ends in the overlap loop are computed with a plain `+`. For pathological test inputs
(`base` near `u64::MAX`) this could panic in debug; harmless in practice since this is
test-construction code, but trivially fixable with the same `checked_add(...).expect(...)`
used two lines above, for consistency.
