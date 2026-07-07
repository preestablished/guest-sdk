# Package 04 — The `determinism_replay` Scaffold

Covers request item 4. Target, verbatim: "when the handoff lands, the
remaining work is filling stubs and running the 1000-iteration
acceptance, not designing a gate." The handoff *has* landed (package
01), so the bar sharpens: the stubs must be shaped so round 2's item 2
fills them without redesign.

Gate: packages 01–03 (01 mints the checklist IDs the stubs cite; 02
and 03 supply the re-seed accessors, `inject_point` mechanics,
`LogFaultPlan`, and the §C ordered-trace helper).

## What the gate must eventually prove (the spec the scaffold encodes)

From bead `m5-determinism-replay-ci-gate` (P0) and
`docs/prompts/guest-sdk-in-guest-chain-milestones-3-5.md:58-59`: 1000
seeded iterations, varied fault plans **and input bursts**,
bit-identical with synthesizer absent, across the four surfaces the
bead enumerates:

- S1: ring C/I pushes
- S2: ring A/W consumer bumps
- S3: pio answers (inject decisions)
- S4: SDK event / drop counter equivalence

The request phrases S1–S3 as "the LogLine digest" family and RAM/
framebuffer; where the request's surface wording and the bead's differ,
**the bead + IMPLEMENTATION-PLAN wording wins** (it is the CI-gate
contract). RAM/framebuffer hashing is hypervisor-side (`VerifyReplay`
already proves `end_state_hash` bit-identity — DRL-4); the guest-sdk
gate owns S1–S4. Record this ownership split in the test's module doc —
it is the kind of decision that otherwise gets re-litigated in round 2.

## Structure

New file `tests/vm/tests/determinism_replay.rs` plus a support module
`tests/vm/src/replay.rs` (so both the ungated self-tests and the gated
in-VM leg share the surface-hashing code).

### `tests/vm/src/replay.rs` — the comparable-run digest

