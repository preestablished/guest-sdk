# Package 04 — Real Determinism Replay, Timing Probe, And 1000-Run Gate

This package converts `tests/vm/tests/determinism_replay.rs` from a scaffold
into the flagship acceptance. Package 02 supplies its live inject,
replay-plan, and decoded-input seams; it does not wait for independent Ms3.

## A. Replace the scaffold, do not layer around it

Delete all three `stub_*` functions, every `MS5-STUB` marker, and the
`DETGUEST_REPLAY_EXERCISE_STUBS` bypass. For each iteration:

1. Restore a record child from the pinned root snapshot.
2. Generate the complete trajectory from the iteration seed: varied fault
   rules (guaranteeing all decision classes over the run) and seeded, logged
   PAD_SET input bursts.
3. Run the live workload with `TableFaultPlan`, retaining decoded decisions,
   mutation/event trace, drop counters, final guest RAM hash, and the
   external hypervisor end-state evidence where package 01 establishes a
   callable run/artifact contract.
4. Restore an independent replay child, ingest only the recorded decisions
   into `LogFaultPlan`, disable the synthesizer, replay the identical input
   burst, and require zero divergences.
5. Compare the request's four surfaces: final guest RAM hash; complete drained
   event stream bytes; drop counters; and inject decisions echoed through the
   agreed canonical LogLine digest. Also retain the scaffold's S1–S4 mutation
   digests as diagnostics. Guest-sdk has no VerifyReplay client: use a concrete
   cross-repo command/artifact contract identified in package 01 or cite the
   matching external evidence; do not fabricate an in-process API.

Resolve the scaffold comment's older surface wording explicitly in code/docs:
the request and implementation-plan acceptance surfaces are authoritative;
S1–S4 remain useful diagnostics, not a substitute for final RAM/event/drop/
inject equality. Add harness APIs for deterministic RAM hashing and complete
raw event bytes, explicitly defining included RAM ranges and canonicalization;
private snapshot memory and decoded events are insufficient. Do not introduce
framebuffer as a fifth acceptance surface.

Write one atomic evidence record after each successful iteration using package
01's schema. A failure record must include the first named divergent surface,
seed, iteration, resume command, serial, and artifact pointers.

## B. Resume and range semantics

Replace the ambiguous `for resume_at..iters` contract with explicit range
semantics (for example `START_ITER` + `ITER_COUNT`, or manifest-derived next
iteration). The 1000-run means exactly iteration IDs 0–999 once each. On
resume, validate configuration identity and refuse overlap/gaps unless an
explicit verification-only rerun flag is used. A final reducer checks 1000
unique green records and hashes the ordered summary.

Add host-only tests for interrupted write recovery, resume at a chunk boundary,
duplicate rejection, gap rejection, changed image/revision rejection, and
summary determinism.

## C. Negative against the real path

Retain the four host-only mismatch unit tests, and add one gated real-path
negative after the positive plumbing works. Perturb exactly one recorded
decision or PAD_SET record in the replay leg; require failure naming the
correct surface/iteration. Run it once separately from the 1000 positives. The
outer test exits zero only when rejection names the expected surface; equality
or wrong classification fails. Preserve the inner failure classification
without counting it as a failed positive iteration.

## D. Measure, then book and execute

Run a 10-iteration probe with production evidence enabled and record total,
per-iteration distribution, setup time, artifact growth, and projected 1000-run
duration. Use that result to choose chunk size and lab window; leave margin for
one chunk rerun. Do not extrapolate from the old empty-inject two-iteration
scaffold.

Execute chunks under one run manifest until the reducer proves 1000/1000.
Chunks may span sessions but not configuration identities. Monitor disk space,
runner temperature/health, worker revision, and service restarts between
chunks; a changed identity starts a new acceptance rather than mixing corpora.

## Done when

- `rg 'MS5-STUB|DETGUEST_REPLAY_EXERCISE_STUBS' tests/vm` finds nothing.
- The 10-run timing probe and real-path negative are preserved.
- The final manifest proves 1000 unique consecutive green iterations with
  seeds, fault/input censuses, four authoritative surface hashes, revisions,
  resume points, and ordered-summary digest.
- `guest-sdk-m5-determinism-replay-ci-gate` closes with those evidence paths.
