# Real-Artifact Run: The Disclosed Skip Is Now Exercised

Recorded 2026-07-07 (phase3-ms5-groundwork-while-blocked item 5),
closing the residual `00-resolution.md` §"Residual Risk" disclosed: the
strengthened no-timer `refwork_ready_hold` assertion was compile-checked
but skipped because `REFWORK_READY_INITRAMFS` was unset.

## Artifact

reference-workload dist `workload-image-0.1.0`
(`../reference-workload/dist/workload-image-0.1.0/`, `built_from`
refwork `7b0c7b2`, `guest_sdk_rev acb1d3e8`):

- initramfs: `initramfs.cpio.zst` decompressed → blake3
  `36f50484f9fc1a8cfe6dd024dccac0a0ce4ab7f504b1e2cea357a00f97390b7d`
  (matches the determinism-hypervisor handback's cite for the same
  dist, byte-for-byte).
- kernel: the artifact ships its own `bzImage`, blake3
  `595466463a37efac6822ffccf3e61d0a2230e7d223a94c0bce5eb78b2f43bee9` —
  used via `REFWORK_READY_BZIMAGE` (not the default
  `image/build/bzImage`).

## Command (guest-sdk `c13ee1a`, host `infra-control`, 2026-07-07)

```bash
REFWORK_READY_INITRAMFS=<scratch>/refwork/initramfs.cpio \
REFWORK_READY_BZIMAGE=../reference-workload/dist/workload-image-0.1.0/bzImage \
DETGUEST_VM_TESTS=1 cargo test -p detguest-vmtest \
  --test refwork_ready_hold -- --test-threads=1 --nocapture
```

(The twins are env-gated, not `#[ignore]`d — a `-- --ignored`
invocation filters them out; run without it.)

## Result: PASS — both twins, bodies executed

```text
running 2 tests
test no_timer_real_harness_reaches_and_holds_ready ... ok
test real_harness_reaches_and_holds_ready ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 7.86s
```

Executed-bodies evidence: zero occurrences of the
`skipping refwork ready-hold` guard line in the `--nocapture` output
(verified with a grep-count re-run), and the 7.9 s wall time is two
full VM boots, not two skips.

The residual is closed: the timer twin and the strengthened no-timer
twin both reach and hold Ready against the real regenerated artifact.
No finding to file.
