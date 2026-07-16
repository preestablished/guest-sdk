# Positive Notes

## The no-mutate-without-sink invariant is enforced structurally, not just by convention
- `lib.rs:12` `#![forbid(unsafe_code)]` plus the trait-based `ChannelWriteSink` mean
  there is no API in the crate that mutates channel memory without taking a
  `&mut dyn ChannelWriteSink`. Both write sites (`push_record`, `drain_ring`) thread the
  sink and report exactly once. This is the cleanest possible expression of
  ARCHITECTURE.md §2's load-bearing invariant.
- `commands.rs:72-118` reports the two-part wrapped write (pad + record) as a *single*
  contiguous `span` in ring order, so the input log sees one faithful mutation rather than
  two partial ones — exactly what a replayer needs.

## Push arithmetic is shared with the guest producer instead of re-derived
- `commands.rs:86` reuses `wire::ring::bytes_needed`/`free`/`contiguous_tail`/`encode_pad`
  rather than re-implementing the pad/wrap math host-side. This is the same discipline the
  guest `Producer::try_push` uses (`ring.rs:155-175`), so the two sides cannot drift —
  precisely the "one implementation, not two" goal in ARCHITECTURE.md §1.

## Drain tolerances are spec-faithful and panic-free over arbitrary bytes
- `drain.rs:230` (`avail > size` ⇒ `CorruptIndices`), `:251` (mid-write `break`), `:255`
  (no-wrap `BadLen`), `:248` (len/align/min validation), `:289` (unknown-kind skip-by-len)
  cover every framing tolerance in API.md §3.0/§3.5. Every read is bounds-checked before
  indexing; a forged ring can never panic the drain — matching the no_std-codec research
  note's "decoders must be total."

## Seqlock and extent-walk reads are correctly bounded and overflow-safe
- `manifest.rs:74-106` implements the API.md §4.2 reader discipline exactly (even-or-retry,
  re-read generation, bounded retries) and revalidates every live entry's extent range
  inside the snapshot. `read_region` (`manifest.rs:113-153`) guards `offset + len` with
  `checked_add`, bounds against `region.len`, checks extent coverage, and walks with
  `split_at_mut` — no off-by-one. The 3-extent discontiguous-stitch acceptance case is
  covered both as a unit test and against the live manifest in the loopback.

## InjectResponder lifecycle is correct and the unmatched path is still logged
- `inject.rs:46-64`: `take_pending_inject` *consumes* the entry, so replaying the same
  iseq correctly falls to Proceed + `unmatched_injects` metric (API.md §5), and the
  `pio_answer` is logged unconditionally — the unmatched-Proceed answer is itself an
  input-log record, which is exactly right.
- The `*`-only `glob_match` (`inject.rs:143-154`) is dependency-free, total, and correctly
  treats an unresolved name as matching only `"*"` (`inject.rs:108-109`), so a not-yet-
  interned point never spuriously matches a specific rule.

## The MockGuestMem segment model rejects straddling and overlap
- `guestmem.rs:93-102` `locate` requires the whole `[gpa, gpa+len)` to sit inside one
  segment (`gpa >= base && end <= s_end`), so a read straddling a hole or two segments
  fails rather than silently splicing — pinned by `unmapped_and_straddling_accesses_fail`.
  Overlap on construction panics (`add_segment`), pinned by `overlapping_segments_panic`.

## Error → init-status mapping matches the PIO ABI
- `channel.rs:54-66` maps every `AttachError` to the correct `IN 0xD37C` status class
  (1 bad GPA, 2 bad magic/version/ring-descriptor, 3 already attached) per API.md §5, with
  ring-descriptor faults correctly classed as 2 ("readable header, not a valid v1 channel").

## Documentation and intent traceability
- Nearly every public item cites the governing API.md/ARCHITECTURE.md section, and the
  module headers restate the normative invariant they uphold (e.g. `commands.rs:3-6` on
  ring-I-has-no-pad-input, `lib.rs:1-11` on the two §2 invariants). `#![deny(missing_docs)]`
  keeps this honest.
