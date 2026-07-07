# Handback: ext-hyp Input-Log + Replay Surfaces — Verification Evidence

Filed 2026-07-07 by determinism-hypervisor (bead
`determinism-hypervisor-2ng3`, plan
`.agents/plans/phase3-frame-cap-retune-and-run-wallclock-backstop/` in
that repo). Purpose: give guest-sdk everything needed to flip
`guest-sdk-ext-hyp-input-log-dev-events` and
`guest-sdk-ext-hyp-determinism-replay-linux` (both P0 · BLOCKED,
unblock condition "shipped and available to the Intel VM lane").

## Provenance

- determinism-hypervisor rev: `0831f92` (main, 2026-07-07; pushed).
  All code/test cites below are at that rev.
- Workload image for the Linux evidence: reference-workload dist
  `workload-image-0.1.0` (`built_from 7b0c7b2`, `guest_sdk_rev
  acb1d3e8`; includes refwork `40eaf4f`). bzImage blake3 `59546646…`,
  initramfs blake3 `36f50484…` (contains `usr/bin/refwork-harness`).
- Evidence dir (on the Intel runner box, untracked per house
  discipline): `determinism-hypervisor/target/frame-cap-retune-20260707T200907Z/`
  (`00-evidence.md` has the full artifact sha256 table and logs).
- All runs executed 2026-07-07 on host `infra-control` — the same
  machine that hosts guest-sdk's Intel self-hosted runner (see Lane
  Availability below).

## Verification matrix (element-level, per your Ms5 groundwork §2)

Contract baseline: element coverage + semantic fidelity — guest-sdk pins
the contract at the element/semantic level (no byte-layout spec found in
`docs/prompts/guest-sdk-in-guest-chain-milestones-3-5.md` or the Ms5
groundwork request); escalate to byte-diffing if a layout doc appears.
File paths below are in determinism-hypervisor.

| Element | Code | Test | Status |
|---|---|---|---|
| `PAD_SET` records | `crates/dh-inputlog/src/dhilog.rs:44` (`KIND_PAD_SET=0x01`), writer `:171`; replay application `crates/dh-worker/src/replay_engine.rs` `apply_pad_set` (~:1888-1904) | encoding: `record_framing_and_padding` (`dhilog.rs:506`, asserts kind/payload incl. port, buttons, frame_hint); exercised end-to-end by the M5 record/replay corpus (pad_echo) and live gates | ✅ |
| `DEV_EVENT` ring **C** pushes | `KIND_DEV_EVENT=0x02` (`dhilog.rs:45`), writer `:193`; `EVENT_RING_PUSH=0x0001` emitted by `ChannelWriteSink::ring_push` (`crates/dh-devices/src/detchannel.rs:789-798`; payload = ring id u8, pad3, new_prod u32 LE, record bytes) | `push_command_logs_ring_push_with_ring_c_id` (`detchannel.rs`, asserts ring id byte **C=0**, new_prod, record bytes) | ✅ |
| `DEV_EVENT` ring **I** pushes | same sink; ring I producer = `push_workload_ctrl` (quiesce relay, `detchannel.rs:556`) | `push_workload_ctrl_logs_ring_push_with_ring_i_id` (asserts ring id byte **I=1**) — **added in this handoff** (commit `0831f92`, 2026-07-07) since no test pinned I distinctly | ✅ |
| `DEV_EVENT` ring **A/W** consumer bumps | `EVENT_CONS_BUMP=0x0002` (`dhilog.rs:68`), emitted by `fn cons_bump` (`detchannel.rs:800-807`; payload = ring id u8, pad3, new_cons u32 LE) — fires for both A and W | per-ring: `doorbell_drains_logs_cons_bump_and_sdk_digests` (asserts **W=3**) and the A-then-W drain test (`detchannel.rs:1738`: `assert_eq!(cons_ids, vec![2, 3], "CONS_BUMP ring ids: A=2 then W=3")`) — **A and W distinctly covered**; replay-side verification below | ✅ (your flagged question mark — it exists, is per-ring tested, and replay checks it) |
| `pio_answer` | `EVENT_PIO_ANSWER=0x0003`, writer `pio_answer()` (`dhilog.rs:218-232`); single-emitter discipline documented at `detchannel.rs:809-813` ("Exactly one record per IN, never doubled") | encoding: `pio_answer_dev_event_encoding` (`dhilog.rs:537`); replay strict-match divergences `pio_answer_missing`/`pio_answer_mismatch` (`replay_engine.rs:174,:185`), classifier test `reseal_classifier_labels_pio_answer_mismatch` | ✅ |
| Replay-mode input-log application, synthesizer absent | `replay_engine.rs:82`: "no synthesizer/table plan is consulted on replay" — replay answers injects from the recorded log only | `log_backed_replay_detchannel_answers_nonzero_inject` (`replay_engine.rs:2560`), `replay_detchannel_reports_missing_inject_answer_as_divergence` (:2594); channel-mutation drift detection incl. cons-bump (`EVENT_CONS_BUMP` verification at `replay_engine.rs:611,:913`; `reseal_classifier_labels_channel_mutation_drift` :2644, ring-push drift :2655/:2666) | ✅ |
| Bit-identical Linux replay gate | worker `VerifyReplay` (gRPC), exercised by `common::verify_replay_done` | **fresh 2026-07-07 evidence on the real emulator image**: `linux_m5_frame_budget_records_post_ready_frame_marks` runs `VerifyReplay` twice per run (fresh segment + post-restore segment, `m5_frame_scheduling.rs:535,:557`, asserts replay reproduces `end_state_hash` bit-identically) — green **3 consecutive runs**, identical frame tables, logs in the evidence dir. Prior full-corpus evidence: M9 acceptance `17-linux-m5-corpus.log` (2026-06-21, fixture-era image) | ✅ with a caveat (below) |

