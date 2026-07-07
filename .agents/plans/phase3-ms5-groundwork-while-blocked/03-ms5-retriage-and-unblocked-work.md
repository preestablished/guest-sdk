# Package 03 — Re-Triage The Ms5 Chain; Land The Genuinely-Unblocked Work

Covers request item 3. Outcome: a bead graph where "blocked" is
load-bearing — every blocked Ms5 bead cites the specific thing it waits
on (a checklist item ID from package 01, a sibling bead, or round-2
scope), and every bead the triage unblocks has its work landed.

## The triage table

Dispositions, from the dep graph as verified 2026-07-07 (re-verify with
`bd show` at execution time; package 01's flips change two edges):

| Bead | Live deps | Disposition |
|---|---|---|
| `m5-sdk-inject-point` (P1) | only `m4-platform-readability-vm` — **closed** | **Unblock, implement now** (§A). BLOCKED was a blanket 2026-06-18 stamp; acceptance is pure unit tests. |
| `m5-host-mutation-log-audit` (P1) | only `m4-host-read-region-restore-tests` — **closed** | **Unblock, implement now** (§C). Host-only tests over the existing sink. |
| `m5-host-log-fault-plan` (P1) | `ext-hyp-input-log-dev-events` (flips in package 01) + closed M4 bead | **Unblock after the flip, implement now** (§B) against synthetic fixtures — its own description scopes DHILOG serialization out. If package 01 withheld the flip, annotate with the failed `ILDE-*` item and skip §B. |
| `m5-channel-reattach-checkpoint` (P1) | `m5-host-mutation-log-audit` | **Unblock when §C closes; annotate as started-via-4bc.** Package 02 delivers its "intern table and pending inject state reconstruction or checkpointing" clause; producer-seq checkpoint/restore already exists (`channel.rs:219-230`, exercised by `m4_snapshot.rs`). The remaining clause — restored branches continue sequences without duplicate records, verified in-VM — is round-2 scope. Annotate the bead with exactly this split; leave open. |
| `m5-vm-inject-roundtrip` (P1) | the three above | Stays blocked — correctly. Annotate: "in-VM leg; round-2 (`phase3-ms5-execution-in-vm-closeout` item 1); deps are the real gate, no external checklist item." |
| `m5-determinism-replay-ci-gate` (P0) | `ext-hyp-determinism-replay-linux` (flips in package 01) + `m5-vm-inject-roundtrip` | Stays blocked on the roundtrip bead. Annotate: "scaffold landed (package 04); remaining: fill stubs + 1000-iteration run, round-2 item 2." |
| `m3m5-ci-intel-vm-lanes` (P0) | transitive | Stays blocked. Annotate: round-2 item 4. |

Annotations via `bd update <id> --notes="..."` (append-style: re-read
existing NOTES with `bd show` first and preserve them — `--notes`
replaces the field). Never `bd edit`.

Annotation wording matters for verification: the phases track will
read the graph as "every blocked bead cites a checklist item"
(`03-verification-offer.md` step 2). For beads blocked on **sibling
beads** rather than external checklist items (the roundtrip, CI-gate,
and lane beads), phrase the note so both are visible: the blocking
sibling/round-2 item first, and the checklist items that ground the
eventual work (e.g. ILDE-6 for the roundtrip) second — so a literal
read of the verification step still finds a citation on every blocked
bead.

Beads outside the Ms5 chain (`m3-*`, `m4-*` stragglers, docs, epics)
are **not** re-triaged here — the request scopes item 3 to the Ms5
chain, and round 2's item 5 owns the full-tail enumeration.

## §A — `m5-sdk-inject-point`: the OUT/IN detcall mechanics

Claim the bead; its description is the spec: "allocate iseq, intern
the point name, emit critical InjectQuery on ring W, OUT then IN on
PORT_INJECT, decode FaultDecision, and return Proceed in standalone or
error cases." File reservations per the bead:
`crates/detguest-sdk/src/inject.rs`, `pio.rs`, `lib.rs`.

Current stub (`crates/detguest-sdk/src/inject.rs`, whole file):
validates the name, returns `Proceed`, emits nothing.

Build order:

1. **`pio.rs`: add the IN helper.** Only `detcall_out` exists
   (`out dx, eax`). Add `detcall_in(port: u16) -> u32` (`in eax, dx`),
   mirroring `detcall_out`'s cfg structure (`not(test)` +
   `target_arch` gates, plus the not-any-arch stub). Be aware there is
   **no existing scriptable PIO mock to extend**: the `#[cfg(test)]`
   `doorbell_w` is a bare no-op that records nothing. The ordering and
   decode tests below therefore need net-new `#[cfg(test)]` mock
   infrastructure — e.g. a thread-local recorder of OUT/doorbell
   operations plus a scriptable answer queue for IN — designed in this
   package. Budget for it; it is the bulk of §A's test-side work.
