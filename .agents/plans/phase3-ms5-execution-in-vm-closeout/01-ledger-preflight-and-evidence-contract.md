# Package 01 — Ledger, Preflight, Image Pin, And Evidence Contract

This package prevents stale ledger state or host folklore from contaminating
the lab run. The filing proposed `04-prep-notes.md`, but `04-` is now occupied
by the status update, so produce `05-prep-notes.md` before feature work.

## A. Re-diff and reconcile the three stale beads

For each bead, record upstream repository SHA, evidence path, artifact digest,
date/host, and the exact acceptance sentence it satisfies. Verify paths and
revisions exist locally; re-run the cheapest upstream validator where one is
documented.

1. `guest-sdk-ext-hyp-m9-linux-guest`: diff its description against the
   hypervisor M9 final acceptance and the current real-image capability
   evidence. Close if Linux M9 is shipped and usable by this lane; otherwise
   replace the generic block note with the exact missing surface.
2. `guest-sdk-ext-refwork-m5-full-suite`: validate the 20/20 handoff comment,
   green stamp, suite report digest, image revision, and artifact root in the
   current reference-workload checkout. Close when the external capability is
   proven.
3. `guest-sdk-m5-reference-workload-20run-gate`: do not close merely because
   item 2 closed. First complete or disposition its sibling contract-test
   dependency; then cite the validated 20-run evidence and close it.

Keep the hypervisor's stale real-image corpus/regression debt (`jyo7`/`i74w`)
in the relevant notes as a separate caveat. It must neither re-block an
already-proven capability nor disappear from the record.

After edits, run `bd ready`, `bd blocked`, and `bd show` for the affected
downstream beads. Capture the before/after graph in prep notes.

Identify the exact callable decoder/API or artifact contract for decoded
PAD_SET and inject-decision records, including upstream revision and command.
Guest-sdk currently has no DHILOG or VerifyReplay client. If no callable
boundary exists, record that upstream gap and block only the work that needs
it; do not invent an in-repo DHILOG parser or call an aspirational API present.

## B. Settle current Intel preflight without changing host state

Run both forms and save complete output plus exit status:

```bash
./scripts/intel-preflight.sh
./scripts/intel-preflight.sh --require-host-hugepages
```

Also record the 2 MiB hugepage sysfs totals and inspect actual memory allocation
in guest-sdk `tests/vm` and the current hypervisor canonical lane. The decision
is evidence-driven:

- If no canonical test uses host hugetlb pages, remove the misleading
  hypervisor-use claim from `scripts/intel-preflight.sh` and
  `docs/ci/intel-runner.md`, retain the optional diagnostic only if another
  named consumer exists, and update stale bead notes to say host hugepages are
  not an entry gate.
- If the canonical lane really maps host hugepages, cite the exact code/config
  consumer and required count. File the operator ask through the hypervisor's
  audited host-config request path; do not write sysfs, invoke `sysctl`, or edit
  shared-host boot configuration from this repo.

The default preflight must pass before booking the lab window. Record kernel
provenance as observed; do not resurrect the filing-time stale-kernel claim if
the current check is green.

## C. Pin the real image and protect the window

Prefer the regenerated reference-workload image named by the handoff. Record:

- guest-sdk, reference-workload, hypervisor/worker, and control-plane SHAs;
- absolute image path, green-stamp contents, and image/initramfs digest;
- kernel provenance/build key and Intel runner identity;
- worker/service revision and confirmation that no refwork cutover overlaps
  the proposed timing probe or 1000-run chunks.

Validate/register the image using the reference-workload handoff's documented
commands. Do not copy it into a second untracked location without recording the
new digest.

## D. Define evidence before writing the long loop

Use a configured evidence directory outside disposable test temporaries for
the raw run. Add a `tests/vm/src/evidence.rs` single-writer API (and only the
minimal serialization/hash dependencies needed) that emits atomic records containing at
least: schema version, run ID, iteration, seed, input-burst census, fault-plan
census, four surface digests, end-state/VerifyReplay reference, revisions,
image digest, timestamps/duration, and pass/fail/divergence. A manifest records
requested range, completed ranges, resume points, and summary hashes.

Resume must validate runner ID; all repository/worker SHAs; image, initramfs,
kernel, and test-binary digests; generator/schema version; seed mapping; and
range. Reject gaps, duplicates, overlap, or drift; a worker/service revision
change invalidates continuation. Enforce a lock/single writer, sync the
temporary record, rename, then sync the directory. Raw corpus goes to an
approved durable store with URL, retention, owner, and digest; commit the
schema, manifest, reduced summary, and evidence index. Record disk-space and
cleanup policy, then verify a clean-checkout download and digest before close.

## Done when

`.agents/requests/phase3-ms5-execution-in-vm-closeout/05-prep-notes.md`
(continue after the request's existing `04-current-status...` file) records the
three bead diffs, current preflight outputs/outcome, pinned image/revisions,
window constraint, and evidence-root/schema decision. The relevant beads are
then genuinely ready or precisely blocked.
