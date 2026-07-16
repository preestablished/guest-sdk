## Action Items

### Critical
- [ ] None. No correctness-breaking issues were found; the wire format matches API.md §3–§5 and ARCHITECTURE.md §2/§3 byte-for-byte, the SPSC ring discipline is sound, and decoders are total.

### Important
- [ ] [crates/detguest-wire/src/events.rs:837-1078 (and manifest.rs / header.rs / ports.rs tests)] Add byte-exact golden fixtures for every v1 payload as API.md §3.5 (L522) normatively requires — current round-trip-only tests cannot catch a wrong-but-symmetric layout between the independently-compiled guest and host sides. Pin at least one record per ring namespace plus the manifest header/region-entry/extent (see I-1 for a `ready_golden_bytes` template).
- [ ] [crates/detguest-wire/src/header.rs:305-316 ChannelHeader::validate] Reject ring descriptors that overlap each other (or require canonical placement via `id.canonical_desc()`). `RingDesc::validate` currently checks each descriptor in isolation, so a header can alias two SPSC rings onto the same bytes and pass attach validation, breaking the single-owner argument ring.rs depends on.
- [ ] [crates/detguest-wire/src/ring.rs:96,205 from_raw] Promote the `size.is_power_of_two() && size >= MAX_RECORD_LEN` invariant from `debug_assert!` to a real `assert!` (or a fallible safe constructor). It is `debug`-only today and guards pointer/length math that produces `&mut [u8]`, so it is a release-build soundness footgun.

### Suggestions
- [ ] [crates/detguest-wire/src/ring.rs:209] Make the peer index `load`-only by construction (read-only newtype) or add a "peer cell: load-only, never store" comment at each `AtomicU32::from_ptr` site; the no-illegal-store invariant is currently hand-discipline.
- [ ] [crates/detguest-wire/src/ring.rs:77,191] Document that `Producer`/`Consumer` are intentionally `Send` but not `Sync` (move across threads, never share `&self` concurrently), alongside the existing single-producer contract.
- [ ] [crates/detguest-wire/src/manifest.rs:313-333] Stop overloading `EncodeError::FieldTooLong` for seqlock misuse (nested begin / end-without-begin); add a dedicated error variant so logs are not misleading.
- [ ] [crates/detguest-wire/src/ring.rs:31-55] Add direct unit tests for `free`/`used`/`contiguous_tail`/`bytes_needed` at the wrap and ring-end boundaries (empty==size, full==0, aligned tail==size ⇒ no pad, len==tail vs len==tail+8).
- [ ] [crates/detguest-wire/src/lib.rs:41, events.rs:4] Add the `decode_record` fuzz target (or a `proptest` over arbitrary bytes into `decode_event`/`decode_command`/`decode_workload_ctrl`) that the doc comments already promise locks in the no-panic property.
- [ ] [crates/detguest-wire/src/ports.rs:100-107] Add `debug_assert!(arg <= FAULT_ARG_MAX)` in `FaultDecision::pack` so a missed caller-side range-check surfaces in tests instead of silently truncating.
- [ ] [crates/detguest-wire/src/events.rs:291-293] Note on the `EventPayload::Pad` arm that real variable-length tail pads go through `try_push`/`encode_pad` (not `encode_event`, which always emits a fixed 16-byte pad).
