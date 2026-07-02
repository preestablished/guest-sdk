# Request: Close Ms4 Region Publication So Phase 3 Can Be Validated

## Who Is Asking

The `rom-operator-bridge` project, acting as the Phase 3 validation surface
(browser-based operator control of real sessions against the deployed
`dh-workerd`). Filed 2026-07-02.

## Why guest-sdk, Why Now

Per the phase plan (determinism docs,
`phases/phase-3-workload-in-the-box.md`), guest-sdk **Milestone 4 is the ⭐
milestone of Phase 3** and appears directly in the phase exit gate:

> guest-sdk Ms4 acceptance: emulator RAM region readable from the host and
> stable across 100× snapshot/restore; Ms5 `determinism_replay` CI gate green.

Everything upstream of you is done or waiting on you:

- hypervisor M9 (Linux guest) — done, final acceptance evidence at
  `../determinism-hypervisor/target/m9-final-acceptance-20260621T004402Z/`.
- hypervisor DH-2 (scheduled input) and DH-5 (capture/region read) —
  implemented and fixture-tested.
- reference-workload M3 (harness vs mock agent) — done; the package-04 image
  handoff (`dist/workload-image-0.1.0/`, manifest hashes recorded) — present.
- reference-workload M4 (in-VM bring-up) is **joint with your Ms4** and is
  recorded as BLOCKED on your M4 gaps in
  `../reference-workload/.agents/plans/guest-sdk-unblock-reference-workload/m4-in-vm-first-room-evidence.md`.

Your own beads already track the work (`guest-sdk-m4-platform-readability-vm`
P0, `guest-sdk-m4-agent-ipc-protocol`/`-server` P1, `guest-sdk-m3m5-ci-intel-vm-lanes`
P0). This request adds the cross-repo context, one new contract change you
need to absorb (see `02-…`), and what the bridge offers for verification.

## The Ask In One Paragraph

Close the three recorded GS-6/Ms4 blockers — (1) the SDK's standalone
`register_region` no-op handle becomes the real mlock + prefault +
pagemap-translation + agent-IPC path, (2) `detguest-agent`'s
`ReverifyRegions` stops being a no-op, (3) the Intel VM acceptance test
proving published regions are readable from the host across 100
snapshot/restore branches runs green — while absorbing the hypervisor's new
framebuffer region contract (geometry from `layout_version`, no in-region
descriptor; determinism-hypervisor `5698d7e`). That unblocks refwork M4, the
first-room gate, and the READY snapshot the operator bridge needs to render
its first real frame.

## Files In This Request

| File | Contents |
|---|---|
| `01-current-state.md` | Evidence-based state: what is done, the three blockers with file references |
| `02-framebuffer-contract-change.md` | New since your last sync: hypervisor `5698d7e` and what it invalidates |
| `03-requested-work.md` | The ask, suggested sequencing, acceptance criteria |
| `04-verification-offer.md` | What the bridge provides to verify the result end-to-end |
