# Suggestions (non-blocking)

## 1. `tests/loopback.rs:648-682` — the acceptance test never exercises `ring_push`/`pio_answer` byte-faithfulness

**What:** Assertion 3 ("every host mutation exactly once") only ever sees `ConsBump`
ops, because the loopback drives drains but never `push_command`/`push_workload_ctrl` or
`InjectResponder::answer`. Byte-faithfulness of `ring_push` spans and `pio_answer` values
is covered by `commands.rs`/`inject.rs` unit tests, so the invariant *is* tested — but the
headline M1 acceptance test does not close the loop on host *writes* to the rings.

**Why:** The most load-bearing invariant is "every host mutation flows through the sink
faithfully." Having the 10^5-event acceptance test also push a handful of ring-C commands
and answer a few INJECT detcalls (and assert the resulting `RingPush`/`PioAnswer` ops
decode back to what was pushed/answered) would make the acceptance test a complete
witness for the whole sink contract, not just the cons-bump leg.

**Snippet (sketch):** after the main loop, before the final assertions, push a couple of
commands and assert the trace decodes:
```rust
ch.push_command(&Command::ReverifyRegions, &mut sink).unwrap();
match sink.ops.last() {
    Some(SinkOp::RingPush { ring: RingId::C, bytes, new_prod }) => {
        let (_, back) = decode_command(bytes).unwrap();
        assert_eq!(back, Command::ReverifyRegions);
        assert_eq!(*new_prod, read_u32(RingId::C.prod_offset()));
    }
    other => panic!("expected ring-C push, got {other:?}"),
}
```
Then relax assertion 3's `other => panic!(...)` to allow `RingPush`/`PioAnswer`.

## 2. `inject.rs:54` — `intern_name(name_id).map(str::to_owned)` allocates a `String` per answer

**What:** Every matched INJECT answer clones the interned name into an owned `String`
solely to pass `Option<&str>` to `FaultPlan::decide`. On the hot INJECT path this is an
allocation per detcall.

**Why:** API.md §1.4 notes inject points sit at I/O boundaries (µs-scale VM exits), so this
is not a measured hot loop, but the allocation is avoidable. The borrow conflict that
forces the clone (`channel` is borrowed mutably by `take_pending_inject`, then immutably
by `intern_name`) can be sidestepped by reading the name *before* removing the pending
entry, or by having `decide` take the `Channel`'s intern table by reference. Low priority.

**Snippet:**
```rust
let value = match channel.pending_injects.get(&iseq).copied() {
    Some(name_id) => {
        let decision = {
            let name = channel.intern_name(name_id); // &str borrow, no clone
            self.plan.decide(iseq, name_id, name).pack()
        };
        channel.pending_injects.remove(&iseq);
        decision
    }
    None => { channel.unmatched_injects += 1; FaultDecision::Proceed.pack() }
};
```
(Requires loosening `pending_injects`/`interns` visibility or adding a helper that returns
both; keep `take_pending_inject` for the simple callers.)

## 3. `tests/loopback.rs:297` — `OwnedPayload::RegionUpdate` is `unreachable!` in the expected-event mapping

**What:** `expected_event` maps `RegionUpdate(_)` to `unreachable!("not produced here")`.
That is true *today* (the simulator only emits `RegionRegister`), but it is a latent
panic if a future edit adds a `RegionUpdate` to the event mix. Same for `Pad`.

**Why:** Defensive; an `unreachable!` in a test helper turns a future test extension into a
confusing panic instead of a compile error. Consider handling `RegionUpdate` symmetrically
with `RegionRegister` (the `OwnedPayload` variant exists), or leaving a comment that the
arm must be filled before adding that kind to the simulator.

## 4. `tests/loopback.rs` — run the unsafe `RawChannelMem` path under miri at least once

**What:** The loopback's `RawChannelMem` + `Producer::from_raw` is the only unsafe surface
the host crate exercises. It is sound by the disjoint-region argument, but x86 (and a
plain `cargo test`) hides aliasing/provenance bugs (per the SPSC research note).

**Why:** A one-shot `cargo +nightly miri test -p detguest-host --test loopback` (or a
smaller miri-sized variant gated behind `cfg(miri)` with `TOTAL` reduced to e.g. 2_000)
would lock in the Stacked/Tree-Borrows soundness of the raw-pointer sharing against future
edits. Not required for M1, but cheap insurance for the one place `unsafe` lives.

## 5. `drain.rs:262-279` — two sequential `if let` matches on the same `owned` value

**What:** The drain folds `NameIntern` and `InjectQuery` via two back-to-back
`if let OwnedPayload::… = owned { … }` blocks, then pushes `owned`. This works (the value
is moved only at the final `out.push`), but a single `match &owned { … }` reads more
clearly and avoids the reader wondering whether the first `if let` consumed the value.

**Why:** Pure readability; no behavioral change. Optional.
