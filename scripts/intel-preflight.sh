#!/usr/bin/env bash
# Intel-box preflight verification (bead guest-sdk-atd) — the Phase 1 entry
# gate from prompts/docs/phase-1-deterministic-execution.md: "Intel box
# preflight passed — pinned kernel, perf_event access, KVM caps". The entire
# in-VM CI tier depends on this passing; it must fail loudly and specifically.
set -uo pipefail

FAIL=0
ok()   { echo "  ok   $*"; }
note() { echo "  note $*"; }
fail() { echo "  FAIL $*"; FAIL=1; }

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
BUILD_DIR="${REPO_ROOT}/image/build"

check_kernel_config() {
  local cfg="$1"
  local required=(
    "# CONFIG_SMP is not set"
    "# CONFIG_NUMA is not set"
    "# CONFIG_COMPACTION is not set"
    "# CONFIG_MIGRATION is not set"
    "# CONFIG_KSM is not set"
    "# CONFIG_TRANSPARENT_HUGEPAGE is not set"
    "# CONFIG_SWAP is not set"
    "# CONFIG_RANDOMIZE_BASE is not set"
    "# CONFIG_RANDOMIZE_MEMORY is not set"
    "# CONFIG_RELOCATABLE is not set"
    "# CONFIG_STRICT_DEVMEM is not set"
    "# CONFIG_NO_HZ_IDLE is not set"
    "# CONFIG_NO_HZ_FULL is not set"
    "# CONFIG_HIGH_RES_TIMERS is not set"
    "# CONFIG_HYPERVISOR_GUEST is not set"
    "CONFIG_HUGETLBFS=y"
    "CONFIG_PROC_FS=y"
    "CONFIG_PROC_PAGE_MONITOR=y"
    "CONFIG_SYSFS=y"
    "CONFIG_PERF_EVENTS=y"
    "CONFIG_DEVTMPFS=y"
    "CONFIG_DEVTMPFS_MOUNT=y"
    "CONFIG_BLK_DEV_INITRD=y"
    "CONFIG_BINFMT_ELF=y"
    "CONFIG_SHMEM=y"
    "CONFIG_MULTIUSER=y"
    "CONFIG_NET=y"
    "CONFIG_UNIX=y"
    "CONFIG_X86_IOPL_IOPERM=y"
    "CONFIG_DEVMEM=y"
    "CONFIG_EPOLL=y"
    "CONFIG_SIGNALFD=y"
    "CONFIG_TIMERFD=y"
    "CONFIG_EVENTFD=y"
    "CONFIG_FUTEX=y"
    "CONFIG_HZ_PERIODIC=y"
    "CONFIG_HZ_100=y"
  )
  local missing=0

  for line in "${required[@]}"; do
    if [[ "$line" == "# CONFIG_"* ]]; then
      # Match image/build.sh: a disabled symbol is OK if explicitly not set or
      # absent because its dependencies are disabled. It is a violation only
      # when the final config enables the symbol.
      local sym="${line#\# }"
      sym="${sym% is not set}"
      if ! grep -qxF "$line" "$cfg" && grep -q "^${sym}=" "$cfg"; then
        fail "${cfg#${REPO_ROOT}/}: ${sym} is enabled; rebuild with image/kernel.config pins"
        missing=1
      fi
    elif ! grep -qxF "$line" "$cfg"; then
      fail "${cfg#${REPO_ROOT}/}: missing ${line}; rebuild with image/kernel.config pins"
      missing=1
    fi
  done

  if [[ $missing -eq 0 ]]; then
    ok "${cfg#${REPO_ROOT}/} satisfies required determinism config pins"
  fi
}

check_kernel_provenance() {
  local prov="$1"
  local expected=(
    "kernel_version=6.12.93"
    "kernel_url=https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-6.12.93.tar.xz"
    "kernel_tarball_sha256=492648a87c0b69c5ac7f43be64792b9000e3439550d4e82e4a14710c49094fa3"
  )

  for line in "${expected[@]}"; do
    if grep -qxF "$line" "$prov"; then
      ok "${prov#${REPO_ROOT}/}: ${line}"
    else
      fail "${prov#${REPO_ROOT}/}: missing ${line}; run ./image/build.sh kernel"
    fi
  done
  for key in kernel_config_fragment_sha256 build_script_sha256 final_config_sha256 build_key; do
    if grep -q "^${key}=[0-9a-f]\\{64\\}$" "$prov"; then
      ok "${prov#${REPO_ROOT}/}: ${key} recorded"
    else
      fail "${prov#${REPO_ROOT}/}: missing ${key}; run ./image/build.sh kernel"
    fi
  done
}

echo "[preflight] CPU virtualization"
if grep -qm1 -E 'vmx' /proc/cpuinfo; then
  ok "VT-x (vmx) present"
else
  fail "no vmx flag in /proc/cpuinfo — not an Intel VT-x box"
fi

echo "[preflight] KVM device + API"
if [[ -r /dev/kvm && -w /dev/kvm ]]; then
  ok "/dev/kvm readable+writable by $(id -un)"
else
  fail "/dev/kvm not accessible — add the runner user to the kvm group"
fi
# KVM_GET_API_VERSION (ioctl 0xAE00) must return 12. python3 ships on the
# runner image; this avoids needing a compiled probe for the gate.
if command -v python3 >/dev/null; then
  api=$(python3 - <<'EOF' 2>/dev/null
import fcntl, os
fd = os.open("/dev/kvm", os.O_RDWR)
print(fcntl.ioctl(fd, 0xAE00))
EOF
  )
  if [[ "${api:-}" == "12" ]]; then
    ok "KVM_GET_API_VERSION == 12"
  else
    fail "KVM API version ${api:-unreadable} (want 12)"
  fi
