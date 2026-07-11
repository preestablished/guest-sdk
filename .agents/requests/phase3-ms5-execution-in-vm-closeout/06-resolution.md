# Phase-3 Ms3–Ms5 execution-in-VM resolution

Resolved 2026-07-11 on `main`. Final candidate before this resolution:
`9a5e4278aa02a4331e4d41a445e099d16e10ad43`.

## Delivered packages and pins

- Ms5 inject/reattach and replay: `c0faf2e` through `fce0c9c`; live KVM
  inject, checkpoint continuation, decoded-decision replay, all four replay
  surfaces, and the deliberate mismatch negative are real paths with no
  scaffold bypass.
- One-time replay acceptance: `ms5-1000-1f5901e`, exactly iterations/seeds
  0–999, 1000/1000 pass, ordered summary `fnv1a64:51855535a968a662`;
  manifest and reduced evidence are under `evidence/` beside this file.
- Ms3 direct input boundary: `f8e9713`; determinism-hypervisor
  `6e348e5961b8ba81d91b7bdd4f79af102b809649`, sealed `pad_echo_6s` DHILOG
  BLAKE3 `f6301f544ff1a0bc232688e232332e1d06a1e9d6cf3eac9d52b026bc1269718a`.
  The upstream `dh-inputlog::LogReader` decoded five `RecordBody::PadSet`
  values which each matched exactly one guest poll/log and never ring I.
- Ms4 stats/capture/churn: `ca02dcd`, `a4c7c87`, `f0ee6e0`. The exact
  0x46040-byte layout-v1 `detsdk.stats` region auto-registers and is host
  readable; the external CaptureSpec/refwork fixture is pinned; the full
  churn gate ran 600 seconds (607.26 seconds including setup/reverify), with
  every extent and manifest generation unchanged and zero P0 alarms.
- Reference workload image revision
  `7b0c7b2434e71d8b3241bf78597be457b281292d`; image manifest BLAKE3
  `af14040444db6f5e182f52193d71abdbbfb8085673b45da76c21dc541ac3dceb`;
  initramfs BLAKE3 `67f1ed...019b`; kernel BLAKE3 `595466...bee9`.
- Worker/VerifyReplay evidence revision
  `f855dfb9800e969e8371016112aace7703ee402d`. guest-sdk consumes neutral
  decoded values; determinism-hypervisor owns DHILOG and VerifyReplay.

## Campaign evidence

The 10-run timing probe completed in 65.743760446 seconds, with per-iteration
range 3.223776941–10.593978531 seconds and ordered summary
`fnv1a64:860a820bf4bfc264`. The resumable flagship campaign then completed in
ten chunks: 4,003.362 measured iteration seconds, range 1.885–14.047 seconds,
10,000 input updates, and 10,000 decisions in each Proceed/Platform/Workload
class. Full RAM, drained raw events, drop counters, and inject-decision
digests were present for every record. The separate real-path perturbation
was rejected at the named `final guest RAM` surface.

Raw records remain at
`/home/infra-admin/evidence/guest-sdk/ms5-1000-1f5901e/`. Raw manifest BLAKE3
is `32ebed...3202d`; committed manifest BLAKE3 is `bdd76c...76a7`; ordered
record-checksum stream BLAKE3 is `be5a44...30b3c`.

## Clean-checkout recurring proof

[GitHub Actions run 29164703369](https://github.com/preestablished/guest-sdk/actions/runs/29164703369)
is green at `9a5e427`. Every hosted lane and the push-only
`[self-hosted,intel,kvm]` lane passed. Artifact `intel-vm-29164703369-1`, ID
`8251914275`, 30-day retention, digest
`sha256:2ced7bdc1121590bc91c5f2f03087952a710e6b5c147db8ec4c43a1caea80845`
was downloaded and inspected. It contains the decoded PAD_SET artifact, all
named Ms3/Ms4/Ms5 logs, and replay records 0–9. The recurring manifest BLAKE3
is `277f07c0de83d3140a0e999817f202f11ca766130cd9d5ea443df4e4bf1961f9`;
ordered summary `fnv1a64:b23a7c88490cfb41`.

The recurring budget is exactly ten iterations, deliberately distinct from
the one-time 1000-run campaign. The CI M4 log proves 100/100 branches and the
full 600-second churn body; no ignored-test skip is counted as success.

## Final quality gates

| Gate | Result |
|---|---|
| `cargo fmt --all --check` | pass, 0.884 s |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass, 36.544 s |
| `cargo test --workspace` | pass, 13.509 s |
| `cargo test -p detguest-wire --no-default-features` | pass, 2.374 s |
| Miri ring suite | pass, 9 tests, 31:21.91 local and run 29164703369 |
| Loom release suite | pass, 3 tests, 1:00.65 local and run 29164703369 |
| musl release build, agent `--check`, static-pie linkage | pass locally and in run 29164703369 |
| `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps` | pass, 0.934 s |
| default `scripts/intel-preflight.sh` | pass, 6.860 s |
| Ms4 100 branches + exact 600-second churn | pass locally and in run 29164703369 |
| real-path perturbed-decision negative | pass, 12.579 s; named `final guest RAM` |
| final named clean-checkout Intel workflow | pass, run 29164703369 |

Tracker hygiene: `bd lint` reports no template warnings and `bd preflight`
returned its readiness checklist with exit 0. `bd doctor --check=conventions`
returned exit 0 but explicitly reports that doctor is not supported by the
configured embedded backend; no stronger doctor result is claimed.

## Tail bead disposition

| Bead | Disposition/evidence |
|---|---|
| `guest-sdk-m3-input-path-acceptance` | closed; upstream decoded artifact + exact KVM polls (`f8e9713`) |
| `guest-sdk-m5-determinism-replay-ci-gate` | closed; 1000 campaign + recurring artifact |
| `guest-sdk-m5-reference-workload-contract-tests` | closed; compatibility matrix and current fixture |
| `guest-sdk-m5-reference-workload-20run-gate` | closed; upstream 20/20 report BLAKE3 `a06051...13c3` |
| `guest-sdk-m4-capture-contract-tests` | closed; pinned capture handoff fixture (`a4c7c87`) |
| `guest-sdk-m4-sdk-stats-region-autoreg` | closed; fixed ABI and host read (`ca02dcd`) |
| `guest-sdk-m4-reverify-churn-test` | closed; exact 600-second gate (`f0ee6e0`) |
| M3/M4/M5 documentation beads | closed; as-built contracts (`9a5e427`) |
| `guest-sdk-m3m5-final-quality-gates` | closes with this resolution and green run |
| milestone epics/root and handoff | close after repository + Beads push verification |

## Phase-3 exit-gate citation

Authoritative phase document:
`/home/infra-admin/git/preestablished/determinism-hypervisor/.agents/docs/phases/phase-3-workload-in-the-box.md:75`,
exit-gate item 2 at lines 79–80: guest-sdk Ms4 host readability stable across
100 snapshot/restores and Ms5 `determinism_replay` CI green. Evidence is the
1000-run index plus green clean-checkout run 29164703369 and artifact
8251914275 above. Reference-workload exit-gate item 1 is recorded by the
closed 20-run handoff bead.
