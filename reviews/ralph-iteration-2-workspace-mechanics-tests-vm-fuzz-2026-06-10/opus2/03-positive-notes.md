# Positive Notes

## The bug fix is exemplary in how it was found and pinned

`crates/detguest-wire/src/record.rs:257-270` + the regression test at `:407-418`. The fix is one
line of real logic (`if l <= RECORD_HEADER_LEN { l..l }`), but the doc comment explains *why* the
empty range must sit at `len` not at `RECORD_HEADER_LEN`, names the fuzz artifact
(`crash-f3aa5f21`), and the regression test reconstructs the exact 8-byte input that triggered the
panic and asserts both `payload_range() == 8..8` *and* that `decode_event` returns
`EventPayload::Pad` without panicking. This is the textbook fuzz-to-regression workflow.

## Spec-anchored golden literals, not just self-consistent goldens

`golden_fixtures.rs:410-482` (`hand_derived_spec_literals`). The byte arrays for `FrameMark`,
`Ready`, `Hello`, and `WorkloadExited` are hand-derived from API.md §3.0-§3.2 with field-by-field
comments, so the encoder is checked against the *spec text*, not merely against its own past
output. This defends against the classic golden-test failure mode where a wrong encoder and a
wrong fixture agree with each other. The `WorkloadExited` case specifically pins two's-complement
`-1` as `0xFFFFFFFF`, which is exactly the kind of sign-handling a self-referential golden would
miss.

## The double-gate on the VM tier is correct and actually tested

`tests/vm/src/lib.rs:39-50`. The `#[ignore]` + `DETGUEST_VM_TESTS=1` env check is belt-and-
suspenders: `cargo test --workspace` never runs it, and an accidental `-- --ignored` on a laptop
fails *soft* (prints the skip, returns). I verified this empirically:
`cargo test -p detguest-vmtest -- --ignored` with no env var passes. The `wire_crate_links` test
(`:53-56`) runs everywhere and gives a permanent signal that the harness still links the wire
crate. The comment block documenting the exact Intel-runner invocation is the right place for it.

## Workspace boundary for the fuzz crate is drawn correctly

`Cargo.toml:13` (`exclude = ["fuzz"]`) plus `fuzz/Cargo.toml`'s own `[workspace]` table making it
a standalone root. This keeps the nightly-only `libfuzzer-sys` dependency out of the hosted
`cargo test --workspace` lanes entirely, while `tests/vm` stays a *normal* member so it gets
built/formatted/linted everywhere. The rationale is spelled out inline in both files. `clippy
--workspace --all-targets` is clean, confirming the boundary holds.

## Loom model delegates placement to the real `ring` math

`loom_ring.rs:51-101`. Rather than re-implementing the wrap/pad arithmetic (which would let the
model and the real code drift), `try_push`/`try_pop` call the crate's own pure functions
(`used`, `free`, `contiguous_tail`, `bytes_needed`). So the *placement logic* under loom is
literally the logic the real halves execute; only the atomic cells (which loom must own) are
modeled. The single-Release-store-publishes-pad-and-record shape (`:62-73`) faithfully mirrors
`ring.rs:166-175`. This is the right way to keep a model honest.

## `unexpected_cfgs` lint is configured for the `loom` cfg

`crates/detguest-wire/Cargo.toml` `[lints.rust]` with
`check-cfg = ["cfg(loom)"]`. This is easy to forget and would otherwise spew `unexpected cfg`
warnings (or, post-1.80, be a hard error) for the `#![cfg(loom)]` test gate. Getting it right shows
attention to the toolchain detail.

## Decoder-totality property kept in the default suite, not only in fuzz

`proptest_roundtrip.rs:240-248` (`decoders_are_total`). Mirroring the fuzz target's "arbitrary
bytes never panic any decoder" property as a proptest keeps that guarantee enforced on every
`cargo test` run, even where nightly+libfuzzer isn't available. The comment correctly notes the
fuzz target "hammers this harder" — the right framing of belt-and-suspenders coverage.