1. `RunDigest { s1_ring_pushes: u64, s2_cons_bumps: u64,
   s3_pio_answers: u64, s4_sdk_events: u64 }` — one FNV-1a-64 per
   surface, following the repo's existing convention
   (`fnv1a64_lines`, `tests/vm/tests/m2_acceptance.rs:304-318` — move
   or re-export the helper into the support crate rather than copying
   it a third time; `game_materialization.rs` already duplicates a
   checksum helper, don't add another).
2. `digest_from_trace(ops: &[SinkOp], events: &[NormalizedEvent])
   -> RunDigest` — S1–S3 fold from the `RecordingSink` trace (package
   03 §C's ordered-trace helper is the input shape); S4 folds from the
   normalized drained event lines plus drop counters (reuse the
   normalization approach of `m3_testload_event_lines`,
   `m2_acceptance.rs:251`).
3. `assert_digests_equal(a, b) -> Result<(), SurfaceMismatch>` naming
   the first divergent surface — the gate's failure message must say
   *which* surface diverged, or a 1000-iteration failure is
   undebuggable.

### Ungated self-test legs (plain `#[test]`, no KVM, no env)

These are what the phases track re-runs from a clean checkout
(`03-verification-offer.md` names them, including the negative):

- **Fixture round trip**: drive a host-only channel (test guest-mem)
  through a scripted mixed workload twice — record leg with
  `TableFaultPlan` (varied Platform/Workload decisions + scripted
  input-burst events), replay leg with `LogFaultPlan` seeded from the
  record leg's decisions — and assert `RunDigest` equality across all
  four surfaces plus zero `LogFaultPlan` divergences.
- **Deliberate-mismatch negative** (acceptance criterion 4 names it):
  perturb exactly one surface per test case — one extra ring push, one
  altered cons-bump, one flipped fault decision, one dropped SDK
  event — and assert `assert_digests_equal` fails naming that surface.
  Four cases, one per surface. A comparison that cannot fail is not
  evidence; this proves it can, per-surface.
- **Seed variation self-test**: two different seeds produce different
  digests (guards against a digest that hashes nothing).

### The gated in-VM leg (stub-bearing)

Follow the house double-gate discipline exactly
(`#[ignore = "KVM tier: Intel runner only (DETGUEST_VM_TESTS=1)"]` +
the `vm_tests_enabled()` early-return; single-threaded lane invocation
documented in the file header, matching `m4_acceptance.rs`).

Iteration skeleton, shaped for round 2's requirements (seeded,
chunked, resumable — round 2's item 2 demands resumability; give it
the seams now):

- `DETGUEST_REPLAY_ITERS` (default small, e.g. 2, for lane smoke),
  `DETGUEST_REPLAY_SEED_BASE`, `DETGUEST_REPLAY_RESUME_AT` env knobs.
- Per iteration: boot-or-restore per the M4 acceptance pattern
  (`m4_acceptance.rs` — root boots once, children restore; the
  package-02 accessors make child channels fully re-seeded), drive the
  workload with a seed-derived fault plan **and a seed-derived input
  burst schedule** (the pv-pad scheduling pattern from
  `m4_acceptance.rs` children is the precedent), run record leg and
  replay leg, compare `RunDigest`s.
- **Stubs** — each a `panic!`/`unimplemented!` with a message citing
  what it awaits, so a stub reached is loud, not silently green.
  With the handback landed, stubs cite round-2 work items rather than
  missing external capability, plus the checklist item that grounds
  them:
  - workload-side `inject_point` call sites in a VM workload bin —
    awaits `m5-vm-inject-roundtrip` (round-2 item 1); grounded by
    ILDE-6.
  - real-recorded decision ingestion (DHILOG-decoded decisions into
    `LogFaultPlan`) — awaits round-2 item 1's DHILOG-backed
    completion; grounded by ILDE-1..6.
  - cross-checking against hypervisor `VerifyReplay` end-state hashes
    — awaits round-2 item 2; grounded by DRL-4/DRL-5.
- **No stub may sit on the ungated self-test path.** `cargo test -p
  detguest-vmtest` (host-only) must be green with every self-test leg
  actually executing.

### CI wiring

Behind the existing `DETGUEST_VM_TESTS` discipline in `tests/vm/` —
the in-VM leg rides the same lane invocation as the other ignored
tests (`--ignored --test-threads=1`); no new CI lane in this round
(`m3m5-ci-intel-vm-lanes` is round-2). The ungated self-tests join the
default host-only test run automatically.

## `scripts/intel-preflight.sh` reconciliation

Lines 224-234 probe for a `determinism_replay` executable and print
"remains blocked by guest-sdk-ext-hyp-determinism-replay-linux" — both
halves go stale after this plan: the bead flips (package 01), and the
handback states no such binary exists (the replay surface is dh-worker
`VerifyReplay` gRPC + DHILOG fixtures; a CLI is available on request).
Update the block: keep the `DETGUEST_REPLAY_TOOL` override behavior,
change the default-absent `note` to state that the guest-sdk Ms5 gate
drives fixtures/harness directly (this scaffold), that no dh binary
named `determinism_replay` exists, and that a CLI wrapper can be
requested from dh if round 2 wants one. Per overview decision 7, do
not file that CLI request now.

## Done when

- `cargo test -p detguest-vmtest` (host-only, ungated) runs the
  fixture round trip, the four deliberate-mismatch negatives, and the
  seed-variation test — all green, from a clean checkout.
- The gated leg compiles under the lane invocation and runs its
  non-stub portion at `DETGUEST_REPLAY_ITERS=2` if lane time permits
  (record the result either way; the request's acceptance only
  requires the self-test legs).
- Every stub message cites its round-2 item + checklist ID; grep for
  the stub marker proves the enumeration (pick one marker string,
  e.g. `MS5-STUB`, and say so in the module doc).
- Preflight message updated.
- `bd update guest-sdk-m5-determinism-replay-ci-gate --notes=...`
  (preserving existing NOTES) records: scaffold landed at SHA,
  remaining work enumerated.
