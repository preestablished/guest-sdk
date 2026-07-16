# Critical and Important Issues

**None.**

There are no must-fix (Critical) or should-fix-before-merge (Important) issues on this
branch. I specifically scrutinized, and cleared, each of the areas the review prompt
flagged as load-bearing:

## Cleared: `payload_range` fix is sound for every reachable `len`

`src/record.rs:263-270`. The new branch returns `l..l` instead of
`RECORD_HEADER_LEN..RECORD_HEADER_LEN` when `l <= RECORD_HEADER_LEN`. The concern was
whether this is correct for all `len` in `8..=16` and for `len = 0`.

- `read_from` (`src/record.rs:220-255`) rejects `len = 0` (and anything `< min`, where
  `min` is `PAD_MIN_LEN = 8` for Pad and `MIN_RECORD_LEN = 16` otherwise) with
  `DecodeError::BadLen` before any header with `len < 8` can exist. So `payload_range`
  is only ever called on a header whose `len ∈ {8} ∪ {16, 24, ...}` (Pads can be 8;
  non-Pads are ≥ 16). For `len = 8`: `8 <= 16` ⇒ `8..8`, which is an empty range that is
  in-bounds for the 8-byte record (this is exactly the fuzz crash `crash-f3aa5f21` that the
  old `16..16` triggered: slicing `bytes[16..16]` out of an 8-byte buffer panics on the
  start index). For `len = 16`: `16 <= 16` ⇒ `16..16` (empty, in-bounds at the very end —
  identical to the old behavior, correct). For `len > 16`: `RECORD_HEADER_LEN..l` (the real
  payload). All three families are correct, and the regression test
  (`src/record.rs:408-418`) plus `decoders_are_total` / the fuzz target lock it in.

## Cleared: loom model faithfully mirrors the real orderings

`tests/loom_ring.rs:50-101` vs `src/ring.rs:144-291`. Orderings match exactly:

| Step | ring.rs | loom_ring.rs |
|---|---|---|
| Producer reads own prod | `Relaxed` (`ring.rs:153`) | `Relaxed` (`loom_ring.rs:52`) |
| Producer reads peer cons | `Acquire` (`ring.rs:154`) | `Acquire` (`loom_ring.rs:53`) |
| Producer publishes prod | `Release` (`ring.rs:174`) | `Release` (`loom_ring.rs:72`) |
| Consumer reads peer prod | `Acquire` (`ring.rs:251`) | `Acquire` (`loom_ring.rs:79`) |
| Consumer reads own cons | `Relaxed` (`ring.rs:252`) | `Relaxed` (`loom_ring.rs:80`) |
| Consumer publishes cons | `Release` (`ring.rs:289`) | `Release` (`loom_ring.rs:99`) |

The placement math (`bytes_needed`, `contiguous_tail`, `free`, `used`) is *the crate's own
pure functions* imported and called by the model (`loom_ring.rs:18,54-55,61,93`), not a
re-implementation, so the wrap/pad arithmetic under loom is literally the production
arithmetic. The model's per-slot `UnsafeCell<u64>` reads/writes stand in for the real
byte copies, and its `try_pop` asserts (no record seen before its bytes, body slots
zero-filled, no wrap) are exactly the properties the release/acquire pairs must guarantee.
This is a legitimate model: the documented reason (`loom` cannot instrument
`core::sync::atomic` over mapped memory) is correct, and the substitution preserves the
ordering edges under test. Both loom tests pass.

## Cleared: `GOLDEN_REGEN` cannot silently mask a failure on the normal path

`golden_fixtures.rs:990-1004`. The regen branch is gated on `GOLDEN_REGEN == "1"` being
present in the environment; on the default test path (no env var) `check` always reads the
checked-in fixture and `assert_eq!`s against it. Regen is opt-in and documented as a
deliberate format-change workflow (module docs, `golden_fixtures.rs:962-965`), and a missing
fixture on the normal path panics loudly (`golden_fixtures.rs:997-998`). No silent-mask path
exists in CI (CI never sets `GOLDEN_REGEN`). See suggestion 02-#1 for a belt-and-suspenders
guard.

## Cleared: workspace gating

`Cargo.toml:858-871`. `tests/vm` is a normal member (so hosted lanes fmt/clippy/build it),
but its only KVM test is `#[ignore]`'d (`tests/vm/src/lib.rs:2161`) AND env-gated
(`vm_tests_enabled()`, `tests/vm/src/lib.rs:2142-2144`), so `cargo test --workspace` skips
it and an accidental `-- --ignored` on a laptop returns early instead of failing. `fuzz/`
is `exclude`d from the workspace and declares its own `[workspace]` root
(`fuzz/Cargo.toml:2058`), so the nightly/libfuzzer dependency never enters
`cargo test --workspace` — confirmed: the workspace test/clippy runs above did not compile
the fuzz crate.

## Cleared: hand-derived spec literals and `.bin` fixtures are byte-correct

I checked the in-source literals in `hand_derived_spec_literals` and `pad_fixtures_byte_exact`
field-by-field against API.md §3.0–§3.2, and dumped a sample of the `.bin` files with `xxd`
(`ready.bin`, `hello.bin`, `frame_mark.bin`, `pad_tail8.bin`, `workload_exited.bin`,
`cmd_quiesce_coop.bin`, `channel_header.bin`, `beacon.bin`, `name_intern_decl.bin`). All
match the spec (e.g. `name_intern_decl.bin`: `len=0x28`, kind=2, flags=0x02 REACHABLE_DECL,
name_id=2, name_len=0x0c for "goal_reached", `_pad`=0; `ready.bin`: len=0x20, kind=14,
unit=0xFFFFFFFF, region_count=0, gen=2). Details in `03-positive-notes.md`.
