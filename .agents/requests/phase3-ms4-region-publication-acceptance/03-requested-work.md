# Requested Work

## What We Need (Behavioral)

Ms4 acceptance as the phase plan states it: **emulator RAM and framebuffer
regions published by a real workload are readable from the host and stable
across 100× snapshot/restore**, on the Intel lab machine, with durable
evidence. Concretely:

1. `detguest-sdk`'s public region registration path actually performs the
   Ms4 mechanics (mlock, prefault, pagemap GVA→GPA translation, registration
   with the agent over IPC) instead of returning a validated no-op handle —
   so that what the manifest advertises is what the kernel guarantees.
2. `detguest-agent` `ReverifyRegions` re-validates published regions
   (translation still correct, pages still resident) rather than
   no-op-succeeding, and the restore/fork path exercises it.
3. The `guest-sdk-m4-platform-readability-vm` acceptance test exists and is
   green: a VM workload publishes `wram` + `framebuffer` (D7 length —
   229,376 bytes, see `02-…`) + `meta`; the host reads them through the
   manifest; contents are exact and stable across 100 snapshot/restore
   branches; readability holds after restore and after fork.

## Suggested Sequencing (Yours To Overrule)

Your bead graph already encodes most of this; from our vantage the order
that unblocks downstream soonest is: agent IPC protocol/server → real
`register_region` → `ReverifyRegions` → the 100× VM acceptance → then the
joint refwork M4 bring-up (their unblock checklist items 2–3 cite your GS-5/
GS-6 for the real workload path, so plan that convergence with the
reference-workload owner as the phase doc suggests — "same agent or tight
pairing").

While you are in the VM-test area: bump the staged fixture's framebuffer to
the D7 length (`02-…` item 1) even before the real workload path lands, so
existing staged-fixture flows stop tripping the new length check.

## Acceptance Criteria

Verified by you (CI / lab, durable artifacts under a recorded artifact
root):

1. Real registration path covered by tests that would fail if mlock,
   translation, or agent registration silently regressed to a no-op.
2. `ReverifyRegions` verified non-no-op (a deliberately corrupted/unmapped
   region is detected).
3. The 100× snapshot/restore readability acceptance green on the
   `infra-control-kvm-intel` runner, artifact pointers + hashes recorded
   (same evidence discipline as the hypervisor's M9 acceptance).

Verified by us / jointly (after refwork M4 regenerates a READY snapshot with
the real workload):

4. Through the deployed worker gRPC: `RestoreSnapshot → GetFramebuffer`
   returns 229,376 bytes of XRGB8888 as a valid frame (black frame
   acceptable pre-first-render).
5. The operator bridge browser preview renders the frame — the human-visible
   half of phase exit gate item 3.

## Out Of Scope For This Request

- The operator-game lab run itself (ROM hash, first-room padlog, the
  implemented `refwork-verify vm-first-room` command) — that is
  reference-workload package-05 territory; this request is the guest-sdk
  half that gates it.
- Ms5 (`determinism_replay` CI gate) — sequenced after Ms4; we mention it
  only because the exit gate bundles them.
- snapshot-store M7 GC — independent Phase 3 work, not yours.
