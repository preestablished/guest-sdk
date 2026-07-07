# Package 05 — The Refwork Real-Artifact Residual And The Ledger Debt

Two small independent items; neither blocks packages 01–04.

## A — Run `refwork_ready_hold`'s no-timer twin against the real artifact (request item 5)

**Gate: reference-workload's regenerated image** (their request
`phase3-m4-first-room-gate-and-m5-stamp`, filed 2026-07-07, regenerates
`dist/workload-image-0.1.0/`'s initramfs). Note: the determinism-
hypervisor handback already cites a `workload-image-0.1.0` dist
(initramfs blake3 `36f50484…`, contains `usr/bin/refwork-harness`) —
**check at execution time whether that artifact already exists** at
`../reference-workload/dist/workload-image-0.1.0/` (or wherever their
dist convention puts it); if so, the gate is already open and this leg
runs now. Record which artifact (path + blake3) was used either way.

The precision trap the request flags: the target is
`tests/vm/tests/refwork_ready_hold.rs` — **not**
`no_timer_post_ready.rs`, which is fixture-only and already green. The
test self-skips (whole body, every assertion) when
`REFWORK_READY_INITRAMFS` is unset; that skip is the disclosed
residual being closed.

Run (on the Intel lane host; both twins, the no-timer one is the
strengthened-assertion residual):

```bash
REFWORK_READY_INITRAMFS=<path-to-real-initramfs> \
DETGUEST_VM_TESTS=1 cargo test -p detguest-vmtest \
  --test refwork_ready_hold -- --test-threads=1 --nocapture
```

(`REFWORK_READY_BZIMAGE` defaults to `../../image/build/bzImage`;
override only if the artifact ships its own kernel — record which was
used.) Confirm from the output that the bodies **executed** (the
"skipping refwork ready-hold" line must be absent) — an accidental
still-skipped green would recreate exactly the residual this item
closes.

Record the result — pass, or a filed finding if it fails (`bd create`
+ a note; either outcome satisfies acceptance criterion 5) — in the
resolution trail of
`.agents/requests/phase3-post-ready-no-frame-under-no-tick/` (the
request dir whose `00-resolution.md:60-68` disclosed the skip),
continuing that dir's file numbering. Cite artifact hashes, revs, and
the exact command.

If the artifact still doesn't exist when this plan's other packages
finish: say so in the resolution file (package 06) and leave this leg
explicitly open with its gate named. Do not hold the handback for it.

## B — Close the ring-a-doorbell-drain ledger debt

Provenance note: this is not one of the request's six numbered items —
it comes from the request's own `01-current-state.md` ("a two-minute
ledger item worth closing while you're here"). It is deliberate,
bounded scope; it must not grow beyond the historical record described
below, and its absence would not block the handback.

`.agents/requests/phase3-ring-a-doorbell-drain/02-fix-and-verify.md:58-60`
promised a `03-resolution.md` "with your routing decision and the fix";
the fix was folded into the boot-scheduling-deadlock work and the file
was never written. Write it now — it is a two-minute historical record,
not new work:

- The routing decision taken, one paragraph.
- Pointer to the commits that carried the fix (they rode the
  boot-scheduling-deadlock resolution; cite the SHAs from that request
  dir's resolution/verification files rather than re-deriving).
- Pointer to the real-worker verification that covered it
  (`1f9a123` "Verify Fix A on the real worker" and the deadlock
  request's verification file).

Ungated; do it in the same session as any other package.

## Done when

- A: the real-artifact run is recorded (executed-bodies evidence,
  artifact hashes, pass/finding) in the no-frame request dir — or the
  resolution file documents the still-closed gate.
- B: `03-resolution.md` exists in the ring-a-doorbell-drain dir.
