# Suggestions (non-blocking)

### S-1. `Consumer` holds a mutable `AtomicU32` over the producer index it must never store to
**File:** `crates/detguest-wire/src/ring.rs:209` (`AtomicU32::from_ptr(prod as *mut u32)`)

The consumer casts the `*const u32` producer cell to `*mut` to build an
`AtomicU32`, even though it only ever `load`s it. The reverse (`Producer` over
`cons`) is the same. This is sound today — nothing calls `store` on the peer
cell — but it relies on hand-discipline rather than the type system, and a future
edit to `pop_into`/`try_push` could add an illegal store with no compiler pushback.
Consider wrapping the peer index in a read-only newtype, or at least add a one-line
comment at each `from_ptr` site stating "peer cell: load-only, never store" so the
invariant is local to the unsafe construction.

### S-2. `Producer`/`Consumer` are `Send` but not `!Sync`-documented; no guard against two producers
**File:** `crates/detguest-wire/src/ring.rs:77, 191` (`unsafe impl Send`)

The SPSC contract ("at most one `Producer` per ring") is enforced only by the
`from_raw` doc-comment. Because `from_raw` is `unsafe`, that is acceptable, but a
short note that the type is intentionally *not* `Sync` (you can move it across
threads but not share `&Producer` to call `try_push` concurrently) would make the
threading model explicit. The `unsafe impl Send` blocks each carry a one-line
justification — good — but neither mentions the absence of `Sync`.

### S-3. `writer_begin`/`writer_end` overload `EncodeError` for protocol misuse
**File:** `crates/detguest-wire/src/manifest.rs:313–333`

A nested `writer_begin` (generation already odd) returns
`EncodeError::FieldTooLong`, and `writer_end` without a matching begin returns the
same. The comments say "nested begin — agent bug" / "end without begin — agent
bug," which is the right intent, but `FieldTooLong` is semantically unrelated and
will read confusingly in logs/panics. Consider a dedicated variant
(e.g. `EncodeError::SeqlockState`) or a separate `Result` type for the seqlock
helpers, so a misuse here is not mistaken for a too-long field.

### S-4. `bytes_needed` / `free` / `contiguous_tail` deserve unit tests at the wrap boundary
**File:** `crates/detguest-wire/src/ring.rs:31–55`

The free-running index math is exercised end-to-end by `two_thread_smoke` and
`free_running_indices_survive_u32_wrap`, which is excellent, but the pure functions
have no direct unit tests. A few `const`-friendly assertions pinning the tricky
cases would document intent and lock the math: `contiguous_tail(0, 4096) == 4096`
(aligned ⇒ full tail, no pad), `bytes_needed(prod, size, len)` when `len == tail`
(no pad) vs `len == tail + 8` (pad), and `free(prod, prod, size) == size` (empty)
vs `free(prod.wrapping_add(size), prod, size) == 0` (full). These also serve as
executable documentation of *why* a pad is never inserted at an aligned position.

### S-5. No `decode_record` fuzz target yet, though the code repeatedly promises one
**Files:** `lib.rs:41`, `events.rs:4`, plus several decoder doc-comments

`DecodeError`'s doc says the no-panic property is "locked in by the `decode_record`
fuzz target," and `events.rs` repeats it. No fuzz target exists in the tree yet.
The decoders *look* total (I traced every slice index in `events.rs` and
`record.rs` back to a dominating length check, per the `rust-nostd-wire-codecs`
guidance), and `decoder_never_reads_past_declared_lengths` is a good targeted test
— but the promised `cargo fuzz` target (or at least a `proptest` over arbitrary
byte slices into `decode_event`/`decode_command`/`decode_workload_ctrl`) is the
thing that actually guarantees it. File it as follow-up work for this crate, not a
later milestone, since the property is asserted *now* in the doc comments.

### S-6. `FaultDecision::pack` silently truncates over-range args; only a doc warns
**File:** `crates/detguest-wire/src/ports.rs:100–107`

`pack` masks `arg & FAULT_ARG_MAX` and the doc says callers "must range-check
first (the host crate does)." Since this is a `const fn` on a `pub` type that any
caller can reach, a `debug_assert!(arg <= FAULT_ARG_MAX)` inside the non-`Proceed`
arm would catch a missed range-check in tests/debug without changing the release
ABI. Minor, but cheap insurance for a value that rides the deterministic inject
path.

### S-7. `record.rs` module doc claims a `Pad` may be 8 bytes but `encode_event(Pad)` always emits 16
**File:** `crates/detguest-wire/src/events.rs:291–293` vs `record.rs:268–289`

`encode_event` for `EventPayload::Pad` calls `encode_pad(buf, RECORD_HEADER_LEN,
seq)` — i.e. a fixed 16-byte pad — while `encode_pad` itself (and the ring's
`try_push` tail-pad path) correctly supports the 8-byte minimal pad. This is fine
(the typed `EventPayload::Pad` is a convenience and 16 is a valid pad length), but
a one-line note on the `EventPayload::Pad` arm clarifying that real tail pads go
through `try_push`/`encode_pad` with their exact tail length — not through
`encode_event` — would prevent a future caller from assuming `encode_event(Pad)`
produces a minimal tail filler.