2. **iseq allocation.** A monotonically increasing u32 in SDK state
   (`with_sdk_state`), starting at a documented origin. The host folds
   `pending_injects` as iseq → name_id from drained `InjectQuery`
   events, and `InjectResponder::answer` treats an unmatched iseq as
   `unmatched_injects` + Proceed — so the wire contract is: **the same
   iseq the SDK OUTs must be the one in the ring-W event**. One
   counter, used for both.
3. **`inject.rs`: the real body.**
   - Intern the name (existing `intern` machinery; that yields
     `name_id`).
   - Emit `EventPayload::InjectQuery { iseq, name_id }` as
     `EventClass::Critical` on ring W **with doorbell**
     (`emit_w_event_with_doorbell`) — the host drains the query inside
     the PIO exit, so publication must precede the detcall. This
     ordering is the bead's stated acceptance; make it structurally
     obvious in the code, not incidental.
   - `detcall_out(PORT_INJECT, iseq)` then
     `let packed = detcall_in(PORT_INJECT)` —
     `PORT_INJECT = 0xD384`, contract documented at
     `crates/detguest-wire/src/ports.rs:23`.
   - `FaultDecision::unpack(packed)` (`ports.rs:124-135`) and return.
   - **Standalone/error paths return `Proceed`**: no channel mapped,
     intern failure/invalid name, ring-W publication failure paths that
     the Critical retry discipline can still exhaust — enumerate each
     path in a test. Keep the existing
     `stats.inject_queries_total` bump in `lib.rs` (already landed);
     consider a companion counter for answered-vs-defaulted if the
     stats layout has room, but do not reshape the stats region here.
