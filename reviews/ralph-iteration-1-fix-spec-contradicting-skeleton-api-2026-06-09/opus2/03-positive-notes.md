# Positive Notes — patterns worth preserving

### P-1. The ring-W deviation is the gold standard for documenting a deliberate spec divergence
`header.rs:92–103` (`RING_W_SIZE`). Rather than silently sizing W at 1 MiB, the
doc comment lays out the actual contradiction in `ARCHITECTURE.md` §2 (a
non-power-of-two 0x1E0000 size vs the same section's free-running-u32 +
power-of-two masking discipline, and `API.md` §2's attach validation that rejects
non-power-of-two descriptors), picks the requirement that *both validation paths
depend on*, and records it as a spec documentation issue. This is exactly how a
known divergence should be captured. The reasoning is correct: a free-running u32
index only stays consistent across the 2^32 wrap when `size` divides 2^32, which a
power of two does and 0x1E0000 does not.

### P-2. Compile-time layout invariants make wire drift a build failure
`header.rs:110–127` and `manifest.rs:56–61` use `const { assert!(...) }` blocks to
pin ring packing (C|I|A|W back-to-back, all power-of-two, manifest fits its area)
and the absolute manifest offsets (`0x1020`, `0x2820`, `0x6820`) straight from
`API.md` §4.1. Layout drift cannot compile, which is far stronger than a runtime
test. Preserve and extend this pattern as new structures land.

### P-3. Decoders are total and bounds-checked, with a test that proves it
Every variable-length decode in `events.rs` validates the embedded length against
both its documented cap **and** the actual payload length before slicing — e.g.
`NameIntern` (`events.rs:522–524`), `AssertViolation` (`535–536`),
`LogLine` (`609–610`). The `decoder_never_reads_past_declared_lengths` test
(`events.rs:1060–1077`) forges a `name_len` of 255 and asserts `BadField` rather
than a panic. This is precisely the "every slice index dominated by a bounds
check" discipline the `rust-nostd-wire-codecs` research calls for.

### P-4. Padding bytes are always zeroed before payload writes
`encode_with` (`events.rs:272`) and `encode_pad` (`record.rs:286`) both
`fill(0)` the full record region before writing fields, so every `_pad`/`_pad2`
slot and every trailing alignment byte goes out as zero — closing the
"uninitialized padding leaks memory onto the wire" pitfall flagged in the research
note. This also makes the byte stream deterministic regardless of buffer reuse.

### P-5. Correct, minimal, well-argued SPSC memory ordering
`ring.rs` follows the canonical discipline exactly: producer Relaxed-loads its own
`prod` and Acquire-loads the peer `cons` (`L142–143`), then a single Release store
publishes pad+record together (`L163–164`); consumer mirrors it (`L230–231,
264–265`). The module-level soundness argument (`L17–22`) frames the split as the
same disjoint-`&mut` reasoning as `split_at_mut` with atomics supplying the
happens-before edges — which is the right mental model. `two_thread_smoke`
(`L429–463`) exercises it across a real thread, and `free_running_indices_survive_u32_wrap`
(`L410–427`) deliberately pre-winds the indices to just below `u32::MAX` and
asserts they wrap cleanly.

### P-6. Consumer copies bytes out before trusting the length, defeating in-ring TOCTOU
`pop_into` (`ring.rs:243–263`) peeks the 8-byte header prefix into local `scratch`,
derives `len` from the **local copy**, range-checks it against `avail`/`tail`, then
copies the full record out and re-validates the header via
`RecordHeader::read_from`. It never re-reads the length field from shared ring
memory after validating it — exactly the defense against the "validate then read a
field a concurrent writer can change" TOCTOU pitfall in the
`spsc-ring-memory-ordering` research. (In this design the producer can't overwrite
unconsumed bytes, but the copy-first pattern is correct regardless and survives a
future relaxation.)

### P-7. The agent skeleton's `Ready` helper and its test encode the actual fix
`detguest-agent/src/lib.rs:26–67`. The whole point of this iteration is replacing
an ad-hoc 9-byte READY encoding with the spec's 16-byte `Ready` payload (kind 14),
and the test `ready_record_is_spec_correct` pins `n == 32`, `buf[2] == EventKind::Ready as u8`
(asserting "kind 14, not READY_RECORD=1"), and decodes the round-trip. The
comment trail makes the regression it guards against unmistakable.

### P-8. Module-scoped unsafe policy is deliberate and explained
`lib.rs:14–17` and `detguest-agent/src/lib.rs:8–14` use crate-level
`#![deny(unsafe_code)]` with a per-module `#![allow(unsafe_code)]` only in `ring`,
and explicitly note that `forbid` was rejected because it would make the permitted
module unwritable. Both `unsafe fn`s carry full `# Safety` contracts, and both
`unsafe impl Send` blocks carry a one-line justification — matching the
`rust-unsafe-review` checklist.

### P-9. FaultDecision packing matches the spec's golden values exactly
`ports.rs:96–121` packs `kind | (arg << 8)` and the `pack_golden_values` test
(`128–143`) pins `Platform{2,512} == 0x0002_0002` and `Workload{200,0xFFFFFF} ==
0xFFFF_FFC8`, which agree with `API.md` §1.4/§5 (bits 0–7 kind, bits 8–31 arg,
0 = Proceed). `unpack` is total over all u32 and `kind_zero_is_always_proceed`
pins the RAZ-noise behavior. Clean, complete, spec-faithful.
