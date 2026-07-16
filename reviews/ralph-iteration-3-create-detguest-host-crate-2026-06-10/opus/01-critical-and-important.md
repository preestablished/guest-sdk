# Critical and Important Findings

**None.**

No Critical findings: the no-mutate-without-sink invariant holds, drain/push arithmetic
is correct, the seqlock and extent-walk paths are bounded and overflow-safe, and there
is no mutate-without-sink path.

No Important findings.

## Evidence for the load-bearing invariants (why there are no Criticals)

### (a) No mutate-without-sink — every channel-memory `gm.write` is reported once, faithfully

Enumerating every production-path write of channel memory (test-module writes excluded):

| Site | Write | Sink report |
|---|---|---|
| `commands.rs:108` | pad bytes at old masked pos (`push_record`) | `sink.ring_push(ring, &span, new_prod)` at `commands.rs:116` |
| `commands.rs:113` | record bytes at wrapped pos (`push_record`) | same single `ring_push` |
| `commands.rs:115` | `write_u32(prod_gpa, new_prod)` | same single `ring_push` |
| `drain.rs:299` | `write_u32(cons_gpa, pos)` | `sink.cons_bump(ring, pos)` at `drain.rs:300` |
| `inject.rs` (no `gm.write`) | — | `sink.pio_answer(PORT_INJECT, value)` at `inject.rs:62` |

- The two-part ring write (pad at `pos & mask`, record at offset 0 after wrap) is
  reported as a single contiguous `span` whose bytes equal, in ring order, exactly the
  bytes that landed (pad then record). `new_prod = prod.wrapping_add(needed)` is the
  published index. **Byte- and index-faithful.** Verified directly by the unit tests
  `push_command_writes_record_and_logs_mutation` (asserts `bytes == rec`,
  `new_prod == 24`) and `wrap_emits_pad_in_same_logged_span` (asserts the 16-byte pad +
  24-byte record span and `new_prod == prod + 40`).
- The cons bump is guarded by `if pos != cons` (`drain.rs:298`), so a no-op drain logs
  nothing — no spurious sink op. `pos` is the post-drain free-running index.
- `RingFull` returns *before* any write or any sink call (`commands.rs:87-89`); the unit
  test `full_ring_reports_ring_full_without_mutating` pins "failed push logs no mutation."
- `pio_answer` is logged on both the matched and the unmatched/Proceed path
  (`inject.rs:62` is unconditional after the `match`), so every `IN 0xD384` answer is
  recorded — correct per API.md §5 (the unmatched-Proceed answer is still an input-log
  record).

### (b) drain_events correctness

- **Mid-write tolerance:** `if len > avail { break; }` (`drain.rs:251`) stops at the last
  complete record without advancing past it; the cons bump uses `pos` (last complete
  boundary), never `prod`. Correct.
- **Pad skip:** `to_owned` returns `None` for `EventPayload::Pad` (`drain.rs:133`); the
  loop still advances `pos += len` and never surfaces the Pad. Correct.
- **Unknown-kind skip-by-len:** `Err(DecodeError::UnknownKind(_))` increments
  `unknown_kind_records` and advances by `len` (`drain.rs:289-295`) — matches API.md §3.5.
- **CorruptIndices:** `avail > size` ⇒ `CorruptIndices` (`drain.rs:230`); `len > tail`
  (record would wrap, which the framing forbids) ⇒ `Decode(BadLen)` (`drain.rs:255-258`);
  malformed `len` (misaligned / below the kind's minimum / over `MAX_RECORD_LEN`) ⇒
  `Decode(BadLen)` (`drain.rs:248`). The `len`-minimum is correctly split: `PAD_MIN_LEN`
  (8) for kind 0, `MIN_RECORD_LEN` (16) otherwise. All paths are bounded reads (no panic).
- **Ring order A-then-W:** `for ring in [RingId::A, RingId::W]` (`drain.rs:211`).
- **seq/vnanos/truncated propagation:** `GuestEvent { seq: hdr.seq, vnanos: hdr.vnanos,
  truncated: hdr.flags & FLAG_TRUNCATED != 0, .. }` (`drain.rs:280-286`).
- **Intern folding:** `NameIntern` folds first-wins on the string, OR-ing
  `reachable_decl` across re-interns (`.and_modify(|e| e.reachable_decl |= reachable_decl)`,
  `drain.rs:271`) — matches API.md §1.2 (REACHABLE_DECL is a declaration flag that should
  stick once set).

### (c) push paths

- Space check uses `bytes_needed(prod, size, total)` against `free(prod, cons, size)`
  (`commands.rs:86-89`) — identical to `wire::ring::try_push` (`ring.rs:155-158`).
- Tail-pad seq consumption matches the guest producer: pad consumes a seq, then the
  record consumes the next (`commands.rs:96-101` mirrors `ring.rs:164-170`). The host is
  the *sole* producer of rings C/I, so its `next_seq_c`/`next_seq_i` are the only seq
  source — no cross-producer consistency requirement, only self-consistency, which holds.
- `vnanos = 0` on host records: `encode_command` hardcodes vnanos 0
  (`events.rs:659-688`), `push_workload_ctrl` passes 0 explicitly (`commands.rs:59`).
- Ring-I type safety: `WorkloadCtrl` has no pad/input-bearing variant (enforced by the
  enum), satisfying ARCHITECTURE.md §2 "Ring I carries no pad data."

### (d) read_manifest / read_region

- Seqlock loop: bounded `SEQLOCK_RETRIES = 64`; odd `g1` ⇒ retry; `g1 != g2` ⇒ retry;
  exhaustion ⇒ `SeqlockLivelock` (`manifest.rs:74-106`). `header.validate()` bounds
  `region_count ≤ 64` and `extent_count ≤ 1024`; each live entry's
  `validate_extents` bounds `extent_off + extent_n ≤ extent_count` with `checked_add`.
- `read_region` extent walk: `offset.checked_add(want)` overflow guard;
  `end > region.len` bound; coverage check `sum(extents.len) >= region.len`; per-extent
  `to_skip` skip then `split_at_mut(take)` — arithmetic is `x.len - to_skip` (always
  positive since the `to_skip >= x.len` branch handles full skips) and
  `min(x.len - to_skip, remaining.len())`. No off-by-one or overflow.

### (e) InjectResponder

- Pending-table lifecycle: `take_pending_inject` *removes* the entry
  (`channel.rs:255-257`), so a re-answered iseq falls to the unmatched/Proceed +
  `unmatched_injects += 1` path. Pinned by `responder_answers_matched_query_via_plan_and_logs_pio`.
- `TableFaultPlan` occurrence counting increments `hits[i]` on every *name-match* of a
  rule, before the occurrence test (`inject.rs:114-115`) — matches the bead intent
  "match by name glob + occurrence index." First matching rule (name AND occurrence) wins.

### (f) loopback test soundness

- `RawChannelMem` aliasing: raw `*mut u8` aliases are not UB; the producers create
  `&mut [u8]` only over the *free* region while the host reads the *used* region via raw
  `copy_nonoverlapping`; phases strictly alternate single-threaded ⇒ no overlapping `&mut`.
  Provenance flows from the single `Box::leak`. Sound.
- Drop bookkeeping mirrors the header writes the simulator performs and is checked
  against `ch.drop_counters()` (assertion 2), including per-kind.
- The "exactly once" mutation check (assertion 3) verifies the trace is `ConsBump`-only,
  strictly advancing per ring, ending at `cons == prod`. Byte-faithfulness of `ring_push`
  is instead pinned by the `commands.rs` unit tests; see suggestion 02-#1.
