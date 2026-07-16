## Action Items

### Critical
- [ ] None.

### Important
- [ ] None.

### Suggestions
- [ ] [tests/proptest_roundtrip.rs:113-133] Add `Pad` (the "+1" kind) to round-trip property coverage — `arb_event()`/`OwnedEvent` generate only the 14 non-Pad kinds; add a small dedicated `pad_round_trip` proptest covering both the 8-byte and 16-byte Pad so `decode(encode(x)) == x` is asserted for Pad through a generator, not only via the static edge fixtures.
- [ ] [tests/proptest_roundtrip.rs:154] Tie `command_round_trip`'s magic `pick in 0u8..6` to the CommandKind count (comment or `const`) so a future 7th command can't be silently left untested by the fall-through `_ => ReverifyRegions`.
- [ ] [tests/proptest_roundtrip.rs:241] Add a one-line comment noting the `0..5000` byte bound deliberately straddles `MAX_RECORD_LEN` (4096) for `decoders_are_total`.
- [ ] [fuzz/fuzz_targets/decode_record.rs:2087-2090] Sweep edge slot indices (e.g. 0, 63, 64, 1023, 1024) for `RegionEntry::read_from`/`Extent::read_from` instead of only `0..4`, since the last-valid / first-invalid slot is where a slot-offset off-by-one would surface.
- [ ] [tests/vm/src/lib.rs:2147-2153] Add a one-line comment on `kvm_available()` explaining the read+write open is an accessibility probe (KVM needs a writable fd), since the handle is opened and immediately dropped.
- [ ] [crates/detguest-wire/tests/golden/manifest_area.bin] (record-only) The 22 KiB manifest golden is the largest fixture and mostly zeros; if repo size ever matters, consider pinning it as a hash + spot-checked offsets. No action needed now.
