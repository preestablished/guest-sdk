# Request: Execute Ms5 + Ms3 In-VM Acceptance — The Phase 3 Tail, Closed Together (Gated)

## Who Is Asking

The phases track, round 2 (2026-07-07). This is the successor that
round-1's own out-of-scope section promised: "Executing Ms5's in-VM
acceptance and the 1000-iteration gate — that starts when the
hypervisor handoff lands; this request is everything short of it."
Round-1 (`phase3-ms5-groundwork-while-blocked/`) is unexecuted as of
this filing; it remains the predecessor, not an alternative — which
makes this request realistically two request-cycles out. It is filed
now because two of its items are ungated prep worth starting today,
and because the lab-window planning (a potentially multi-hour gate
run) needs lead time.

## Why guest-sdk, Why This Chunk

Phase 3 exit gate 2 requires "Ms5 `determinism_replay` CI gate green" —
the single remaining starred obligation this repo owns. And the tail is
bigger than Ms5 alone: `m3m5-ci-intel-vm-lanes` (P0) transitively
depends on `m3-vm-real-workload-e2e`, so the CI-lane bead **cannot
close without the Ms3 e2e acceptance landing too**, and
`m3-input-path-acceptance` unblocks on the *same* external bead as the
Ms5 fault-plan work. One in-VM push closes the whole Phase-3 tail;
splitting it would pay the lab-session overhead twice.

Phase 4 asks nothing else of this repo: it appears in Phase 4 only via
the entry requirement's real captures — which are the *output* of
exactly this acceptance work. Finishing Phase 3 in-VM acceptance *is*
guest-sdk's Phase-4 debt.

## Entry Conditions Preview (Full List In `02-`)

Four distinct gates, one of them owned by nobody yet and flagged hard:

1. Round-1's groundwork (4bc, inject-point mechanics, fault-plan
   adapter, the `determinism_replay` scaffold).
2. The hypervisor DHILOG/replay handoff received (both checklist beads
   flipped) — their round-1 item 3.
3. **`ext-hyp-m9-linux-guest`** — a *third* external bead the two-bead
   handoff never covered. The hypervisor's M9 is long done (final
   acceptance 2026-06-21), so this is almost certainly another
   stale-verification flip — and **we disposition it ourselves**
   against their M9 evidence, folding it into the receiving-side diff
   that round-1 item 6 defines; the hypervisor gets an FYI, not new
   work, unless the diff finds a real gap.
4. **Intel preflight — split into its two true halves.** The
   `m3-vm-real-workload-e2e` bead NOTES record: missing reserved 2 MiB
   hugepages + stale cached kernel provenance. These are different
   problems: the provenance is **guest-sdk-local** (the preflight's own
   fix hint is `./image/build.sh kernel` — a repo rebuild, ungated, do
   now), while the hugepage claim is **unconfirmed** — this repo's own
   preflight comment attributes the need to the hypervisor harness,
   but a sweep of the hypervisor repo finds no hugepage usage at all
   (their host posture is THP-off). So the hugepage leg is a
   *reconciliation outcome*: either the comment is stale and the leg
   dissolves, or the need is real and the change routes through the
   hypervisor's audited host-config regime
   (`docs/ops/host-config-intel-box.md` / `apply-host-config.sh`) via
   a one-line ask in their request dir, operator-executed. guest-sdk
   is tracker-of-record; it does not mutate the shared Intel box
   out-of-band.

## The Ask In One Paragraph

When the gates hold: land the in-VM inject round trip
(`m5-vm-inject-roundtrip` + its side-chain), fill the scaffold's stubs
with the real DHILOG-backed records, run the flagship
`determinism_replay` acceptance — 1000 consecutive iterations with
varied fault plans, bit-identical across the four §Ms5 hash surfaces —
land the two Ms3 acceptance beads (`m3-vm-real-workload-e2e`,
`m3-input-path-acceptance`) in the same lab window, wire
`m3m5-ci-intel-vm-lanes` into the `in_vm` CI lane, and update the
refwork/guest-sdk handoff files so Phase 3 exit gate 2 is citable green.

## Files In This Request

| File | Contents |
|---|---|
| `01-current-state.md` | The bead graph, the four gates, host-preflight evidence |
| `02-requested-work.md` | Entry conditions, the ask, acceptance criteria, out of scope |
| `03-verification-offer.md` | Cross-request choreography and handback |
