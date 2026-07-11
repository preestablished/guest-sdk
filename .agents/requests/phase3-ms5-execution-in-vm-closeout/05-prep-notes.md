# Package 01 Prep Notes — 2026-07-11

## Ledger reconciliation

| Bead | Evidence re-diff | Disposition |
|---|---|---|
| `guest-sdk-ext-hyp-m9-linux-guest` | determinism-hypervisor `target/m9-final-acceptance-20260621T004402Z/`, tested SHA `f855dfb9800e969e8371016112aace7703ee402d`; Linux Ready 1/1, worker API 2/2, Linux M5 corpus 1/1 | Closed 2026-07-11. The separate real-image corpus debt `determinism-hypervisor-jyo7`/`i74w` remains disclosed and does not negate shipped M9 capability. |
| `guest-sdk-ext-refwork-m5-full-suite` | reference-workload `.agents/plans/guest-sdk-unblock-reference-workload/m5-suite-evidence.md`; image rev `7b0c7b2434e71d8b3241bf78597be457b281292d`, worker `30d0cb9`; report `target/m5-acceptance-20260707/vm-suite-real-20x-report.json` BLAKE3 `a06051df0ce076daa49f48298b25959b7a83dac8deb23cf247177f6c2bbe13c3` | Closed 2026-07-11 after the local report digest matched the handoff. |
| `guest-sdk-m5-reference-workload-20run-gate` | Upstream 20/20 is proven as above. Its sibling guest-sdk contract-test bead remains live, so this downstream bead remains blocked until that acceptance is completed. | Not administratively closed from upstream evidence alone. |

Before reconciliation, the two external beads were explicitly `BLOCKED` and
the downstream Ms3/Ms5 work was not ready despite landed handoffs. After the
evidence-bearing closures and transition of stale explicit statuses, the live
inject, determinism, Ms3, and M4-tail implementation beads became ready. The
channel-reattach and live-inject beads subsequently closed with focused KVM
evidence at guest-sdk commits `c0faf2e` and `293e165`.

## Decoded input and VerifyReplay boundary

DHILOG serialization and validation remain hypervisor-owned at tested revision
`f855dfb9800e969e8371016112aace7703ee402d`:

- `crates/dh-inputlog/src/reader.rs` exposes validated `RecordBody::PadSet`;
- `crates/dh-worker/tests/m5_record_replay.rs` demonstrates the concrete
  worker `VerifyReplay` gRPC stream and `Done.end_state_hash` contract; and
- `docs/phase-2-exit-gate.md:190-192` records Linux inject replay and the
  1000-child VerifyReplay evidence.

There is no shipped standalone VerifyReplay CLI or guest-sdk client. The
guest-sdk gate therefore consumes decoded `LoggedDecision` values through
`LogFaultPlan`, never parses DHILOG bytes, and records the exact external
VerifyReplay evidence reference in every iteration. PAD_SET acceptance must
likewise consume a neutral decoded adapter backed by the hypervisor reader;
direct `PvPad::schedule` alone is not sufficient evidence.

## Intel preflight and hugepage conclusion

Commands run on `infra-control` on 2026-07-11:

```text
./scripts/intel-preflight.sh
  PASS: VT-x, /dev/kvm API 12, perf_event_paranoid=1, cargo 1.97.0,
  musl target, kernel 6.12.93 provenance/config, pv-pad, replay/snapshot.

./scripts/intel-preflight.sh --require-host-hugepages
  FAIL: 2 MiB pool empty (0 free / 0 total).
```

The 2 MiB sysfs values were `nr=0`, `free=0`, `resv=0`, `surplus=0`.
guest-sdk `tests/vm` maps anonymous host RAM and obtains its hugepages from
the guest-internal `hugepages=4` command line. No canonical hypervisor lane
consumer of host hugetlb pages was found. Host hugepages are therefore not an
entry gate and the optional script/docs attribution is stale; package 05 will
remove that misleading consumer claim without changing host state.

