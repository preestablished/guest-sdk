# Action Items

### Critical
- [ ] None.

### Important
- [ ] None.

### Suggestions
- [ ] [tests/loopback.rs:648-682] Extend the M1 acceptance test to also push a few ring-C
      commands and answer a few INJECT detcalls, asserting the resulting `RingPush`/
      `PioAnswer` sink ops decode back to what was pushed/answered — so the headline
      acceptance test witnesses host *writes* (byte-faithful `ring_push`/`pio_answer`),
      not only cons-bumps.
- [ ] [inject.rs:54] Avoid the per-answer `String` allocation: read the interned name as
      `&str` before removing the pending entry (or pass the intern table by reference to
      `FaultPlan::decide`) so a matched INJECT answer does not clone the name.
- [ ] [tests/loopback.rs:297] Replace the `RegionUpdate(_) => unreachable!()` arm in
      `expected_event` with a real mapping (the `OwnedPayload::RegionUpdate` variant
      exists) or a comment, so adding that kind to the simulator later is a compile-time
      change, not a runtime panic.
- [ ] [tests/loopback.rs] Add a one-shot miri run of the `RawChannelMem` path
      (`cargo +nightly miri test -p detguest-host --test loopback`, optionally a
      `cfg(miri)` variant with a reduced `TOTAL`) to lock in the raw-pointer aliasing/
      provenance soundness against future edits.
- [ ] [drain.rs:262-279] Optionally fold the two sequential `if let OwnedPayload::… = owned`
      blocks into a single `match &owned { … }` for readability (no behavior change).
