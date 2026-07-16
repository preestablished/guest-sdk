# Critical & Important Findings

**None.**

I went looking specifically for the things a first reviewer would gloss over, and found no
must-fix or should-fix defects in the shipped code. The two areas most likely to hide a bug both
came up clean on close inspection:

## Why the `payload_range` fix is complete (not just locally patched)

The fix returns `l..l` (empty range *at* `len`) instead of `16..len` for any record with
`len <= 16`. I verified every consumer:

- `decode_event` (events.rs:512), `decode_command` (events.rs:703), and
  `decode_workload_ctrl` (events.rs:798) are the only `payload_range()` call sites outside the
  fix's own doc/test.
- All three call `RecordHeader::read_from` *first*. `read_from` (record.rs:232-239) sets
  `min = PAD_MIN_LEN (8)` only when `kind == Pad (0)`, otherwise `min = MIN_RECORD_LEN (16)`, and
  rejects `l < min` with `BadLen`. So the only way to reach `payload_range` with `len <= 16` is
  either a `len == 8` `Pad` (→ `8..8`) or a `len == 16` record (→ `16..16`); both are empty and
  both are in-bounds for the record's own bytes. Slicing `&bytes[8..8]` / `&bytes[16..16]` cannot
  panic.
- The "record claiming `len == 8` with `kind != 0`" attack the prompt asked about is rejected at
  `read_from` before `payload_range` is ever called — confirmed by the existing
  `framing_rules_enforced` test (record.rs:339-349). Each non-`Pad` decode arm additionally
  guards `payload.len() < N`, so even a hypothetical empty payload returns `BadLen`, never an
  out-of-bounds index.

Conclusion: the fix is correct at every call site; there is no second un-patched path.

## Why the SPSC orderings are unchanged and sound

`src/ring.rs`'s production code is byte-for-byte the iteration-1 code — the only diff is a
test-only `const N` shrink under `cfg!(miri)` (ring.rs:481-483). The release/acquire discipline
(producer Relaxed-loads its own `prod`, Acquire-loads peer `cons`, single Release-store publishes
pad+record together; consumer mirror) matches the rule table in
`spsc-ring-memory-ordering.md`. The TOCTOU concern from that note (re-validating a length read
from concurrently-writable ring memory) is explicitly handled: `pop_into` copies bytes out and
re-parses the local copy (ring.rs:282-287). Nothing in this branch weakens that.

See `02-suggestions.md` for the non-blocking coverage and maintainability items.
