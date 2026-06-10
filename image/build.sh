#!/usr/bin/env bash
# image/build.sh — the repo's ONE kernel build + initramfs assembly
# (beads guest-sdk-zzq / guest-sdk-laj / guest-sdk-2uy; IMPLEMENTATION-PLAN M2).
#
# Usage:
#   image/build.sh kernel                 # build image/build/bzImage
#   image/build.sh initramfs <stage-dir>  # build image/build/initramfs.cpio
#   image/build.sh all <stage-dir>
#
# The kernel pin (version + tarball SHA256) lives in image/KERNEL.md and is
# duplicated here as the single source the script executes; bump both
# together. Rebuilds are skipped when version+config are unchanged
# (build/.kernel-build-key). The kernel CMDLINE is not set here — owned by
# determinism-hypervisor (see image/KERNEL.md).
#
# The initramfs layout is the spec'd minimal image (ARCHITECTURE.md §4):
#   /init -> /sbin/detguest-agent   (symlink; the agent IS the init path,
#                                    no other init binary exists)
#   /sbin/detguest-agent            (static musl binary, from <stage-dir>)
#   /etc/detguest/boot.toml         (from <stage-dir>, API.md §7)
#   plus any workload binaries the stage dir provides, and the empty
#   mountpoint dirs the agent expects (/proc /sys /dev /dev/hugepages /run).
set -euo pipefail

KERNEL_VERSION=6.12.93
KERNEL_URL="https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-${KERNEL_VERSION}.tar.xz"
KERNEL_SHA256=492648a87c0b69c5ac7f43be64792b9000e3439550d4e82e4a14710c49094fa3

# Determinism-critical lines build.sh re-asserts in the FINAL .config
# (olddefconfig silently flipping any of these must fail the build).
REQUIRED_SET=(
  "# CONFIG_SMP is not set"
  "# CONFIG_COMPACTION is not set"
  "# CONFIG_MIGRATION is not set"
  "# CONFIG_KSM is not set"
  "# CONFIG_TRANSPARENT_HUGEPAGE is not set"
  "# CONFIG_SWAP is not set"
  "# CONFIG_RANDOMIZE_BASE is not set"
  "CONFIG_HUGETLBFS=y"
  "CONFIG_PROC_PAGE_MONITOR=y"
  "CONFIG_PERF_EVENTS=y"
  "CONFIG_DEVTMPFS=y"
  "CONFIG_BLK_DEV_INITRD=y"
  "CONFIG_UNIX=y"
  "CONFIG_X86_IOPL_IOPERM=y"
  "CONFIG_DEVMEM=y"
  "CONFIG_HZ_PERIODIC=y"
  "CONFIG_HZ_100=y"
)

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUILD="${SCRIPT_DIR}/build"
SRC="${BUILD}/linux-${KERNEL_VERSION}"
NPROC="$(nproc)"
DOCKER_IMAGE=ubuntu:24.04

log() { echo "[build.sh] $*" >&2; }
die() { echo "[build.sh] ERROR: $*" >&2; exit 1; }

have_native_toolchain() {
  command -v gcc >/dev/null && command -v flex >/dev/null \
    && command -v bison >/dev/null && command -v bc >/dev/null
}

# Run a build command either natively or inside a container that has the
# kernel build deps (this box deliberately has no root; docker is the
# no-sudo path to flex/bison/libelf). Only ${BUILD} is mounted in the
# container — every path a containerized command touches must live there.
run_build() {
  if have_native_toolchain; then
    (cd "$SRC" && "$@")
  else
    local img=detguest-kernel-build:24.04
    if ! docker image inspect "$img" >/dev/null 2>&1; then
      log "creating kernel build image ${img}"
      # A failed/interrupted earlier run leaves the named container behind
      # and would wedge `docker run --name` forever — clear it first.
      docker rm -f detguest-kbuild-tmp >/dev/null 2>&1 || true
      docker run --name detguest-kbuild-tmp "$DOCKER_IMAGE" bash -c \
        'apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
           build-essential flex bison bc libelf-dev libssl-dev cpio xz-utils kmod >/dev/null'
      docker commit detguest-kbuild-tmp "$img" >/dev/null
      docker rm detguest-kbuild-tmp >/dev/null
    fi
    docker run --rm -v "${BUILD}:${BUILD}" -w "$SRC" \
      -e HOME=/tmp -u "$(id -u):$(id -g)" "$img" "$@"
  fi
}

fetch_kernel() {
  mkdir -p "$BUILD"
  local tarball="${BUILD}/linux-${KERNEL_VERSION}.tar.xz"
  if [[ ! -f "$tarball" ]] || ! echo "${KERNEL_SHA256}  ${tarball}" | sha256sum -c --quiet -; then
    log "downloading linux-${KERNEL_VERSION}"
    curl -fL --retry 3 -o "$tarball" "$KERNEL_URL"
    echo "${KERNEL_SHA256}  ${tarball}" | sha256sum -c --quiet - \
      || die "tarball SHA256 mismatch — refusing to build unpinned source"
  fi
  if [[ ! -d "$SRC" ]]; then
    log "extracting"
    tar -C "$BUILD" -xf "$tarball"
  fi
}

