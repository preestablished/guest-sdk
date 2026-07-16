# Review Overview — Image Kernel Pinning / Config / Build

- **Branch:** `ralph/iteration-4-image-kernel-pinning-config-build`
- **Date:** 2026-06-10
- **Reviewer:** Claude Opus (2nd reviewer)
- **Base:** `main` (`git diff main...HEAD`, 9 files, +372)

## Summary

This branch adds the repo's single deterministic-guest kernel build pipeline (`image/build.sh`),
the pinned-version decision record (`image/KERNEL.md`, linux-6.12.93 + SHA256), the minimal
determinism kernel-config fragment (`image/kernel.config`), and two trivial static-musl test
workloads. I verified the build end-to-end on this host: the cached `bzImage` is genuine x86_64
6.12.93, the build-key recomputes byte-for-byte, the determinism set holds in the *final* `.config`
(SMP/COMPACTION/KSM/THP off; MIGRATION/SWAP/RANDOMIZE_BASE absent via unmet deps; no KVM_GUEST /
PARAVIRT / kvmclock — correctly no paravirt time source; CONFIG_64BIT=y overrode tinyconfig's 32-bit
default → X86_64=y), and the merge_config `-m` path is **not** the classic `.config.merged` trap
(it writes back to `.config`, which `olddefconfig` then consumes correctly — the native-toolchain
path is sound). The initramfs assembles with the correct `/init -> /sbin/detguest-agent` symlink and
root:root ownership, and the static-link guard fires on a non-static agent. The kernel always
accepts the uncompressed newc cpio (RD_* compressors are irrelevant; they ended up =y from
olddefconfig defaults but are unused). Workloads build clean; KERNEL.md's "2027+" LTS claim is
accurate (actual EOL Dec 2028).

The work is solid and the deterministic intent is well-executed, but two correctness-of-the-artifact
gaps undercut the stated guarantees: (1) the initramfs is **not byte-reproducible** despite the
header comment and log message both claiming "reproducible/deterministic" — file mtimes leak into
the cpio (proven: two same-content builds differ at byte 53); (2) the docker fallback leaves a
fixed-name `detguest-kbuild-tmp` container on any failed/interrupted first run, wedging every
subsequent build with a name conflict (this exact failure occurred earlier in this session).
Neither is a showstopper for "it boots," but both deserve fixing before this is leaned on in CI or
by other contributors.

## Verdict

**REQUEST_CHANGES** — two Important issues (initramfs non-reproducibility vs. documented claim;
docker temp-container name collision on retry). All other findings are suggestions.

## Stats

| | |
|---|---|
| Files changed | 9 (+372 / −0) |
| Critical | 0 |
| Important | 2 |
| Suggestions | 6 |
| Build verified | bzImage cached no-op; initramfs assembled; workloads compiled; merge_config source audited |
| shellcheck | clean except 1 SC2001 (style) |
