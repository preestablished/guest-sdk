# Review — iteration 4: image track + test workloads

- **Branch:** `ralph/iteration-4-image-kernel-pinning-config-build` (local: `…-pinning-config-build`)
- **Base:** `main`
- **Date:** 2026-06-10
- **Reviewer:** Claude Opus

## Summary

This iteration adds the repo's single kernel build (`image/KERNEL.md` pinning linux-6.12.93
+ SHA256, `image/kernel.config` determinism fragment over tinyconfig, `image/build.sh`
fetch→verify→configure→assert→build→reproducible-cpio pipeline with a no-sudo docker
toolchain fallback) plus two trivial static-musl test workloads (`autostart-trivial`,
`print-lines`). The work is well-structured, well-documented, and the determinism intent is
clearly traceable to ARCHITECTURE.md §5/§7 and IMPLEMENTATION-PLAN M2. I verified on this
machine that the kernel cache key is byte-correct, the `assert_required_set` absent-symbol
logic genuinely matches the final `.config` (MIGRATION/SWAP/RANDOMIZE_BASE are *absent*, not
"is not set"), and the kernel cmdline is correctly absent everywhere (clean-room boundary
respected — no leakage found). Two issues hold it back from a clean approve: (1) the config
fragment lacks `CONFIG_NET=y`/`CONFIG_UNIX=y`, but ARCHITECTURE.md §4.2 specifies the
agent↔SDK control plane is `socketpair(AF_UNIX, SOCK_SEQPACKET)` — this image cannot create
that socket and the M3 agent will fail at runtime on it; (2) the "byte-reproducible
initramfs" claim is overstated — I reproduced two *different* cpio digests from the same
inputs under `umask 022` vs `umask 077`, because the mountpoint dirs created by `mkdir` carry
umask-dependent mode bits into the newc headers.

## Verdict

**REQUEST_CHANGES**

The two findings above are both fixable with one-line changes and should land before this
image is treated as the canonical pinned build that M2/M3 acceptance depends on. Everything
else is suggestion-grade.

## Stats

| | |
|---|---|
| Commits reviewed | 1 (`4c08568`) |
| Files changed | 9 (+372 / -0) |
| New files | `image/{KERNEL.md,build.sh,kernel.config}`, `tests/vm/workloads/{Cargo.toml,src/bin/*.rs}` |
| Critical findings | 0 |
| Important findings | 3 |
| Suggestions | 6 |
| Build re-verified | `bash -n` OK; `build.sh kernel` cached-skip OK; workloads `cargo build` OK; initramfs reproducibility tested (umask-sensitive) |