4. **Unit tests** (the bead's acceptance): (a) ordering — ring-W
   InjectQuery publication observed **before** the OUT on the mock;
   (b) packed decision decoding — Proceed / Platform / Workload
   variants round-trip through the mocked IN; (c) standalone mode
   returns Proceed without touching the port; (d) iseq increments per
   call and matches the emitted event.

Close the bead citing the commit. Do **not** extend `testload` or
`tests/vm` here — that is `m5-vm-inject-roundtrip` (round 2).

## §B — `m5-host-log-fault-plan`: adapter over supplied replay decisions

Claim after package 01 flips `ext-hyp-input-log-dev-events`. Spec from
the bead: "a guest-sdk-owned adapter over supplied replay decisions
while leaving DHILOG serialization to determinism-hypervisor.
Acceptance: tests prove same iseq and name_id produce logged decisions
with the synthesizer absent, unmatched entries fail loudly or proceed
only where API says so."

Current skeleton: `crates/detguest-host/src/inject.rs:126-139` —
`LogFaultPlan` is empty, answers Proceed, with a TODO pointing at the
replay cursor over "(iseq, decision) records."

Design:

1. `LogFaultPlan::new(decisions: Vec<LoggedDecision>)` where
   `LoggedDecision { iseq: u32, name_id: u32, decision: FaultDecision }`.
   The constructor is the "supplied replay decisions" seam: in round 2
   the caller feeds it decisions parsed from real DHILOG records
   (parsing stays hypervisor-format-owned per the bead — guest-sdk
   consumes decoded decisions, not DHILOG bytes); today tests feed it
   synthetic fixtures.
2. `decide(iseq, name_id, name)`:
   - Cursor-advance in log order; a matching `(iseq, name_id)` at the
     cursor returns its decision verbatim.
   - **Mismatch semantics must be explicit and loud** — this is the
     replay-divergence surface. Distinguish and count: (a) iseq
     mismatch at cursor, (b) name_id mismatch for a matching iseq,
     (c) query past end of log. Each returns `Proceed` (the
     `FaultPlan` trait cannot fail) but records a divergence the
     harness can assert on — expose
     `divergences(&self) -> &[LogDivergence]` (or a counter struct).
     "Fail loudly" for a host-side plan means the *test/gate* fails on
     nonzero divergences; `decide` itself must not panic (it runs
     inside the PIO exit path).
3. Keep `Default` (empty log = every query is a past-end divergence
   answered Proceed) or drop it deliberately — decide against how
   `from_snapshot` constructs responders today
   (`InjectResponder::new(TableFaultPlan::new(Vec::new()))`).
4. Tests: same-iseq/name_id sequence replays a fixture of varied
   Platform/Workload/Proceed decisions verbatim (record-with-
   `TableFaultPlan` → feed its public `decisions` field (a
   `Vec<(u32, FaultDecision)>`, `inject.rs:88`) into `LogFaultPlan` →
   identical answers — this is the fixture round trip package 04's
   scaffold reuses); each divergence class detected and counted;
   interleaving with `InjectResponder::answer` over a real channel
   (existing test guest-mem) produces the same `pio_answer` values via
   the `RecordingSink`.

No recorded DHILOG fixtures exist in this repo — do not fabricate
"realistic" ones. If higher-fidelity fixtures are wanted later, ask the
hypervisor via their request dir (their M9 machinery holds sealed
DHILOG artifacts). Round-1 fidelity bar is the synthetic round trip.

Close the bead noting the DHILOG-backed leg rides round 2.

## §C — `m5-host-mutation-log-audit`: every host mutation reported exactly once

Claim; spec from the bead: "exhaustive tests that every host mutation
is reported exactly once: ring C and I pushes, ring A and W consumer
index bumps, and pio_answer for inject. Include wrap pads in ring_push
spans and failed pushes not logging. Acceptance: a single ordered trace
can replay all host-owned channel mutations." Reservations:
`crates/detguest-host/src/commands.rs`, `drain.rs`, `inject.rs`, tests.

The machinery exists (`ChannelWriteSink` / `RecordingSink` / `SinkOp`,
`crates/detguest-host/src/lib.rs:30-95`); this bead is the exhaustive
proof, plus fixing any gap the proof finds:

1. Enumerate every host-side channel mutation site in
   `commands.rs`/`drain.rs`/`inject.rs`; assert each is mirrored by
   exactly one `SinkOp` — ring C push, ring I push, cons-bump A,
   cons-bump W, `pio_answer` — with correct payloads (ring id,
   new_prod/new_cons, record bytes, packed answer).
2. Edge cases the bead names: a push whose span includes a wrap pad
   logs the span it actually wrote; a **failed** push (ring full)
   logs nothing.
3. The "single ordered trace" acceptance: one test drives a mixed
   workload (commands pushed, rings drained, injects answered) and
   then replays the recorded `SinkOp` trace against a second
   guest-mem/channel, asserting byte-identical ring state and
   identical counters. This test is a direct precursor of package
   04's replay-fidelity surface — write it as a reusable helper
   (`tests` support module in detguest-host), not a one-off.

If a mutation site turns out not to route through the sink, that is a
real bug this bead exists to catch — fix it in the same change.

Close the bead; its closure flips `m5-channel-reattach-checkpoint`'s
last dep, which then gets the annotation from the triage table (open,
started-via-4bc, in-VM verification in round 2).

## Done when

- Triage table applied: every Ms5-chain bead's status/NOTES matches its
  disposition row (verify with `bd show` per bead).
- §A, §B, §C landed with green unit tests, beads closed with commit
  SHAs.
- `cargo test -p detguest-sdk -p detguest-host -p detguest-wire` green,
  plus the workspace's usual host-only gates (fmt/clippy per CI).
