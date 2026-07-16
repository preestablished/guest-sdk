# Positive Notes

### P1 — The `ChannelWriteSink` invariant is interpreted correctly against ARCHITECTURE.md §2

`crates/detguest-host/src/commands.rs:112-116` publishes the producer index and then reports
the *single* `ring_push(ring, &span, new_prod)` covering pad+record bytes plus the new index.
This exactly matches ARCHITECTURE.md §2's loggable-mutation list (lines 173-176): "pushing a
record (ring id + bytes)" and "bumping a consumer index (ring id + new index)" are two record
types; the producer-index publish is *part of* the push record, not a separate mutation. The
implementation does **not** double-log the prod store, and the `RecordingSink` trace in
`push_command_writes_record_and_logs_mutation` (commands.rs:160-170) pins exactly one
`RingPush` per push. This is the subtle thing it would be easy to get wrong, and it is right.

### P2 — Push framing is literally the same math as the SDK producer

`push_record` (commands.rs:72-118) reuses `bytes_needed`/`free`/`contiguous_tail`/`encode_pad`
from `detguest-wire::ring`/`record` — the same functions `Producer::try_push`
(detguest-wire/src/ring.rs:144-186) uses — including the pad-consumes-its-own-seq rule
(commands.rs:96-98 mirrors ring.rs:165-167). Host pushes and guest pushes therefore frame
identically by construction, not by parallel re-implementation. The `wrap_emits_pad_in_same_logged_span`
test (commands.rs:217-251) verifies the pad+record land in one logged span.

### P3 — Drain tolerances match the spec precisely and fail closed

`drain.rs:229-258` gets the boundary conditions right: `avail > size` (not `>=`) is
`CorruptIndices` so a legitimately full ring is accepted; `len > avail` is a clean *stop* (the
producer is mid-write — partial records never partially decode); `len > tail` is *corruption*
(records never wrap). Unknown kinds are skipped by `len` and counted in `unknown_kind_records`
(drain.rs:289-292) per API.md §3.5, rather than aborting the drain. `Pad` records advance the
consumer and are consumed silently (the `to_owned` → `None` path, drain.rs:133), and the
cons-bump is logged exactly once at the end (drain.rs:298-301) even when the only records
consumed were pads.

### P4 — Seqlock reader discipline is correct and bounded

`read_manifest` (manifest.rs:70-107) implements the canonical even-generation /
copy / re-read-generation / retry-on-change loop, rejects odd (writer-in-progress) generations,
and bounds retries (`SEQLOCK_RETRIES = 64`) so a stuck/corrupt generation word surfaces as
`SeqlockLivelock` rather than hanging. The `seqlock_odd_generation_then_recovery` test
(manifest.rs:242-268) exercises both the torn-read rejection and recovery. This matches the
research note on seqlock reader discipline.

### P5 — The loopback test's aliasing argument is genuinely sound

`RawChannelMem` is `Copy`; `Channel::attach` takes one copy, `sim.mem` is another, and the
`Producer`s hold raw pointers — all three alias the *same* leaked allocation (the real
"shared hugepage" model). This is only sound because the test is single-threaded and "phases
strictly alternate" (loopback.rs:37-39): producers write the free region, the host reads the
used region and owns the consumer cells, and the two never run concurrently. The SAFETY
comments (loopback.rs:328-329) name exactly the invariant being relied on. The crate proper
keeps `#![forbid(unsafe_code)]`; all `unsafe` is quarantined to this test harness.

### P6 — Type system used to enforce a normative invariant

`push_workload_ctrl` / `WorkloadCtrl` (commands.rs:1-6, 46-65) make it *impossible* to put
pad input on ring I — there is simply no input-bearing variant — which is how the module
comment says ARCHITECTURE.md §2's "ring I never carries pad input" is meant to be enforced.
Encoding a normative constraint as an unrepresentable state is the right tool here.

### P7 — Honest, well-reasoned spec-deviation documentation

The `RING_W_SIZE` comment (detguest-wire/src/header.rs:92-103) and the `AttachError`
status-mapping comments (channel.rs:54-65) document *why* the implementation departs from a
literal reading of the layout table (power-of-two requirement vs the 0x1E0000 figure), rather
than silently picking one. This is exactly the kind of load-bearing reasoning a future
maintainer needs.
