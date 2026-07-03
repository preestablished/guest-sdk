# Verification

## Reproduce The Gap (two minutes, no worker needed)

```sh
cd ~/git/preestablished/reference-workload && cargo run -q --locked -p xtask -- image build
cd ~/git/preestablished/guest-sdk
zstd -qf -d -o /tmp/probe-initramfs.cpio \
  ~/git/preestablished/reference-workload/dist/workload-image-0.1.0/initramfs.cpio.zst
BOOT_PROBE_INITRAMFS=/tmp/probe-initramfs.cpio \
  cargo test -p detguest-vmtest --test boot_probe -- --nocapture
```

The probe (`tests/vm/tests/boot_probe.rs`, added by this session — env
gated, inert in plain `cargo test`) prints full serial plus drained
guest events; today the last event is the harness's
cannot-read-`/dev/vdb` fault. Caveat: the probe harness has **no
pv-blk device**, so under it a correct implementation will fault at the
pv-blk read instead — the probe's job is layer-by-layer visibility, not
end-to-end success. End-to-end goes through the real worker:

```sh
# the Phase 3 step-2 invocation (ops doc rom-bridge-o73-ready-snapshot.md,
# scratch paths; ask the bridge session for the exact scratch recipe)
dh-m9-ready-handoff ... # with DH_M9_GAME_IMAGE staged
```

## What Green Looks Like

1. Your VM tier: a test where the agent materializes a known game image
   via pv-blk and the unit reads it back byte-exact (checksummed —
   the fixture's readback checksum sets the precedent), plus a loud
   distinct fault when pv-blk is absent/corrupt. Negative tests per the
   ecosystem convention (shown to fail with the guard reverted).
2. The real-worker handoff (bridge side runs it on your word): boot →
   `Hello` → `WorkloadStarted` → LoadGame succeeds → regions register →
   **`Ready` at a recorded icount** — the step-2 exit evidence (READY
   icount, region count/manifest generation, state hash), unblocking
   READY-snapshot regeneration (step 3) and everything after it.

## Handback

`03-resolution.md` here per the series convention: commits, the
boot.toml/LoadGame semantics you chose, lock-bump instructions for
reference-workload, VM-tier evidence. We re-verify (probe + real-worker
boot) and respond with `04-verification.md`.
