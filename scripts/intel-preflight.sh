#!/usr/bin/env bash
# Intel-box preflight verification (bead guest-sdk-atd) — the Phase 1 entry
# gate from prompts/docs/phase-1-deterministic-execution.md: "Intel box
# preflight passed — pinned kernel, perf_event access, KVM caps". The entire
# in-VM CI tier depends on this passing; it must fail loudly and specifically.
set -uo pipefail

FAIL=0
ok()   { echo "  ok   $*"; }
fail() { echo "  FAIL $*"; FAIL=1; }

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
if [[ -n "$para" && "$para" -le 1 ]]; then
  ok "perf_event_paranoid = ${para} (≤ 1)"
else
  fail "perf_event_paranoid = ${para:-unreadable} (need ≤ 1 for PERF_COUNT_HW_INSTRUCTIONS in a guest)"
fi

echo "[preflight] hugepages (2 MiB channel page for in-VM guests' host side)"
if [[ -d /sys/kernel/mm/hugepages/hugepages-2048kB ]]; then
  ok "2 MiB hugepage support present"
else
  fail "no 2 MiB hugepage support"
fi

echo "[preflight] Rust toolchain"
if command -v cargo >/dev/null; then
  ok "cargo $(cargo --version | cut -d' ' -f2)"
else
  fail "cargo not on PATH"
fi
if rustup target list --installed 2>/dev/null | grep -q x86_64-unknown-linux-musl; then
  ok "x86_64-unknown-linux-musl target installed"
else
  fail "musl target missing — rustup target add x86_64-unknown-linux-musl"
fi

echo "[preflight] pinned kernel artifact"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BZ="${SCRIPT_DIR}/../image/build/bzImage"
if [[ -f "$BZ" ]]; then
  ok "image/build/bzImage present ($(stat -c%s "$BZ") bytes)"
else
  # Not fatal: the harness job builds it via image/build.sh (cached);
  # surface it so a cold runner explains its first slow run.
  echo "  note bzImage not prebuilt — image/build.sh kernel will build it"
fi

if [[ $FAIL -ne 0 ]]; then
  echo "[preflight] FAILED — the in-VM tier cannot run on this machine"
  exit 1
fi
echo "[preflight] all gates passed"
