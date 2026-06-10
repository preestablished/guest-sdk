# Kernel source acquisition and version pinning

Decision record for the `image/` kernel build (bead guest-sdk-2uy). This file
owns: which kernel, where its source comes from, and how build artifacts are
cached. The **kernel cmdline is explicitly NOT configured here** — the
canonical deterministic cmdline is owned by determinism-hypervisor
ARCHITECTURE.md §2.3, which is not in this repo's doc set (tracked as
[issue #1](https://github.com/preestablished/guest-sdk/issues/1)); this repo's
`tests/vm/` harness uses a minimal, explicitly non-canonical cmdline for its
own boots.

## Pinned version

| | |
|---|---|
| Version | **6.12.93** (longterm/LTS) |
| Source | `https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-6.12.93.tar.xz` |
| SHA256 | `492648a87c0b69c5ac7f43be64792b9000e3439550d4e82e4a14710c49094fa3` |

Rationale:

- **LTS line.** 6.12 is a longterm series (projected maintenance into 2027+),
  so security backports keep flowing without us chasing mainline churn; the
  pin is to the exact point release, bumped deliberately.
- **Supports the determinism set.** Every knob the platform requires is
  available and well-aged in 6.12: `COMPACTION=n`, `MIGRATION=n`, `KSM=n`, no
  THP, no swap, `SMP=n` single CPU, hugetlbfs, perf_event
  (`PERF_COUNT_HW_INSTRUCTIONS` retired-instruction counting), devtmpfs,
  procfs (`/proc/<pid>/pagemap` via `PROC_PAGE_MONITOR`), sysfs.
- **Tarball + SHA256 over git clone.** A fixed tarball with a pinned digest is
  reproducible, proxy-cacheable, and avoids a 5 GiB git history in CI. The
  digest above is from `cdn.kernel.org/pub/linux/kernel/v6.x/sha256sums.asc`
  (2026-06-10).

## Build artifact caching

- `image/build/` (gitignored) holds the downloaded tarball, the extracted
  tree, and the outputs (`bzImage`, `initramfs.cpio`).
- `image/build.sh` only re-downloads when the tarball or its digest check is
  missing/stale, and only reconfigures+rebuilds when the kernel version or
  `image/kernel.config` changed (it stamps `build/.kernel-build-key` with
  `sha256(version + config fragment)` and compares).
- **CI cache key**: `kernel-${KERNEL_VERSION}-$(sha256sum image/kernel.config)`
  over `image/build/bzImage` only — the consumers (`tests/vm/` is the join;
  there is exactly **one** kernel build in this repo) never need the source
  tree, just the image.

## Consumers

- `image/build.sh kernel` — produces `image/build/bzImage` from this pin.
- `tests/vm/` (HARNESS_KVM_BASIC and the in-VM tier) boots that `bzImage`
  with its own minimal **non-canonical** cmdline.
- The demo image (`reference-workload`'s `xtask image`) bakes the same kernel
  per IMPLEMENTATION-PLAN M2 ("exactly one kernel build in this repo").