else
  fail "python3 unavailable for the KVM API probe"
fi

echo "[preflight] perf_event access (retired-instruction counter)"
para=$(cat /proc/sys/kernel/perf_event_paranoid 2>/dev/null)
if [[ "${para:-}" =~ ^-?[0-9]+$ && "$para" -le 1 ]]; then
  ok "perf_event_paranoid = ${para} (≤ 1)"
else
  fail "perf_event_paranoid = ${para:-unreadable} (need ≤ 1 for PERF_COUNT_HW_INSTRUCTIONS in a guest)"
fi

# HOST 2 MiB hugepages are NOT needed by anything in tests/vm: the harness
# backs guest RAM with a plain anonymous mmap, and the agent's hugetlbfs
# channel page comes from the GUEST-internal pool (`hugepages=4` on the
# guest cmdline, satisfied inside the VM). The check remains for the
# determinism-hypervisor repo's harness, which does map host hugepages —
# opt in with --require-host-hugepages when preflighting for that use.
if [[ " ${*:-} " == *" --require-host-hugepages "* ]]; then
  echo "[preflight] host hugepages (2 MiB pool for the hypervisor harness)"
  HP_2M="/sys/kernel/mm/hugepages/hugepages-2048kB"
  if [[ -d "$HP_2M" ]]; then
    ok "2 MiB hugepage support present"
    nr=$(cat "${HP_2M}/nr_hugepages" 2>/dev/null)
    free=$(cat "${HP_2M}/free_hugepages" 2>/dev/null)
    if [[ "${nr:-}" =~ ^[0-9]+$ && "${free:-}" =~ ^[0-9]+$ && "$nr" -gt 0 && "$free" -gt 0 ]]; then
      ok "2 MiB hugepage pool has ${free}/${nr} pages free"
    else
      fail "2 MiB hugepage pool empty (${free:-?}/${nr:-?} free/total) — reserve pages with hugepages=N or vm.nr_hugepages"
    fi
  else
    fail "no 2 MiB hugepage support"
  fi
else
  note "host hugepage check skipped (guest-sdk tests need none; pass --require-host-hugepages for the hypervisor harness)"
fi

echo "[preflight] Rust toolchain"
if command -v cargo >/dev/null; then
  ok "cargo $(cargo --version | cut -d' ' -f2)"
else
  fail "cargo not on PATH"
fi
if command -v rustup >/dev/null; then
  if rustup target list --installed 2>/dev/null | grep -q x86_64-unknown-linux-musl; then
    ok "x86_64-unknown-linux-musl target installed"
  else
    fail "musl target missing — rustup target add x86_64-unknown-linux-musl"
  fi
else
  fail "rustup not on PATH — cannot verify x86_64-unknown-linux-musl target"
fi

echo "[preflight] pinned kernel artifact"
BZ="${BUILD_DIR}/bzImage"
FINAL_CONFIG="${BUILD_DIR}/kernel.final.config"
PROVENANCE="${BUILD_DIR}/kernel.provenance"
if [[ -f "$BZ" ]]; then
  ok "image/build/bzImage present ($(stat -c%s "$BZ") bytes)"
  if [[ -f "$FINAL_CONFIG" ]]; then
    check_kernel_config "$FINAL_CONFIG"
  else
    fail "image/build/kernel.final.config missing — run ./image/build.sh kernel to refresh the pinned artifact"
  fi
  if [[ -f "$PROVENANCE" ]]; then
    check_kernel_provenance "$PROVENANCE"
  else
    fail "image/build/kernel.provenance missing — run ./image/build.sh kernel to refresh the pinned artifact"
  fi
else
  fail "image/build/bzImage missing — run ./image/build.sh kernel before the Intel VM lane; cold kernel builds must not hide inside the test timeout"
  if [[ -f "$FINAL_CONFIG" ]]; then
    check_kernel_config "$FINAL_CONFIG"
  else
    note "image/build/kernel.final.config not inspectable until ./image/build.sh kernel has run"
  fi
  if [[ -f "$PROVENANCE" ]]; then
    check_kernel_provenance "$PROVENANCE"
  else
    note "image/build/kernel.provenance not inspectable until ./image/build.sh kernel has run"
  fi
fi

echo "[preflight] pv-pad harness prerequisites"
if grep -q "pub const PVPAD_BASE" "${REPO_ROOT}/tests/vm/src/harness/pio.rs" \
   && grep -q "frame_counter_writes" "${REPO_ROOT}/tests/vm/src/harness/mod.rs"; then
  ok "pv-pad latch and FRAME_COUNTER observation hooks present in tests/vm harness"
else
  fail "pv-pad harness hooks missing — expected PVPAD_BASE and frame_counter_writes in tests/vm/src/harness"
fi

echo "[preflight] replay and snapshot providers"
REPLAY_TOOL="${DETGUEST_REPLAY_TOOL:-determinism_replay}"
if command -v "$REPLAY_TOOL" >/dev/null; then
  ok "replay tool available: ${REPLAY_TOOL}"
elif [[ -n "${DETGUEST_REPLAY_TOOL:-}" ]]; then
  fail "DETGUEST_REPLAY_TOOL=${DETGUEST_REPLAY_TOOL} is not on PATH or not executable"
else
  note "determinism_replay not on PATH — M5 replay gate remains blocked by guest-sdk-ext-hyp-determinism-replay-linux"
fi
note "local harness provides KVM snapshot/fork/restore (tests/vm/src/harness/snapshot.rs — M4 acceptance tier); production snapshots remain determinism-hypervisor's"

if [[ $FAIL -ne 0 ]]; then
  echo "[preflight] FAILED — the in-VM tier cannot run on this machine"
  exit 1
fi
echo "[preflight] all gates passed"
