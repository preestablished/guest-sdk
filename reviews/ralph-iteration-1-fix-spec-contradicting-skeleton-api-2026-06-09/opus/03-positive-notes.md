# Positive Notes

Patterns worth preserving as the SDK / agent / host crates fill in around this wire layer.

### Compile-time layout pinning catches spec drift before runtime

`crates/detguest-wire/src/header.rs:109-127` and `crates/detguest-wire/src/manifest.rs:55-61`
use `const { assert!(...) }` blocks to lock the channel and manifest offsets to the
`ARCHITECTURE.md` §2 / `API.md` §4 numbers — ring areas pack back-to-back, index cells
are 64-byte separated, the manifest fits `0x1000..0x8000`, and the absolute offsets
`0x1020`/`0x2820`/`0x6820` are asserted directly. Layout drift fails *compilation*, not
a test run. This is the right place to enforce a byte contract.

### The unsafe module is genuinely minimal and its soundness argument is explicit

`crates/detguest-wire/src/ring.rs:16-23` states the disjoint-ownership argument
(`producer owns [prod, cons+size)`, `consumer owns [cons, prod)`, ownership transferred
only through release/acquire on the index cells — "the same split-borrow reasoning as
`split_at_mut`"). Every `unsafe` block (`:178`, `:273`) has a `// SAFETY:` comment that
discharges the obligation by pointing at that argument rather than restating the code.
`unsafe fn from_raw` (`:89`, `:199`) carries a full caller-contract list (validity,
power-of-two size, one-half-per-ring SPSC contract, seq continuation). This is exactly
the structure the unsafe-review research note prescribes.

### Memory-ordering discipline is correct on both halves

`crates/detguest-wire/src/ring.rs:142-143` / `:163-164` (producer) and `:230-231` /
`:264-265` (consumer): each side `Relaxed`-loads its own index and `Acquire`-loads the
peer's, and publishes with a single `Release` store that covers both the pad and the
record together (`:163`). This matches the spsc-ring research rule table line-for-line,
and the `two_thread_smoke` test (`:429`) exercises it from a real second thread.

### Decoders are total and copy-before-validate

The `decode_event` / `decode_command` / `decode_workload_ctrl` paths
(`crates/detguest-wire/src/events.rs:502-811`) bounds-check every length field against
both its documented cap and `payload.len()` before slicing (e.g. `NameIntern` at
`:522-524`, `AssertViolation` at `:535-536`, `LogLine` at `:609-610`). `pop_into`
(`crates/detguest-wire/src/ring.rs:243-263`) copies the bytes out of the ring and
validates from the local `scratch` copy, closing the TOCTOU window the research note
flags. `decoder_never_reads_past_declared_lengths` (`events.rs:1060`) locks the forged-
length behaviour in.

### The deliberate `RING_W_SIZE` deviation is reasoned, not hand-waved

`crates/detguest-wire/src/header.rs:92-103` does not silently diverge from the spec — it
states *why* `0x1E0000` is unusable (non-power-of-two breaks the free-running `u32`
index discipline and the attach validation, both of which the same `ARCHITECTURE.md` §2
requires), picks the largest power of two that fits the documented area, and flags it as
a spec-documentation issue to reconcile. I verified both dependency claims against the
spec; the reasoning holds.

### `Pad`-consumes-a-seq invariant is encoded and tested

`crates/detguest-wire/src/ring.rs:153` allocates a seq for the tail pad *before* the
record's seq, and `wrap_inserts_pad_and_seq_stays_monotonic` (`:347`) asserts the
consumer sees a strictly monotonic seq stream across wraps (`expected_seq > pushed_frames`
proves a pad actually occurred). This keeps per-ring `seq` gap-free through tail pads,
matching the framing note in `record.rs:22-23`.

### Spec-correctness fix in the agent skeleton is asserted at the byte level

`crates/detguest-agent/src/lib.rs:49-67` (`ready_record_is_spec_correct`) explicitly
asserts the new `Ready` record is the 32-byte, kind-14 form (`EventKind::Ready as u8`,
not the old ad-hoc 9-byte `READY_RECORD=1`) and round-trips the payload. The test name
and inline comments document precisely the spec-contradiction this branch set out to
fix.
