# Resolution: Ms5 Groundwork Landed — Round 2 Is A Short Execution

Resolved 2026-07-07 against plan
`.agents/plans/phase3-ms5-groundwork-while-blocked/` (repo baseline
`db50f76`; plan at `b1d89dc`). All six request items are done; the only
leg the request allowed to stay gated (item 5) is **also done** — the
refwork artifact turned out to already exist.

**Order inversion (the request's sanctioned fallback):** the
determinism-hypervisor evidence arrived (`a4d4e6e`/`db50f76`) before the
item-2 checklist landed, so the checklist was minted from the bead
contracts first and the evidence diffed against it on receipt — same
rigor, opposite order. Verifiers checking item 2/6 should expect that
order (their step 3 anticipates it).

## Commits (this repo, in order)

| SHA | Package | Content |
|---|---|---|
| `1fe500c` | 01 | Checklist diff execution notes (`.agents/plans/phase3-ms5-groundwork-while-blocked/07-execution-notes.md`) |
| `6c0c1c3` | 02 | `guest-sdk-4bc`: Channel intern/pending-inject re-seed accessors + harness re-seed |
| `873e174` | 03 | `inject_point` mechanics, `LogFaultPlan` adapter, mutation-log audit + replay module |
| `c13ee1a` | 04 | `determinism_replay` scaffold (four surfaces, self-tests, MS5-STUB stubs), preflight message fix |
| `635533e` | 05 | Refwork real-artifact run record + ring-a-doorbell-drain `03-resolution.md` |
| (this commit) | 06 | This resolution + clippy type-alias fix |

Hypervisor-repo ack: `determinism-hypervisor` `d8abd74`
(`.agents/requests/phase3-frame-cap-retune-and-run-wallclock-backstop/05-guest-sdk-ack.md`,
pushed).

## Item 2 + 6 — checklist, diff, flip, ack

- Checklist lives **in the two bead descriptions** (`bd show`):
  `guest-sdk-ext-hyp-input-log-dev-events` carries `ILDE-1..7`,
  `guest-sdk-ext-hyp-determinism-replay-linux` carries `DRL-1..5`.
- Diff table (checklist ID → handback evidence → verdict) with an
  independent spot-check of every cited dh symbol/test at `0831f92`:
  `.agents/plans/phase3-ms5-groundwork-while-blocked/07-execution-notes.md`.
  Every item satisfied.
- **Both beads flipped (closed)**. DRL-4 verdict: satisfied with the
  disclosed caveat recorded verbatim in the flip annotation — dh's
  fixture-era Linux corpus gate is stale vs the real image (their bead
  `determinism-hypervisor-jyo7`, P1); the capability is freshly
  evidenced on real `workload-image-0.1.0` via VerifyReplay ×2/run,
  green 3 consecutive runs.
- Ack (their acceptance criterion 3) committed in their repo at
  `d8abd74`; it also carries the "checklist is live" pointer.

## Item 1 — `guest-sdk-4bc` (closed at `6c0c1c3`)

`Channel::interns`/`restore_interns` + `pending_injects`/
`restore_pending_injects` (ProducerSeqs pattern; public
`InternSnapshotEntry` carrier documents the lossy-UTF-8 name rule).
Harness `snapshot()` captures from the channel's own maps;
`from_snapshot` re-seeds both; the "cannot be re-seeded yet" notes are
gone (`grep -c "cannot be re-seeded" tests/vm/src/harness/snapshot.rs`
→ 0). Proving tests: host-only re-seed unit tests, plus a post-restore
assertion in `m4_snapshot::snapshot_restore_guest_still_runs` that a
child resolves a root-interned `name_id` without any drain — the gated
tier was **executed** on the lane host (3/3 ok), not shipped cold.
Snapshots are in-process only (never serialized), so the
`HostChannelState` field addition has no format-compat concern.

## Item 3 — re-triage + the genuinely-unblocked work (at `873e174`)

Triage as executed (every blocked bead now cites its specific gate):

| Bead | Disposition |
|---|---|
| `m5-sdk-inject-point` | **Closed.** Real OUT/IN detcall mechanics + `detcall_in` + scriptable PIO mock; ordering/decoding/standalone/iseq/exhaustion unit tests. |
| `m5-host-log-fault-plan` | **Closed** (dep flipped by item 6). Cursor adapter over supplied `LoggedDecision`s; divergences classified (iseq/name_id/past-end), counted, answered Proceed; fixture round trip vs `TableFaultPlan`. DHILOG-backed leg noted as round-2. |
| `m5-host-mutation-log-audit` | **Closed.** All write sites audited (`push_record`, `drain_ring` only — no unsinked mutation found); `detguest_host::replay` applies a `SinkOp` trace to a second channel; byte-identical page after single-ordered-trace replay, wrap-pad and failed-push edges pinned. |
| `m5-channel-reattach-checkpoint` | **Open (unblocked by the audit closing), annotated**: started-via-4bc; remaining clause (restored branches continue sequences without duplicates, in-VM) is round-2 item 1. |
| `m5-vm-inject-roundtrip` | Blocked, annotated: sibling deps are the gate; round-2 item 1; grounded by ILDE-6. |
| `m5-determinism-replay-ci-gate` | Blocked on the roundtrip bead only (external dep flipped), annotated: scaffold landed at `c13ee1a`; remaining = fill MS5-STUBs + 1000-iteration run (round-2 item 2); grounded by DRL-4/5. |
| `m3m5-ci-intel-vm-lanes` | Blocked on siblings, annotated: round-2 item 4; no external checklist item. |

No recorded DHILOG fixtures were fabricated; round-1 fidelity is the
synthetic round trip, per the request's scope note.

## Item 4 — the scaffold (at `c13ee1a`)

`tests/vm/src/replay.rs` (RunDigest S1–S4, `digest_from_trace`,
`assert_digests_equal` naming the first divergent surface;
`fnv1a64_lines` moved here from m2 instead of a third copy) +
`tests/vm/tests/determinism_replay.rs`. Ownership split recorded in the
module doc: hypervisor `VerifyReplay` owns RAM/framebuffer bit-identity
(DRL-4/5); this gate owns S1–S4 per the bead + IMPLEMENTATION-PLAN
wording.

Self-test output (clean checkout, `cargo test -p detguest-vmtest`,
host-only — verified from a fresh `git clone` with the `control-plane`
sibling present, which the workspace requires):

```text
test fixture_round_trip_is_bit_identical_across_all_surfaces ... ok
test negative_one_extra_ring_push_fails_naming_s1 ... ok
test negative_altered_cons_bump_fails_naming_s2 ... ok
test negative_flipped_fault_decision_fails_naming_s3 ... ok
test negative_dropped_sdk_event_fails_naming_s4 ... ok
test seed_variation_produces_different_digests ... ok
test same_seed_record_legs_are_bit_identical ... ok
test result: ok. 7 passed; 0 failed; 1 ignored (the gated leg)
```

The gated in-VM leg (seeded/chunked/resumable via
`DETGUEST_REPLAY_ITERS`/`SEED_BASE`/`RESUME_AT`) ran green on the lane
host at the default `ITERS=2` — both iterations bit-identical across
all four surfaces. Stubs: marker `MS5-STUB`, three hits, each citing
its round-2 item + checklist ID (workload inject call sites →
`m5-vm-inject-roundtrip`/ILDE-6; DHILOG-decoded ingestion → round-2
item 1/ILDE-1..6; VerifyReplay cross-check → round-2 item 2/DRL-4..5).
Stub loudness verified: `DETGUEST_REPLAY_EXERCISE_STUBS=inject` panics
with the citation. No stub sits on the ungated path.
`scripts/intel-preflight.sh`'s stale probe note updated (no dh binary
named `determinism_replay` exists; CLI available on request — not
requested this round, per plan decision 7).

## Item 5 — refwork real-artifact run (at `635533e`)

The gate was already open: `workload-image-0.1.0` exists in
`../reference-workload/dist/` (initramfs decompressed blake3
`36f50484…`, byte-identical to the dh handback's cite; artifact's own
bzImage `59546646…` used via `REFWORK_READY_BZIMAGE`). Both
`refwork_ready_hold` twins ran with bodies executed (zero
"skipping" lines, 7.9 s of real boots): **2/2 pass**. Recorded in
`.agents/requests/phase3-post-ready-no-frame-under-no-tick/01-real-artifact-run.md`.
No finding to file.

Ledger debt: `.agents/requests/phase3-ring-a-doorbell-drain/03-resolution.md`
now exists (routing decision = agent-side via deadlock Fix A `70851a2`;
real-worker verification `1f9a123`; the step-2 emit-loop bounding did
not ride along and is flagged as wanting its own bead if still desired).

## Gates

`cargo fmt --check`, `cargo clippy --workspace --all-targets --
-D warnings`, and the full host-only workspace test suite are green at
this commit; the clean-checkout re-run above matches. Lane-host
executions this round: m4_snapshot 3/3, determinism_replay gated leg
2/2 iterations, refwork_ready_hold 2/2.
