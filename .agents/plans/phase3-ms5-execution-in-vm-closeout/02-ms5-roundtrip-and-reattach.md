# Package 02 — Ms5 Live Inject Round Trip And Restore Continuity

Covers `guest-sdk-m5-channel-reattach-checkpoint` and
`guest-sdk-m5-vm-inject-roundtrip`. Claim the former first; closing it should
make the latter ready.

## Workload fixture

Extend `tests/vm/workloads/src/bin/testload.rs` or add a purpose-built Ms5
workload binary using the existing workload build/image plumbing. The fixture
must call `detguest_sdk::inject_point` at stable named points on multiple
frames, retain each returned `FaultDecision`, and expose deterministic
behavior showing both Platform and Workload decisions were observed. Use a
versioned canonical LogLine schema; no generic structured inject-result event
exists. Since `inject_point` does not expose iseq, correlate host iseq/name_id
to workload point name and local occurrence in query order.

Keep input bursts active in the same trajectory so inject and pv-pad paths are
not separately trivial. Pin the call order; the same seed must produce the
same `(iseq, name_id)` query sequence.

Keep artifact roles explicit: the guest-sdk-built fixture image proves the new
inject API path. The pinned reference-workload image proves real-workload Ms3
and replay compatibility only if it was rebuilt with the implementing
guest-sdk SHA and inspection confirms it contains the required call sites.

## Harness round trip

In `tests/vm/src/harness/pio.rs` and a focused test under `tests/vm/tests/`:

1. Boot the real Linux guest/workload through detguest-agent and attach the
   real `detguest-host::Channel`.
2. Assign at least one deterministic known point to each of Proceed, Platform,
   and Workload; seed-vary other arguments/order without probabilistic class
   coverage.
3. For every `OUT PORT_INJECT`, assert the harness first drains the matching
   ring-W `InjectQuery`, answers the same iseq, stores the packed IN result,
   and emits exactly one `SinkOp::PioAnswer`/decision record.
4. Correlate host records to workload-observed returns. Fail with serial,
   decoded event trail, pending inject map, and sink trace.
5. Feed the decoded recorded decisions into `LogFaultPlan`; run the replay leg
   with the synthesizer absent and require identical decisions plus zero log
   divergences. DHILOG byte decoding remains hypervisor-owned, so use its
   decoded-record boundary established in package 01 rather than duplicating
   the file format in guest-sdk.

`VmHarness` is currently hard-wired to `InjectResponder<TableFaultPlan>` and
snapshot restore rebuilds that concrete type. Add an explicit plan-mode seam:
prefer a small enum wrapping Table/Log plans (or make the harness generic if
simpler). Tests must recover LogFaultPlan divergence and remaining-record state.

Add the neutral decoded PAD_SET adapter shared with package 03 here so the Ms5
seeded input-burst trajectory does not depend on closing either Ms3 bead.

This replaces the scaffold's inject and ingestion stubs with real helpers;
remove the `DETGUEST_REPLAY_EXERCISE_STUBS` escape hatch for those paths so the
gated test cannot silently skip them.

## Snapshot/reattach proof

Take a quiet-boundary root snapshot after at least one completed inject query and before later repeated
points. Restore two or more children with the channel base, producer sequence
checkpoint, A/W consumer state, intern table, and pending inject map restored
through existing harness APIs. Prove:

- child records continue sequence numbers without reset, duplicate, or gap;
- completed queries are not answered twice and later queries are not lost;
- identical restored children receive identical decisions at identical
  sequence points; and
- channel mutation traces replay to byte-identical state.

The synchronous PIO handler drains and answers before returning, so live VM
snapshots cannot observe a drained-but-unanswered query. Test pending-inject
map checkpoint/restore separately in a host-only Channel test; do not add a
mid-exit snapshot hook unless production behavior needs one.

Include a negative that intentionally corrupts one restored sequence/checkpoint
field and demonstrates a named mismatch rather than accidental equality.

## Verification and closure

Fixture/round-trip plumbing may precede the reattach assertion; bead closure
still follows dependency order. Run host/unit tests first, then the focused ignored KVM tests with
`DETGUEST_VM_TESTS=1` and `--test-threads=1`. Close
`m5-channel-reattach-checkpoint` only after restore continuity passes, then
close `m5-vm-inject-roundtrip` after the live + log-backed round trip passes.
Evidence must name the image/revisions from package 01 and include a decoded
query/answer table.
