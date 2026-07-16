# Positive Notes

## The `payload_range` fix is minimal, correct, and well-documented

`src/record.rs:257-270`. The one-line change (`l..l` instead of
`RECORD_HEADER_LEN..RECORD_HEADER_LEN`) is exactly the right fix for the fuzz crash, and the
doc comment explains *why* an empty range still has to be in-bounds for the record's own
bytes — not just *what* changed. The regression test (`src/record.rs:407-418`) reproduces the
exact fuzz artifact (`crash-f3aa5f21`, the bytes `[0x08, 0x00, ..., 0xFC, 0x0A]`) and asserts
both `payload_range() == 8..8` and that the full `decode_event` path no longer panics. This
is textbook fuzz-finding hygiene: minimal fix, named artifact, pinned regression.

## Hand-derived spec literals check the encoder against the spec text, not its own output

`tests/golden_fixtures.rs:1362-1434` and `1178-1179`. The `hand_derived_spec_literals` test
pins FrameMark, Ready, Hello, and WorkloadExited as in-source literal byte arrays derived by
hand from API.md, and `pad_fixtures_byte_exact` does the same for the 8-byte Pad. This breaks
the circularity of golden-only testing (where `encode(x) == golden` only proves the encoder
agrees with its past self). I verified these literals field-by-field against API.md §3.0–§3.2
and they are correct, including the two's-complement `exit_code = -1` → `0xFFFFFFFF` and
`agent_version` packing `0.1.0` → `0x00000100`. This is the strongest part of the fixture
suite.

## `.bin` fixtures verified byte-correct against the spec via xxd

Sampled with `xxd`: `ready.bin` (`20 00 0e 00 ... ffffffff 00000000 02...`), `hello.bin`
(`... 01000000 00010000 03...`), `frame_mark.bin` (`18 00 0d 00 ...`), `pad_tail8.bin`
(`08 00 00 00 29 00 00 00` — len=8, kind=0, seq=41), `name_intern_decl.bin` (`28 00 02 02 ...`
flags=REACHABLE_DECL, name_len=0x0c="goal_reached"), `cmd_quiesce_coop.bin`
(mode field = 0 COOP), `channel_header.bin` ("DETGUEST", proto 1). Every sampled fixture
matches the spec's field layout exactly.

## Both directions asserted for every fixture

`tests/golden_fixtures.rs:1119-1128` (and the per-test decode-back asserts throughout).
Each fixture is checked as both `encode(x) == fixture` *and* `decode(fixture) == x` with the
header fields (`seq`, `vnanos`, `flags`) verified too. The spec-named edge cases
(truncated AssertViolation → TRUNCATED flag + clipped to `MAX_DETAILS`; `logline_max` at
exactly the cap → *no* TRUNCATED flag) directly test the §3.2 clipping boundary, which is the
easiest place to get an off-by-one wrong.

## Loom model delegates to the crate's real placement math

`tests/loom_ring.rs:18,54-55,61,93`. Rather than re-deriving the wrap/pad arithmetic (which
would risk testing a model that diverges from production), the loom model imports and calls
`bytes_needed`, `contiguous_tail`, `free`, and `used` directly. Combined with the
ordering-table match documented in `01-critical-and-important.md`, this makes the loom test a
genuine check of the production protocol, not a parallel reimplementation. The two scenarios
(3-record stream that forces a tail pad in a 64 B ring; exactly-full ring that unblocks after
a pop) cover the two interleavings that matter.

## Workspace gating is correctly layered and clearly commented

`Cargo.toml:858-871`, `tests/vm/src/lib.rs:1-19`, `fuzz/Cargo.toml:2056-2058`. The
double-gate on the KVM test (`#[ignore]` *and* `DETGUEST_VM_TESTS=1`) means neither
`cargo test --workspace` nor an accidental `-- --ignored` on a non-Intel box runs it, and the
fuzz crate's own `[workspace]` root plus the parent `exclude = ["fuzz"]` keeps the nightly
libfuzzer dependency out of hosted lanes entirely. The comments on each of these explain the
*why* (hosted CI must fmt/clippy `tests/vm`; fuzz must not enter `--workspace`), which is
exactly the kind of rationale that prevents a future contributor from "simplifying" the gate
and breaking CI.

## miri-friendly load shrink is gated, not unconditional

`src/ring.rs:478-483`. `const N: u32 = if cfg!(miri) { 300 } else { 200_000 };` keeps the full
200k-iteration two-thread hammer on normal runs and only shrinks under miri (where the test is
the strongest UB check for the raw-pointer paths but runs orders of magnitude slower). The
comment states exactly that. This preserves coverage where it is cheap and keeps the miri lane
tractable.

## Decoders are demonstrably total

`src/events.rs:510-655,701-759,796-819`. Every fixed-size payload slice is preceded by an
explicit `payload.len() < N` check, and every variable-length field (`name_len`,
`details_len`, `msg_len`) is validated against both its documented cap *and* `payload.len()`
before slicing. The `8 + name_len` style arithmetic cannot overflow because the length is
capped at `MAX_NAME`/`MAX_DETAILS`/`MAX_LOG_MSG` first and `payload.len()` derives from a
u16 record length ≤ 4096. The fuzz target plus `decoders_are_total` lock this property in.