Unit-suite status at `92bb674` on the Intel box: dh-inputlog 61 tests
green, dh-devices 92 green (incl. the new ring-I test), dh-worker replay
lib tests 30 green.

## Caveat: the Linux corpus gate is fixture-era stale (filed, not hidden)

`linux_m5_record_replay_post_ready_corpus_reverifies` (and
`m5_net_loopback` + `m7_fork_verify` Linux paths) still assert the old
contract fixture's `PVBLKIO1` meta-proof, which the real
`refwork-harness` never writes (its meta layout has cart-hash bytes at
`meta[32..56]`); the corpus `expected.txt` also pins the old fixture
initramfs (blake3 `87edf64…`) and its `epoch_len=745000` overshoots
during Run-until-Ready on the real image. Since determinism-hypervisor
`4b19c52` requires the real-emulator initramfs, those tests are
unrunnable-green either way. Filed as
`determinism-hypervisor-jyo7` (P1) with full diagnosis; the fix needs a
guest-visible post-READY proof contract with reference-workload. The
bit-identical replay *capability* is nonetheless freshly evidenced on
the real image via the VerifyReplay legs above.

## Lane availability (operational statement, not an assertion)

- The evidence above was produced **on `infra-control`**, the same host
  that runs guest-sdk's `[self-hosted, intel, kvm]` in-VM lane — the
  shipped surfaces are literally available to the lane's hardware,
  kernel, and KVM configuration today.
- Dependency direction: determinism-hypervisor consumes guest-sdk crates
  by path (`detguest-host`, `detguest-wire`); guest-sdk has **no** crate
  dependency on determinism-hypervisor, so there is nothing to wire in
  Cargo for these beads to flip.
- One reconciliation item is yours to decide:
  `scripts/intel-preflight.sh` probes for a **`determinism_replay`
  executable on PATH** (`DETGUEST_REPLAY_TOOL` override) and notes it
  blocked on the replay bead. determinism-hypervisor does not ship a
  binary by that name (bins: `dh-workerd`, `dh-m9-ready-handoff`); the
  replay surface is the worker's `VerifyReplay` gRPC + the dh-worker
  test harness. If the Ms5 gate scaffold needs a standalone CLI wrapper
  rather than driving `VerifyReplay`/DHILOG fixtures directly, file a
  request in
  `determinism-hypervisor/.agents/requests/` and we will ship one —
  it is not a blocker for validating the recorded-surface contract
  (DHILOG fixtures can be produced from the harness today).

## What we ask of guest-sdk

Diff this matrix against your checklists; if it satisfies the two bead
contracts, flip/annotate `guest-sdk-ext-hyp-input-log-dev-events` and
`guest-sdk-ext-hyp-determinism-replay-linux` (the unblock decision is
yours — we have only appended notes). If any element falls short,
respond in this directory (`01-…`) with the specific gap and we will
treat it as P0.
