# Ms5 1000-Iteration Evidence Index

Run `ms5-1000-1f5901e` completed on `infra-control` with 1000 unique,
consecutive green iterations (IDs and seeds 0–999). The committed manifest
pins the runner, repository/worker/workload revisions, image, initramfs,
kernel, exact test binary, schema/generator, seed mapping, and requested
range. The reduced summary records the campaign census and final digests.

Raw single-writer records and chunk logs are retained at:

```text
/home/infra-admin/evidence/guest-sdk/ms5-1000-1f5901e/
```

The raw corpus is 4.3 MiB and contains `records/000000.json` through
`records/000999.json`, `manifest.json`, and chunk logs. Owner: guest-sdk
Phase-3 acceptance maintainers. Local retention: through Phase-3 handoff
verification. The trusted push-only Intel workflow publishes a separate,
small recurring sample with explicit retention; it proves clean-checkout CI
wiring without pretending to rerun or duplicate the one-time 1000-iteration
lab corpus.

Integrity:

- manifest BLAKE3:
  `32ebed7b54c99d66415e010274332d95e58d6c686bbdbb2fe4f8228e9e73202d`
  (raw atomic file, no trailing newline); committed JSON copy BLAKE3:
  `bdd76cac67465802c3b53ecc1eed72509bf8d91e91355f9bc662246dac6d76a7`
  (same JSON content with a repository newline)
- ordered stream of per-record BLAKE3 lines:
  `be5a44c1f9f0c6f8215d44c4b42f940ebc034d8cd60e409daf99f60df9430b3c`
- ordered campaign summary:
  `fnv1a64:51855535a968a662`

The deliberate real-path negative separately passed at commit `c082357`,
rejecting one perturbed decoded decision at the named `final guest RAM`
surface.

## Recurring clean-checkout CI proof

Push run [29163502130](https://github.com/preestablished/guest-sdk/actions/runs/29163502130)
at guest-sdk `fdda5b390566ad45d157a079b30494e5f7d8066f` passed the named
Intel job on runner `intel-box`. Its retained artifact
`intel-vm-29163502130-1` (artifact ID `8251516481`, 30-day retention) has
GitHub digest
`sha256:3f25481bd31a004fe6c311a6f238b28be4a0ffff19c700df3b1b7b6c547602b7`.
The downloaded replay manifest BLAKE3 is
`06cb8797bdc198c940e7bc56fc4e2775f0f4b4c1729d1005535f8c85f127235f`.

The manifest proves exact iterations/seeds 0–9, all pass, ordered summary
`fnv1a64:f84f13e8a0fd9916`. Its ten records contain 100 input bursts and 100
decisions in each Proceed/Platform/Workload class, with all four authoritative
surface digests and the pinned external VerifyReplay reference present. The
artifact also contains the preflight and every named Ms3/Ms4/Ms5 log; the
workflow step conclusions and downloaded bodies confirm that none of the
ignored KVM tests green-skipped.
