# Current State (Evidence-Based)

Repo `main` at `c2d48d8` (the round-1 filing commit), clean tree,
assessed 2026-07-07. Census (`bd list --status <s> --limit 0` — mind
bd's 50-row default): 134 = 106 closed + 27 blocked + 1 ready
(`guest-sdk-4bc`), 0 in progress. Round-1 is entirely unexecuted (all
six items verified untouched).

## The Tail's Bead Graph (Live Edges Vs Blanket Labels)

- `m5-sdk-inject-point` (P1): only dep **closed** — blanket label;
  round-1 item 3 unblocks and does it. (SDK-side work:
  `crates/detguest-sdk/src/{inject,pio,lib}.rs`.)
- `m5-host-log-fault-plan` (P1): live edge →
  `ext-hyp-input-log-dev-events`.
- `m5-channel-reattach-checkpoint` → `m5-host-mutation-log-audit`
  (P1 side-chain): bottoms out on closed deps — blanket labels.
- `m5-vm-inject-roundtrip` (P1): deps are the fault-plan/side-chain
  host beads plus the SDK-side `m5-sdk-inject-point`; no direct
  external edge.
- `m5-determinism-replay-ci-gate` (P0, flagship): live edges →
  `ext-hyp-determinism-replay-linux` + `m5-vm-inject-roundtrip`.
  Plan bar (IMPLEMENTATION-PLAN "Milestone 5", line ~154): "1000
  consecutive iterations with varied fault plans **and input bursts
  (seeded, logged)**," bit-identical across the plan's four surfaces:
  (a) final guest RAM hash, (b) the complete drained event stream
  byte-for-byte, (c) drop counters, (d) all inject decisions (echoed
  via LogLine digest). *(No framebuffer surface — an earlier draft
  imported that from Phase-4 capture language; the plan doesn't ask
  for it here.)*
- `m3-vm-real-workload-e2e` (P0): live edge → `ext-hyp-m9-linux-guest`.
- `m3-input-path-acceptance` (P0): live edge →
  `ext-hyp-input-log-dev-events` — same bead as the fault-plan work.
- `m3m5-ci-intel-vm-lanes` (P0): **direct** live edges to
  `m3-vm-real-workload-e2e` + `m5-determinism-replay-ci-gate` (plus a
  closed m4 dep) — the coupling that makes Ms3+Ms5 one closeout.

**The rest of the tail this request must not forget** (from
`bd list --status blocked`, live state): `m3m5-final-quality-gates`
(**P0**), `m3m5-handoff-closeout` (P1),
`m5-reference-workload-contract-tests` (P1), `m3-docs-as-built` /
`m5-docs-replay` (P2 — unblock the moment the acceptances close), and
three still-blocked **M4 stragglers** (`m4-capture-contract-tests`,
`m4-reverify-churn-test`, `m4-sdk-stats-region-autoreg`) — the first
of which also dents any "Phase 4 asks nothing else of this repo"
claim, since Phase 4 consumes exactly the captures whose contract
tests are un-landed.

## The Third External Bead

`ext-hyp-m9-linux-guest` (P0, last touched 2026-06-18) was never in
round-1's two-bead checklist, and the hypervisor's round-1 handoff
item targets only the DHILOG/replay pair. Reality: hypervisor M9
closed with final acceptance evidence 2026-06-21
(`../determinism-hypervisor/target/m9-final-acceptance-20260621T004402Z/`).
We disposition it ourselves against that evidence (receiving-side
diff); the hypervisor gets an FYI unless a gap is found.

## The Host Question — Corrected By Actually Running The Preflight

Two claims circulated; only one survives:

- **Kernel provenance: already green.** `scripts/intel-preflight.sh`
  run today passes all gates including every `kernel.provenance`
  check (kernel 6.12.93, tarball sha, build_key ok). The
  `m3m5-ci-intel-vm-lanes` NOTES said so on 2026-07-02 ("intel-preflight
  now passes on this host"); the e2e bead's 2026-06-18 NOTES predate
  that fix. Not a blocker; drop it from every narrative.
- **Hugepages: genuinely empty, need genuinely unconfirmed.**
  `/sys/.../hugepages-2048kB` shows 0/0 reserved and
  `--require-host-hugepages` FAILs. But whether M3 e2e *needs* host
  hugepages is open: the script's own comment says nothing in
  `tests/vm` does and attributes the need to the hypervisor harness —
  while a sweep of the hypervisor repo finds no hugepage usage at all
  (their host posture is THP-off). Either the comment is stale and
  the requirement dissolves, or it's real and belongs in the
  hypervisor's audited host-config regime
  (`docs/ops/host-config-intel-box.md` / `apply-host-config.sh`).
  Reconciling this is ungated prep work in `02-`.

## Cross-Request Position

- Upstream: hypervisor round-1 item 3 (DHILOG/replay handoff) —
  unexecuted; their round-2 (OOM fix) is orthogonal to this repo.
- Sideways: reference-workload round-1 feeds the real-workload legs;
  their M5 stamp is the other half of exit gates 1–2. Their operator
  cutover restarts the live worker — never schedule the 1000-iteration
  run across that window.
- Downstream: Phase 4's capture corpus assumes this tail is green.