build_key() {
  # Covers everything that shapes bzImage: the pin, the fragment, and this
  # script itself (the config pipeline lives here).
  { echo "$KERNEL_VERSION"; cat "${SCRIPT_DIR}/kernel.config" "${BASH_SOURCE[0]}"; } \
    | sha256sum | cut -d' ' -f1
}

assert_required_set() {
  local cfg="$1" missing=0
  for line in "${REQUIRED_SET[@]}"; do
    if [[ "$line" == "# CONFIG_"* ]]; then
      # Disabled is satisfied by an explicit "not set" line OR by the symbol
      # being absent entirely (kconfig omits symbols with unmet deps —
      # e.g. SWAP without BLOCK, RANDOMIZE_BASE without RELOCATABLE).
      local sym="${line#\# }"; sym="${sym% is not set}"
      if ! grep -qxF "$line" "$cfg" && grep -q "^${sym}=" "$cfg"; then
        log "VIOLATION: ${sym} is enabled in the final .config"
        missing=1
      fi
    elif ! grep -qxF "$line" "$cfg"; then
      log "MISSING in final .config: ${line}"
      missing=1
    fi
  done
  [[ $missing -eq 0 ]] || die "determinism config set violated (olddefconfig flipped something?)"
}

cmd_kernel() {
  local key keyfile="${BUILD}/.kernel-build-key"
  key="$(build_key)"
  if [[ -f "${BUILD}/bzImage" && -f "$keyfile" && "$(cat "$keyfile")" == "$key" ]]; then
    log "bzImage up to date (key ${key:0:12}…) — skipping rebuild"
    return 0
  fi
  fetch_kernel
  log "configuring (tinyconfig + fragment + olddefconfig)"
  # The fragment must be under ${BUILD} to be visible inside the container.
  cp "${SCRIPT_DIR}/kernel.config" "${BUILD}/kernel.config.fragment"
  run_build make -s tinyconfig
  run_build scripts/kconfig/merge_config.sh -m .config "${BUILD}/kernel.config.fragment"
  run_build make -s olddefconfig
  assert_required_set "${SRC}/.config"
  log "building bzImage with -j${NPROC} (expect tens of minutes on small hosts)"
  run_build make -s "-j${NPROC}" bzImage
  cp "${SRC}/arch/x86/boot/bzImage" "${BUILD}/bzImage"
  echo "$key" > "$keyfile"
  log "OK: ${BUILD}/bzImage"
}

cmd_initramfs() {
  local stage="${1:?usage: build.sh initramfs <stage-dir>}"
  [[ -x "${stage}/sbin/detguest-agent" ]] \
    || die "stage dir must provide an executable sbin/detguest-agent (static musl build)"
  if command -v file >/dev/null; then
    # Rust musl emits "statically linked" or "static-pie linked".
    file "${stage}/sbin/detguest-agent" | grep -Eq 'static(ally|-pie) linked' \
      || die "detguest-agent must be statically linked (musl)"
  fi
  [[ -f "${stage}/etc/detguest/boot.toml" ]] \
    || die "stage dir must provide etc/detguest/boot.toml (API.md §7)"

  local root="${BUILD}/initramfs-root"
  rm -rf "$root"
  # Byte-reproducibility requires normalizing everything the newc header
  # records beyond what --reproducible covers (it only zeroes dev/ino):
  # umask-derived modes, the stage dir's own mode propagated by cp -a onto
  # rootfs /, and mtimes. The READY-point icount is a pure function of the
  # image, so identical inputs must yield identical bytes.
  (
    umask 022
    mkdir -p "$root"/{proc,sys,dev,dev/hugepages,run,etc/detguest,sbin}
    cp -a "${stage}/." "$root/"
    chmod 0755 "$root"
    # The image's only init path: /init IS the agent (ARCHITECTURE.md §4
    # step 1; no dh-init binary exists anywhere).
    ln -sf /sbin/detguest-agent "${root}/init"
    # Canonical modes: the image is a pure function of (content bytes,
    # executable bit) — stage-dir umask/mode noise must not leak in.
    find "$root" -type d -exec chmod 0755 {} +
    find "$root" -type f -exec sh -c \
      'for f; do if [ -x "$f" ]; then chmod 0755 "$f"; else chmod 0644 "$f"; fi; done' _ {} +
    find "$root" -exec touch -h -d @0 {} +
  )

  log "assembling initramfs.cpio (newc, uncompressed, normalized + sorted)"
  ( cd "$root" && find . -print0 | LC_ALL=C sort -z \
      | cpio --null -o -H newc --reproducible --owner=0:0 2>/dev/null \
  ) > "${BUILD}/initramfs.cpio"
  log "OK: ${BUILD}/initramfs.cpio ($(stat -c%s "${BUILD}/initramfs.cpio") bytes)"
}

case "${1:-}" in
  kernel) cmd_kernel ;;
  initramfs) shift; cmd_initramfs "$@" ;;
  all) shift; cmd_kernel; cmd_initramfs "$@" ;;
  *) die "usage: build.sh kernel | initramfs <stage-dir> | all <stage-dir>" ;;
esac
