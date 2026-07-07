# Current State (Evidence-Based)

Repo `main` at `a3e6f0e`, clean tree, assessed 2026-07-07.

## Done And Verified (Phase 3 Ledger So Far)

- **Ms0–Ms2**: wire crate (no_std/miri/loom/fuzz CI), host channel, PID-1
  agent with in-VM acceptance and the static-musl lane.
- **Ms4 ⭐**: real mlock/prefault/pagemap registration + agent-IPC, real
  `ReverifyRegions`, the 100× snapshot/restore acceptance — delivered and
  independently verified
  (`requests/phase3-ms4-region-publication-acceptance/05-resolution.md`,
  `06-verification.md`).
- **Request dirs resolved**: ms4-region-publication,
  game-device-materialization, boot-scheduling-deadlock (verified on the
  real worker — first real emulator+game READY, 2026-07-05),
  post-ready-no-frame (classified not-a-guest-sdk-bug; guards added).
  ring-a-doorbell-drain was folded into the deadlock fix but never got
  the `03-resolution.md` its own `02-` promised — a two-minute ledger
  item worth closing while you're here.
- **Ms3 functionally complete, with one caveat**: the SDK surface exists
  (`crates/detguest-sdk/src/lib.rs` — init, assert_always,
  expect_reachable, coverage_beacon, inject_point, poll_input,
  frame_mark, quiesce_check, log_line, stats), but `inject_point` is a
  Proceed-only stub (no InjectQuery emission, no detcall — per its own
  bead's note); the real mechanics are Ms5 work. Its formal in-VM
  acceptance beads are blocked on the same external chain as Ms5.

## The Blocked Graph

Bead census (`bd list --status <s> --limit 0` — beware bd's default
50-row limit, which produced a wrong count in an earlier draft):
134 total = 106 closed + 27 blocked + 1 open/ready (`guest-sdk-4bc`,
P2), 0 in progress. The Ms5 chain as tracked:

- `guest-sdk-m5-host-log-fault-plan` — "Replace LogFaultPlan skeleton"
  (`crates/detguest-host/src/inject.rs`); marked blocked, and it does
  carry a live dep edge on `ext-hyp-input-log-dev-events` — though its
  own description scopes DHILOG serialization *out* and defines the
  adapter over "supplied replay decisions," which is fixture-testable.
- `guest-sdk-m5-sdk-inject-point` — the OUT/IN detcall round trip;
  marked blocked, but its **only dependency
  (`m4-platform-readability-vm`) is closed** — the BLOCKED status is a
  blanket note stamped 2026-06-18, and its acceptance is pure unit tests
  (ring-W publication ordering, packed decision decoding).
- `guest-sdk-m5-vm-inject-roundtrip` — in-VM leg; blocked, but its
  side-chain (`m5-channel-reattach-checkpoint` →
  `m5-host-mutation-log-audit`) bottoms out on a **closed** dep — same
  blanket-note pattern as inject-point.
- `guest-sdk-m5-determinism-replay-ci-gate` (P0) — the flagship:
  1000-iteration bit-identical replay ("1000 consecutive iterations with
  varied fault plans", IMPLEMENTATION-PLAN §Ms5); blocked, with a live
  dep edge on `ext-hyp-determinism-replay-linux`.
- `guest-sdk-m3m5-ci-intel-vm-lanes` (P0) — wiring into the `in_vm` CI
  lane; blocked.

To be precise about the graph (an earlier draft overclaimed): only
**two** of these carry live dependency edges to the external beads —
`m5-host-log-fault-plan` → `ext-hyp-input-log-dev-events` and
`m5-determinism-replay-ci-gate` → `ext-hyp-determinism-replay-linux`.
The rest are blocked by manually stamped status. The external beads:
`guest-sdk-ext-hyp-input-log-dev-events` (PAD_SET / DEV_EVENT encodings —
ring C/I pushes, ring A/W consumer bumps, `pio_answer`) and
`guest-sdk-ext-hyp-determinism-replay-linux` (bit-identical Linux replay
gate, replay-mode input-log application). Both P0, both last touched
**2026-06-18** — three days before the hypervisor's M9 acceptance landed
its Linux record-replay corpus gate.

## What Moved On The Hypervisor Side

Per today's request in their repo (item 3 asks them to verify and hand
off): `dh-inputlog` defines `KIND_PAD_SET`/`KIND_DEV_EVENT` including
`pio_answer`; `dh-worker`'s replay engine applies PadSet/DevEvent in
replay mode; the M9 evidence includes the Linux M5 record-replay corpus
run. Unverified against your bead contracts: the ring A/W consumer-bump
encodings, and Intel-VM-lane availability of all of it — which is exactly
what their request makes them check. Your beads' own unblock condition
("shipped *and available to the Intel VM lane*") is about to be tested.

## Known Small Debts In This Repo

- **`guest-sdk-4bc` (P2, ready)**: `detguest-host::Channel` lacks
  intern-map re-seed accessors; harness snapshots carry intern records
  but children resolve names via manifest bytes only (documented in
  `tests/vm/src/harness/snapshot.rs`; filed as follow-up in the Ms4
  resolution's "Notes for anyone touching this next").
- **Residual from the no-frame resolution**
  (`requests/phase3-post-ready-no-frame-under-no-tick/00-resolution.md`,
  lines 60–68): the fixture-based `tests/vm/tests/no_timer_post_ready.rs`
  is green, but the **real-artifact twin in
  `tests/vm/tests/refwork_ready_hold.rs`** ("Gated: runs only when
  `REFWORK_READY_INITRAMFS` is set") had its bodies skipped —
  the env var was unset locally, so the strengthened no-timer assertion
  has never run against a real reference-workload artifact.
  Reference-workload's request (filed today) regenerates exactly the
  artifact needed (`dist/workload-image-0.1.0/`'s initramfs).

## Who Waits On Ms5

Phase 3 exit gate 2 names it; the phase cannot close without it. Behind
that, Phase 4's entry (real captures, feature-map exercise) assumes the
determinism story is proven end-to-end — Ms5 is the proof.
