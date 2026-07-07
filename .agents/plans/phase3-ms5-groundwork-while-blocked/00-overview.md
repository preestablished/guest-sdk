# Plan: Ms5 Groundwork While Blocked — Stage The Determinism-Replay Gate

Answers `.agents/requests/phase3-ms5-groundwork-while-blocked/` (phases
track, 2026-07-07). Read that directory first — this plan does not
repeat its context. Repo baseline: `main` at `db50f76`, clean tree.

## One fact changed since the request was filed

The request's item 2 planned for the hypervisor to verify against our
checklist; its stated timing fallback was "if their evidence arrives
before your checklist lands, diff the evidence against the checklist on
receipt instead — same rigor, opposite order." **That fallback is now
the operative order**: the determinism-hypervisor handback landed at
`a4d4e6e`/`db50f76`
(`.agents/requests/phase3-ext-hyp-input-log-and-replay-handoff/00-handback.md`,
dh rev `0831f92`), with an element-level verification matrix, and both
`ext-hyp-*` beads already carry appended NOTES pointing at it. The flip
decision is explicitly left to us ("the unblock decision is yours").

Consequence: request items 2 and 6 collapse into a single package —
write the checklist into the bead descriptions, diff the handback
matrix against it, flip/annotate both beads, drop the acknowledgment
note. It runs **first**, because the checklist item IDs it mints are
cited by every stub and every still-blocked bead downstream.

## Goal (behavioral)

When round 2 (`phase3-ms5-execution-in-vm-closeout/`) starts, Ms5 is a
**short execution, not a cold start**: the ready bead is closed, the
SDK/host inject mechanics exist with unit coverage, every blocked bead
cites the specific checklist item it waits on, and the
`determinism_replay` scaffold compiles and passes its self-test legs
(including a deliberate-mismatch negative) with only the
hypervisor-record-dependent steps stubbed.

## Packages

| File | Package | Request items | Gate |
|---|---|---|---|
| `01-checklist-and-handback-diff.md` | Checklist into ext-hyp beads; diff the arrived handback; flip/annotate; ack note | 2 + 6 | none — do first |
| `02-channel-reseed-accessors.md` | `guest-sdk-4bc`: intern-map / pending-inject re-seed accessors + harness re-seed | 1 | none |
| `03-ms5-retriage-and-unblocked-work.md` | Re-triage the Ms5 chain; implement `m5-sdk-inject-point` mechanics, `LogFaultPlan` adapter, host mutation-log audit | 3 | none |
| `04-determinism-replay-scaffold.md` | The `determinism_replay` test scaffold: four hash surfaces, iteration loop, negative self-test, stubs citing checklist IDs | 4 | packages 01–03 |
| `05-refwork-residual-and-ledger.md` | `refwork_ready_hold` real-artifact run; ring-a-doorbell-drain `03-resolution.md` ledger debt | 5 | refwork artifact (run leg only) |
| `06-verification-and-handback.md` | Acceptance mapping, resolution file, bd/git push | — | packages 01–05 |

Sequencing: 01 → 02 → 03 → 04, then 06. Package 05's ledger half is
ungated (do any time); its run half fires when reference-workload's
regenerated artifact lands and must not block the resolution — if the
artifact hasn't landed by then, record the gate state in the resolution
and leave the run leg open as the request's acceptance criterion 5
allows ("pass, or a filed finding — either is a win over unexercised"
— the unexercised case is what must not survive *once the artifact
exists*).

The request's suggested sequencing (2 → 1 → 3) predates the handback's
arrival; running the combined 01 first preserves its intent (sharpen
the contract before consuming it) under the new order.

## Load-bearing design decisions (argued in the packages)

1. **Checklist items get stable IDs** (`ILDE-*`, `DRL-*`) so stubs and
   bead annotations cite them mechanically, not by prose (package 01).
   Acceptance criterion 3 ("carries the specific checklist item it
   waits on") and criterion 4 ("stubs enumerate exactly which checklist
   items they await") both need a citable unit.
2. **Both ext-hyp beads flip only if the diff is clean per-item**; the
   handback's disclosed caveat (dh's fixture-era Linux corpus gate,
   their bead `determinism-hypervisor-jyo7`) is recorded in the flip
   annotation, not glossed (package 01).
3. **`m5-sdk-inject-point` is unblocked and implemented now** — its
   only dep (`m4-platform-readability-vm`) is closed, and its
   acceptance is pure unit tests. The detcall `IN` helper does not
   exist yet (`pio.rs` has only `detcall_out`); adding it is part of
   this bead, not a blocker (package 03).
4. **`m5-host-log-fault-plan` is re-scoped and implemented against
   self-authored synthetic decision fixtures.** Its own description
   already scopes DHILOG serialization out ("adapter over supplied
   replay decisions"). Its dep edge on `ext-hyp-input-log-dev-events`
   resolves via package 01's flip; if the flip is withheld, the bead
   stays annotated with the exact failed `ILDE-*` item (package 03).
5. **`m5-host-mutation-log-audit` is unblocked and implemented now** —
   its only dep is closed, it is host-only unit testing over the
   existing `ChannelWriteSink`/`RecordingSink`, and its "single
   ordered trace can replay all host-owned channel mutations"
   acceptance is exactly the input the scaffold's hash surfaces
   consume (package 03).
6. **The scaffold's self-test legs run ungated** (no KVM, no
   `DETGUEST_VM_TESTS`) so the phases track can re-run them from a
   clean checkout, per `03-verification-offer.md`. Only the in-VM leg
   uses the existing double-gate discipline (package 04).
7. **No `determinism_replay` CLI is requested from the hypervisor in
   this round.** The handback states no such binary exists and offers
   one on request; the scaffold drives fixtures directly, and the
   stale `intel-preflight.sh` probe message is updated to match
   reality after the flip (package 04). The CLI question belongs to
   round 2's gate execution if it surfaces at all.
8. **In-VM execution stays out.** `m5-vm-inject-roundtrip`, the
   1000-iteration gate, `m5-channel-reattach-checkpoint`'s in-VM
   verification, and `m3m5-ci-intel-vm-lanes` are round-2 scope
   (`phase3-ms5-execution-in-vm-closeout/` claims them explicitly);
   this plan annotates those beads with what they wait on and stops.
   Likewise round-2's prep item A (`ext-hyp-m9-linux-guest`
   disposition) is **not** pulled forward — it is round-2's declared
   ungated prep.

## Beads discipline

Before starting a package, claim its beads (`bd update <id> --claim`);
close with `bd close <id> --reason="..."` citing the commit SHA. All
annotation edits go through `bd update <id> --notes=...` (never `bd
edit` — it opens `$EDITOR` and blocks agents). `bd dolt push` + `git
push` at every session end, per `CLAUDE.md`.
