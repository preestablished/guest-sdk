## Action Items

### Critical
- [ ] None.

### Important
- [ ] [crates/detguest-wire/src/ring.rs:174] Change `fn slice_mut(&self, ...) -> &mut [u8]` to take `&mut self`. As written it triggers `clippy::mut_from_ref` (deny-by-default), which fails `cargo clippy --workspace` and removes the borrow-checker's aliasing guarantee in the only unsafe module. `try_push` already holds `&mut self` and the pad-slice borrow ends at the `encode_pad` call, so the second `slice_mut` for the record borrows freely — no logic change needed. Re-run `cargo clippy --workspace --all-targets` to confirm the error clears.

### Suggestions
- [ ] [crates/detguest-wire/src/ring.rs:136] Replace the `total_len >= PAD_MIN_LEN && total_len <= MAX_RECORD_LEN` chain with `(PAD_MIN_LEN..=MAX_RECORD_LEN).contains(&total_len)` to clear the `manual_range_contains` clippy warning (the only output left after the Important fix).
- [ ] [crates/detguest-wire/src/ring.rs:262] Either drop the redundant `RecordHeader::read_from(&scratch[..len])?` re-validation in `pop_into` (the inline checks already cover it) or reword the comment to "defense-in-depth re-parse" so the inline checks aren't later mistaken for removable.
- [ ] [crates/detguest-wire/src/ring.rs:36] Make `free()` panic-free against corrupt/forged index pairs by using `size.saturating_sub(used(prod, cons))` (consistent with the crate's "arbitrary bytes never cause UB" posture; no live hazard today since `pop_into` doesn't call it).
- [ ] [crates/detguest-wire/src/ports.rs:100] Guard against the lossy `FaultDecision::Platform { kind: 0, .. }` (and `Workload { kind: 0 }`) construction — add `debug_assert!(kind != 0)` in `pack()` or a doc line restating the `1..=63` / `64..=255` kind invariants, since such a value silently round-trips back to `Proceed` dropping `arg`.
- [ ] [crates/detguest-wire/src/events.rs:502] Add a doc note to `decode_event` (and the other decoders) that string fields are returned as raw `&[u8]` with UTF-8/NUL validity being the producer's contract, not enforced here, so host consumers don't assume `from_utf8` always succeeds.
- [ ] [crates/detguest-wire/src/events.rs (tests)] Add byte-exact golden fixtures (`assert_eq!(&buf[..n], &[…])`) for every v1 payload and for the manifest header/entry/extent, per `API.md` §3.5 ("golden tests pin every byte") and the wire-codec research note — round-trip tests alone can't catch wrong-but-symmetric layouts. Tracking-only for this skeleton checkpoint.
