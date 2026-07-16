# Critical and Important Issues

## Critical

**None.** I specifically scrutinized for: UB/soundness holes in `ring.rs`, spec
violations producing wrong bytes, decoder panics on arbitrary bytes, and data
corruption. None were found.

- The ring's raw-pointer slices are backed by a correct disjoint-region argument
  (producer owns `[prod, cons+size)`, consumer owns `[cons, prod)`), the
  acquire/release pairs on the index cells are exactly right (producer:
  `Relaxed` own-index load, `Acquire` peer load, `Release` publish; consumer:
  symmetric), and the `u32` free-running index math (`used = prod.wrapping_sub(cons)`,
  power-of-two mask) is wrap-safe — exercised by `free_running_indices_survive_u32_wrap`.
- The `pop_into` TOCTOU surface is handled per the research note: the variable-length
  bytes are copied out of the ring (`copy_out`) and the header is validated from the
  *local* `scratch` copy, never re-read from shared memory.
- All event/command/workload-ctrl decoders bounds-check every variable-length field
  (`name_len`/`details_len`/`msg_len`) against both its documented cap and the actual
  payload length before slicing, so forged lengths yield `Err`, not a panic or an
  out-of-bounds read (`decoder_never_reads_past_declared_lengths` covers this).
- Byte layouts match the specs exactly (offsets, sizes, kind numbers, flag bits,
  and the `u32`-vs-`u64` `manifest_generation` distinction between `RegionRegister`
  and `Ready`). The deliberate `RING_W_SIZE` deviation is justified and documented.

## Important

### IMP-1 — `Producer::slice_mut` returns `&mut [u8]` from `&self` (mut_from_ref); breaks clippy-gated CI and removes the borrow-checker aliasing net in the only unsafe module

- **Severity:** Important
- **File:** `crates/detguest-wire/src/ring.rs:174-179` (and its two call sites at
  `:154` and `:160` inside `try_push`)

```rust
fn slice_mut(&self, off: u32, n: usize) -> &mut [u8] {
    debug_assert!(off as usize + n <= self.size as usize);
    // SAFETY: range lies inside the free region exclusively owned by this
    // producer (checked against cons above); see module-level argument.
    unsafe { core::slice::from_raw_parts_mut(self.data.add(off as usize), n) }
}
```

`clippy::mut_from_ref` is **deny-by-default**, so `cargo clippy --workspace` currently
fails to compile `detguest-wire`:

```
error: mutable borrow from immutable input(s)
   --> crates/detguest-wire/src/ring.rs:174:48
```

Two problems:

1. **CI breakage.** Any pipeline that runs clippy (the IMPLEMENTATION-PLAN M-series
   strongly implies a lint gate, and `lib.rs` already opts into `#![deny(missing_docs)]`
   / `#![deny(unsafe_code)]` discipline) fails here. This is the only thing standing
   between the branch and a green clippy run.
2. **Latent aliasing footgun.** Producing a `&mut [u8]` from `&self` means the borrow
   checker will *not* stop a future edit from holding two live `&mut` slices over the
   ring simultaneously. Today `try_push` is safe because the pad slice (`:154`) is
   fully consumed by `encode_pad` before the record slice (`:160`) is taken, and the
   two ranges are disjoint anyway (pad covers `[off, ring_end)`, record covers
   `[0, total_len)`). But that safety rests entirely on call-site discipline, not on
   the type system — exactly the "unsafe abstraction whose API can be misused" pattern
   the unsafe-review research note warns against.

**Suggested fix** — take `&mut self`; `try_push` already has `&mut self`, and the two
calls don't overlap in NLL lifetime (the first `dst` is last-used in `encode_pad`):

```rust
fn slice_mut(&mut self, off: u32, n: usize) -> &mut [u8] {
    debug_assert!(off as usize + n <= self.size as usize);
    // SAFETY: range lies inside the free region exclusively owned by this
    // producer (checked against cons above); see module-level argument.
    unsafe { core::slice::from_raw_parts_mut(self.data.add(off as usize), n) }
}
```

No other change is required: the borrow of the pad `dst` ends at the `encode_pad`
call, so the subsequent `self.slice_mut(...)` for the record borrows `&mut self`
freely. This satisfies clippy and restores the borrow-checker's guarantee that the two
in-flight slices cannot illegally coexist. Re-run `cargo clippy --workspace
--all-targets` to confirm the error clears (only the separate `manual_range_contains`
warning — see suggestions — should remain).