The replay fixture also exposed the quick-TSC calibration fallback as a real
pre-userspace flake. `VmConfig` now pins `lpj=4096` for every harness boot;
normal lanes retain PIT scheduling, while timerless replay children suppress
IRQ0 only after the continuous workload has reached the root snapshot.

## Image and revision pin

| Component | Pin / digest |
|---|---|
| guest-sdk probe implementation | `fce0c9c` |
| reference-workload image revision | `7b0c7b2434e71d8b3241bf78597be457b281292d` |
| reference-workload current checkout at reconciliation | `4eb8a3a99197ae9e937c544ed0e4d320ee9da546` |
| determinism-hypervisor tested worker | `f855dfb9800e969e8371016112aace7703ee402d` |
| hypervisor current checkout at reconciliation | `6e348e5961b8ba81d91b7bdd4f79af102b809649` |
| image manifest BLAKE3 | `af14040444db6f5e182f52193d71abdbbfb8085673b45da76c21dc541ac3dceb` |
| reference initramfs BLAKE3 | `67f1ed56769cd3f05b294c18068fb0c75d547da03ec2d6bc34bc127dd04c019b` |
| reference bzImage BLAKE3 | `595466463a37efac6822ffccf3e61d0a2230e7d223a94c0bce5eb78b2f43bee9` |
| green stamp BLAKE3 | `5728f328ebd650b1245cdd87f5e404ea73e98d0a7bfb20467a5e22a90d700348` |
| runner | `infra-control` |

The reference-workload handoff states that no worker/refwork cutover may
overlap a campaign. Every resumed chunk pins the worker revision in the
manifest; any service revision change starts a new run rather than mixing
evidence.

## Evidence contract and timing probe

`tests/vm/src/evidence.rs` schema version 1 uses one exclusive writer lock,
synced temporary files, atomic rename, and directory sync. Resume validates
runner, guest-sdk/worker/workload revisions, image/initramfs/kernel/test-binary
digests, generator/schema, seed mapping, and the full requested range. Gaps,
duplicates, overlap, identity drift, and range drift are rejected. Raw
campaign evidence lives outside disposable test directories under
`/home/infra-admin/evidence/guest-sdk/`; committed resolution material will
carry the reduced manifest/summary and durable-store index.

The production-evidence probe at
`/home/infra-admin/evidence/guest-sdk/ms5-probe-fce0c9c/` completed 10/10:

- campaign time: 65.743760446 seconds;
- total test time including setup: 70.96 seconds;
- per-iteration range: 3.223776941–10.593978531 seconds;
- raw record count: 10 consecutive iteration IDs 0–9;
- every iteration covered Proceed, Platform, Workload and ten seeded input
  updates, with equality across full 128 MiB guest RAM, complete raw event
  records, drop counters, and inject-decision LogLines; and
- ordered summary: `fnv1a64:860a820bf4bfc264`.

Linear projection is about 1 hour 50 minutes for 1000 iterations before
setup and margin. The lab campaign will use resumable chunks with at least one
chunk of rerun margin. The recurring push lane must use a much smaller exact
budget selected after the full campaign; it will not run 1000 on every push.

## Flagship campaign completion

Run `ms5-1000-1f5901e` completed after the probe in ten resumable chunks.
The reducer proved exactly 1000 unique consecutive green records, iteration
IDs and seeds 0–999, zero non-pass outcomes, 10,000 decisions in each of the
Proceed/Platform/Workload classes, 10,000 seeded input updates, and no missing
authoritative surface digest. Total measured iteration time was 4,003.362
seconds; the per-iteration range was 1.885–14.047 seconds. Final ordered
summary: `fnv1a64:51855535a968a662`.

Committed reduced evidence and the exact manifest are under `evidence/` next
to this file. Raw records/chunk logs remain at the durable local evidence root
pending package-05 upload by the trusted push-only Intel workflow; the
flagship Bead is intentionally not closed until that artifact URL and a green
recurring workflow proof exist.
