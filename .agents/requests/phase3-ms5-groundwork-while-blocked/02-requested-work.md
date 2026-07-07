# Requested Work

## What We Need (Behavioral)

1. **`guest-sdk-4bc` — do the ready bead.** Intern-map and pending-inject
   re-seed accessors on `detguest-host::Channel`, closing the debt the
   Ms4 resolution filed. Small, unblocked, on the Ms5 path (replay
   harnesses re-seed child channels).
2. **Write the handoff acceptance checklist — into the bead
   descriptions.** Turn each of your two ext-hyp bead contracts into an
   explicit per-item checklist (record kinds, encodings, ring semantics,
   replay behaviors, Intel-VM-lane availability — each item phrased as
   something the hypervisor can cite a test or evidence file against),
   **in the two bead descriptions themselves** (not a side doc — their
   request verifies against the bead contracts, so that is the one
   location their text already points at). Then drop a one-line note in
   their request dir saying the checklist is live. Timing fallback: their
   suggested sequencing runs the verification first, so if their evidence
   arrives before your checklist lands, diff the evidence against the
   checklist on receipt instead — same rigor, opposite order.
3. **Re-triage the Ms5 chain's blocked states.** The dep graph already
   proves the labels are conservative in at least one case:
   `m5-sdk-inject-point`'s only dependency is closed — its BLOCKED
   status is a blanket 2026-06-18 note, and its acceptance is pure unit
   tests. Unblock it and do the SDK-side OUT/IN detcall mechanics now.
   For `m5-host-log-fault-plan`, the question is whether its open dep
   edge on `ext-hyp-input-log-dev-events` should be re-scoped: the
   bead's own description scopes DHILOG serialization *out* (it stays
   hypervisor-owned) and defines the adapter over "supplied replay
   decisions" — buildable against synthetic decision fixtures you author
   yourself. Be clear-eyed that no recorded DHILOG fixtures exist in
   this repo; if you want real records for higher-fidelity tests, the
   hypervisor's M9 machinery holds sealed DHILOG artifacts — ask via
   their request dir rather than assuming. If a bead stays blocked,
   record *which checklist item* blocks it. The outcome is a bead graph
   where "blocked" is load-bearing, not conservative.
4. **Scaffold `determinism_replay`.** Build the test to the point where
   the only missing inputs are the external records: harness structure,
   fixture plumbing, the hash comparison across the four surfaces
   IMPLEMENTATION-PLAN §Ms5 names (RAM/framebuffer; ring contents / SDK
   events; drop counters; inject decisions via the LogLine digest),
   iteration loop, CI lane wiring (behind the existing
   `DETGUEST_VM_TESTS` discipline in `tests/vm/`), with the
   hypervisor-dependent steps stubbed — each stub citing the checklist
   item (from item 2) it awaits. Target: when the handoff lands, the
   remaining work is filling stubs and running the 1000-iteration
   acceptance, not designing a gate.
5. **Close the no-frame residual when the artifact exists.** When
   reference-workload's regenerated image lands (their request, filed
   today), run the **`refwork_ready_hold` no-timer twin** (the test gated
   on `REFWORK_READY_INITRAMFS` — *not* `no_timer_post_ready`, which is
   fixture-only and already green) against the real artifact and record
   the result in that request dir's resolution trail — converting a
   disclosed skipped-body guard into an exercised one.
6. **Receive the handoff.** When the hypervisor's evidence arrives:
   diff it against the item-2 checklist, flip or annotate both
   `ext-hyp-*` beads accordingly, and drop a one-line acknowledgment in
   their request dir (their acceptance criterion 3 literally waits on
   your acknowledgment). Cheap, and it is the hinge the whole
   convergence turns on.

## Suggested Sequencing (Yours To Overrule)

2 first (it sharpens someone else's in-flight work), then 1, then 3;
4 follows 3's findings. 5 and 6 fire on their respective arrivals
(refwork artifact; hypervisor evidence) whenever those happen.

## Acceptance Criteria

1. `guest-sdk-4bc` closed, with a harness test proving the substance: a
   child `Channel` re-seeded via the new accessors resolves a ring-event
   `name_id` *without* falling back to manifest bytes (the exact
   limitation `tests/vm/src/harness/snapshot.rs` documents); that note
   updated.
2. Both ext-hyp bead descriptions carry the checklist, covering at
   minimum: PAD_SET; DEV_EVENT for ring C/I pushes; ring A/W consumer
   bumps; `pio_answer`; replay-mode input-log application; the
   bit-identical Linux replay gate; Intel-VM-lane availability — each
   item evidence-citable. The hypervisor request dir has the pointer
   note.
3. Every Ms5-chain bead is either unblocked-and-started or carries the
   specific checklist item it waits on; any bead un-blocked by the
   triage has its work landed or in review.
4. The `determinism_replay` scaffold compiles and runs its
   non-hypervisor-dependent legs (fixture round trip, hash comparison
   self-test — including a deliberate-mismatch negative proving the
   comparison can fail); stubs enumerate exactly which checklist items
   they await.
5. The `refwork_ready_hold` real-artifact run (env var set, bodies
   executed) is recorded — pass, or a filed finding if it fails; either
   is a win over unexercised.
6. On handoff arrival: checklist diff done, both ext-hyp beads
   flipped/annotated, acknowledgment note in the hypervisor request dir.

## Out Of Scope For This Request

- Executing Ms5's in-VM acceptance and the 1000-iteration gate — that
  starts when the hypervisor handoff lands; this request is everything
  short of it.
- Ms3's formal in-VM acceptance beads — same external chain; they ride
  the same handoff and aren't separately staged here.
- Ms6 (quiesce/hardening/perf) — Phase 8 scope.
- Anything in reference-workload's or the hypervisor's requests — linked,
  not duplicated.
