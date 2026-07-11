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
verification and the linked recurring-CI proof. Package 05 must upload this
directory through the trusted push-only Intel workflow with explicit
retention; that artifact URL becomes the clean-checkout download reference.
Until that linked workflow artifact exists and its digest is reverified, the
flagship code gate is proven locally but its Bead remains open.

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
