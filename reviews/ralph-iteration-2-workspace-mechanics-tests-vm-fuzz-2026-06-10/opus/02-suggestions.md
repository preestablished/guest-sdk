# Suggestions (non-blocking)

## 1. `arb_event()` and the golden event table omit `Pad` (the "+1" kind)

`tests/proptest_roundtrip.rs:113-133`, `tests/golden_fixtures.rs:1022-1118`.

The prompt frames the kinds as "14+1" (the 14 EventKind variants plus `Pad`). The
proptest `OwnedEvent` enum / `arb_event()` strategy generates exactly the 14 non-Pad
variants; `Pad` (kind 0) is not in the round-trip generator, and it is not in the
`event_fixtures_byte_exact` cases table either. `Pad` *is* covered elsewhere — the
dedicated `pad_fixtures_byte_exact` test (`golden_fixtures.rs:1172-1188`), the
`eight_byte_pad_decodes_as_event_without_panicking` regression
(`src/record.rs:407-418`), and `decoders_are_total` — so totality and the edge fixture
are not at risk. But the *round-trip* property `decode(encode(x)) == x` is never asserted
for `Pad` through the generator, and the 16-byte (full-header) `Pad` round-trip isn't
property-tested at all (only the 8-byte and 40-byte fixtures exist). Consider adding a
`Pad` arm to the round-trip coverage so the "+1" is explicit. Because `Pad` carries no
payload and an 8-byte Pad has no `vnanos`, a small dedicated proptest is clearer than
shoehorning it into `arb_event()`:

```rust
proptest! {
    #[test]
    fn pad_round_trip(seq in any::<u32>(), full in any::<bool>()) {
        let len = if full { 16 } else { 8 };
        let mut buf = [0xAAu8; 16];
        let n = detguest_wire::record::encode_pad(&mut buf, len, seq).unwrap();
        let (hdr, ev) = decode_event(&buf[..n]).unwrap();
        prop_assert_eq!(ev, EventPayload::Pad);
        prop_assert_eq!(hdr.seq, seq);
        prop_assert_eq!(hdr.len as usize, len);
    }
}
```

## 2. `command_round_trip` `pick in 0u8..6` is brittle if a CommandKind is added

`tests/proptest_roundtrip.rs:154,162-174`. The strategy picks one of 6 commands by a magic
`0u8..6` range and a fall-through `_ => Command::ReverifyRegions`. If a 7th command is added
later, the range silently keeps testing only the first 6 and the new variant is never
round-tripped — a wrong-but-symmetric layout in it would pass CI. A `#[non_exhaustive]`-style
exhaustiveness guard isn't available for a numeric pick, but a comment tying `0u8..6` to the
CommandKind count, or driving the count from a `const`, makes the omission loud at review
time. Same pattern applies, less severely, to the boolean-driven `workload_ctrl_round_trip`
(only 2 variants, lower risk).

## 3. `decoders_are_total` upper bound (5000) is below `MAX_RECORD_LEN`-adjacent edge sizes

`tests/proptest_roundtrip.rs:241`. The byte vector is `0..5000`. `MAX_RECORD_LEN` is 4096,
so this does cover the boundary, but the fuzz target (`fuzz/fuzz_targets/decode_record.rs`)
has no explicit length cap and will explore both shorter and longer inputs. The proptest
bound is fine for the in-suite property; just noting that the two harnesses intentionally
differ in reach — no change required, but a one-line comment that 5000 deliberately straddles
4096 would document intent.

## 4. Fuzz target's `RegionEntry`/`Extent` loop uses fixed `0..4`

`fuzz/fuzz_targets/decode_record.rs:2087-2090`. The slot index is swept `0..4` for both
`RegionEntry::read_from` and `Extent::read_from`. That covers the bounds-check entry path,
but the interesting boundary is the *last valid slot* (region slot 63, extent slot 1023) and
the *first out-of-range slot* (64, 1024), which are what would catch an off-by-one in the
slot-offset math. Consider sweeping a couple of high/edge slots too (e.g. `[0, 63, 64, 1023,
1024]`), since the manifest parsers index by slot and the fuzz target is the totality gate
for them.

## 5. `kvm_available()` opens `/dev/kvm` read+write but never uses the handle

`tests/vm/src/lib.rs:2147-2153`. The probe opens with `.read(true).write(true)` purely to
test accessibility, then drops the `File`. That's fine and even appropriate (KVM needs RW),
but a short comment ("RW because KVM_CREATE_VM needs a writable fd; we only probe
openability") would pre-empt a future reader wondering why a probe takes a write handle.
Minor.

## 6. `manifest_area.bin` is a 22 KiB checked-in binary fixture

`crates/detguest-wire/tests/golden/manifest_area.bin` (22560 bytes). This is by far the
largest fixture and is mostly zeroed manifest area. It is correct and the byte-exact pin is
valuable (it catches header-count / entry-offset drift), but it dominates the fixture
directory's size. If repo size becomes a concern later, the manifest golden could be pinned
as a hash plus a few spot-checked offsets rather than the full 22 KiB blob. Not worth doing
now — flagging only so the trade-off is on record.
