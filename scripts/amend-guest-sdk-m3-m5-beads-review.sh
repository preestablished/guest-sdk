#!/usr/bin/env bash
# Project: guest-sdk
# Change: reviewer fixes for guest-sdk M3-M5 bead graph
# Generated: 2026-06-18
#
# Run after scripts/create-guest-sdk-m3-m5-beads.sh. This applies the two
# subagent review fixes: real external-blocker beads, connected epics,
# missing detsdk.stats and capture/reference gate tasks, image fixture
# dependency, and clearer implementation priorities.

set -euo pipefail

create() {
  local id="$1"
  local title="$2"
  local priority="$3"
  local labels="$4"
  local type="$5"
  local description="$6"

  if bd show "$id" >/dev/null 2>&1; then
    printf '%s\n' "$id"
  else
    bd create \
      --id "$id" \
      --title "$title" \
      --priority "$priority" \
      --labels "$labels" \
      --type "$type" \
      --description "$description" \
      --silent
  fi
}

dep() {
  local child="$1"
  local parent="$2"

  bd dep add "$child" "$parent" || true
}

remove_dep() {
  local child="$1"
  local parent="$2"

  bd dep remove "$child" "$parent" || true
}

echo "Applying reviewer amendments to guest-sdk M3-M5 bead graph..."

EXT_HYP_M9=$(create "guest-sdk-ext-hyp-m9-linux-guest" "External blocker: determinism-hypervisor M9 Linux guest support shipped" "0" "external,contracts,m3" "task" "Tracks determinism-hypervisor-owned Linux guest M9 support plus Linux reruns of prior gates. Guest-sdk validation beads may close only after the external capability is confirmed shipped or explicitly waived by a human.")
EXT_HYP_INPUT_LOG=$(create "guest-sdk-ext-hyp-input-log-dev-events" "External blocker: hypervisor PAD_SET and channel DEV_EVENT input-log support shipped" "0" "external,contracts,m3,m5" "task" "Tracks determinism-hypervisor-owned PAD_SET landing, channel mutation DEV_EVENT encodings for ring C and I pushes, ring A and W consumer bumps, and pio_answer records. Required for pv-pad input acceptance and replay.")
EXT_HYP_CAPTURE=$(create "guest-sdk-ext-hyp-capture-region-read" "External blocker: hypervisor capture engine reads named guest regions" "0" "external,contracts,m4" "task" "Tracks determinism-hypervisor-owned CaptureSpec and ReadGuestMemory(region) support, feature_bytes, fb_lz4, layout-version failure semantics, framebuffer metadata, and manifest/channel reattach after restore.")
EXT_HYP_REPLAY=$(create "guest-sdk-ext-hyp-determinism-replay-linux" "External blocker: hypervisor determinism_replay Linux guest gate shipped" "0" "external,contracts,m5" "task" "Tracks the determinism-hypervisor-owned bit-identical determinism_replay Linux guest gate and replay-mode input-log application with synthesizer absent.")
EXT_REF_M3=$(create "guest-sdk-ext-refwork-m3-mock-agent" "External blocker: reference-workload M3 mock-agent protocol ready" "0" "external,contracts,m4" "task" "Tracks reference-workload-owned mock-agent protocol readiness needed by guest-sdk unit.control contract tests. This repo validates compatibility but does not implement reference-workload.")
EXT_REF_M4=$(create "guest-sdk-ext-refwork-m4-image-handoff" "External blocker: reference-workload M4 image handoff ready" "0" "external,contracts,m4" "task" "Tracks reference-workload-owned image handoff, region naming, emulator image assumptions, and control-protocol assets needed by guest-sdk M4 integration validation.")
EXT_REF_M5=$(create "guest-sdk-ext-refwork-m5-full-suite" "External blocker: reference-workload M5 full suite ready" "0" "external,contracts,m5" "task" "Tracks reference-workload-owned M5 suite readiness. Guest-sdk records and validates handoff surfaces but does not own that repo's full-suite implementation.")

M4_STATS=$(create "guest-sdk-m4-sdk-stats-region-autoreg" "Auto-register detsdk.stats region with fixed SDK stats layout" "1" "m4,impl,testing" "task" "Implement SDK-owned detsdk.stats auto-registration from init(): fixed SdkStats layout including stats_version, beacon counters, reachability counters, assertion counters, stable size and alignment, allocation that cannot relocate, and host read_region(\"detsdk.stats\") validation. File reservations: crates/detguest-sdk/src/beacons.rs, regions.rs, lib.rs, tests/vm/tests. Acceptance: layout is byte-pinned and region is published automatically after register_region support lands.")
M4_CAPTURE_CONTRACT=$(create "guest-sdk-m4-capture-contract-tests" "Add M4 capture and reference-region contract tests" "1" "m4,contracts,testing" "task" "Add guest-sdk-side contract tests or fixtures for hypervisor CaptureSpec, ReadGuestMemory(region), feature_bytes, fb_lz4, layout-version failure semantics, framebuffer metadata, and reference region names wram, framebuffer, and optional vram. Do not implement hypervisor or reference-workload-owned code. Acceptance: platform-readability validation has explicit contracts for every external capture surface named in the phase prompt.")
M5_REF_20=$(create "guest-sdk-m5-reference-workload-20run-gate" "Track reference-workload M5 20 consecutive zero-flake gate handoff" "0" "m5,contracts,testing,external" "task" "Record the separate reference-workload full-suite success criterion from the phase doc: 20 consecutive zero-flake runs. This is a handoff and compatibility gate, not guest-sdk implementation work. Acceptance: guest-sdk exposes required SDK and host APIs and the external reference-workload M5 suite blocker is closed or explicitly waived.")
M4_KERNEL_SOURCE=$(create "guest-sdk-m4-kernel-source-provenance" "Pin kernel source version checksum and final config provenance" "1" "m4,image,docs" "task" "Extend image/KERNEL.md, image/build.sh, and related docs so kernel source version, tarball checksum, build key inputs, and final .config provenance are explicitly pinned and auditable for M4 memory-publication acceptance. Acceptance: source pin drift or final config drift is visible before Intel VM tests run.")

# Replace inert external:* dependencies with first-class local blocker beads.
remove_dep "guest-sdk-m3-vm-real-workload-e2e" "external:determinism-hypervisor:m9-linux-guest"
remove_dep "guest-sdk-m3-input-path-acceptance" "external:determinism-hypervisor:pad-set-channel-dev-event-input-log"
remove_dep "guest-sdk-m4-unit-control-reference-handoff" "external:reference-workload:m3-mock-agent-protocol"
remove_dep "guest-sdk-m4-unit-control-reference-handoff" "external:reference-workload:m4-image-handoff"
remove_dep "guest-sdk-m4-platform-readability-vm" "external:determinism-hypervisor:capture-engine-read-guest-memory-region"
remove_dep "guest-sdk-m5-host-log-fault-plan" "external:determinism-hypervisor:pad-set-channel-dev-event-input-log"
remove_dep "guest-sdk-m5-determinism-replay-ci-gate" "external:determinism-hypervisor:determinism-replay-linux-guest-gate"
remove_dep "guest-sdk-m5-reference-workload-contract-tests" "external:reference-workload:m5-full-suite"

for ext in \
  "$EXT_HYP_M9" \
  "$EXT_HYP_INPUT_LOG" \
  "$EXT_HYP_CAPTURE" \
  "$EXT_HYP_REPLAY" \
  "$EXT_REF_M3" \
  "$EXT_REF_M4" \
  "$EXT_REF_M5"
do
  dep "$ext" "guest-sdk-m3m5-cross-repo-blockers"
done

dep "guest-sdk-m3-vm-real-workload-e2e" "$EXT_HYP_M9"
dep "guest-sdk-m3-input-path-acceptance" "$EXT_HYP_INPUT_LOG"
dep "guest-sdk-m4-unit-control-reference-handoff" "$EXT_REF_M3"
dep "guest-sdk-m4-unit-control-reference-handoff" "$EXT_REF_M4"
dep "guest-sdk-m4-platform-readability-vm" "$EXT_HYP_CAPTURE"
dep "guest-sdk-m5-host-log-fault-plan" "$EXT_HYP_INPUT_LOG"
dep "guest-sdk-m5-determinism-replay-ci-gate" "$EXT_HYP_REPLAY"
dep "guest-sdk-m5-reference-workload-contract-tests" "$EXT_REF_M5"

# Missing owned tasks and graph connectivity.
dep "$M4_STATS" "guest-sdk-m3-sdk-intern-stats"
dep "$M4_STATS" "guest-sdk-m4-sdk-register-region"
dep "$M4_STATS" "guest-sdk-m4-agent-manifest-writer"
dep "guest-sdk-m4-platform-readability-vm" "$M4_STATS"
dep "guest-sdk-m4-docs-contracts" "$M4_STATS"

dep "$M4_CAPTURE_CONTRACT" "$EXT_HYP_CAPTURE"
dep "$M4_CAPTURE_CONTRACT" "$EXT_REF_M4"
dep "guest-sdk-m4-platform-readability-vm" "$M4_CAPTURE_CONTRACT"
dep "guest-sdk-m4-docs-contracts" "$M4_CAPTURE_CONTRACT"

dep "$M5_REF_20" "guest-sdk-m5-reference-workload-contract-tests"
dep "$M5_REF_20" "$EXT_REF_M5"
dep "guest-sdk-m5-docs-replay" "$M5_REF_20"

dep "$M4_KERNEL_SOURCE" "guest-sdk-m4-kernel-config-pins"
dep "guest-sdk-m4-image-boot-fixtures" "$M4_KERNEL_SOURCE"
dep "guest-sdk-m4-platform-readability-vm" "guest-sdk-m4-image-boot-fixtures"

# Convert tracking epics to ordinary tracking tasks before wiring them to
# terminal task beads. Beads enforces that epic issues can only be blocked by
# other epics; these are planning rollups, not executable implementation epics.
bd update "guest-sdk-m3m5-root" --type task --priority 3 --remove-label epic --add-label tracking
bd update "guest-sdk-m3-real-workload-epic" --type task --priority 3 --remove-label epic --add-label tracking
bd update "guest-sdk-m4-region-publication-epic" --type task --priority 3 --remove-label epic --add-label tracking
bd update "guest-sdk-m5-inject-replay-epic" --type task --priority 3 --remove-label epic --add-label tracking

# Connect tracking rollups so they do not appear as immediately actionable work.
dep "guest-sdk-m3-real-workload-epic" "guest-sdk-m3-docs-as-built"
dep "guest-sdk-m4-region-publication-epic" "guest-sdk-m4-docs-contracts"
dep "guest-sdk-m5-inject-replay-epic" "guest-sdk-m5-docs-replay"
dep "guest-sdk-m3m5-root" "guest-sdk-m3-real-workload-epic"
dep "guest-sdk-m3m5-root" "guest-sdk-m4-region-publication-epic"
dep "guest-sdk-m3m5-root" "guest-sdk-m5-inject-replay-epic"
dep "guest-sdk-m3m5-root" "guest-sdk-m3m5-handoff-closeout"

# Demote ordinary implementation tasks so P0 remains meaningful.
bd update "guest-sdk-m3-sdk-channel-init" --priority 1
bd update "guest-sdk-m3-sdk-ring-w-producer-policy" --priority 1
bd update "guest-sdk-m4-agent-ipc-protocol" --priority 1
bd update "guest-sdk-m4-sdk-register-region" --priority 1
bd update "guest-sdk-m4-agent-ipc-server" --priority 1
bd update "guest-sdk-m4-agent-pagemap-pid-extents" --priority 1
bd update "guest-sdk-m4-agent-manifest-writer" --priority 1
bd update "guest-sdk-m4-ready-gate-expected-regions" --priority 1
bd update "guest-sdk-m4-kernel-config-pins" --priority 1
bd update "guest-sdk-m5-sdk-inject-point" --priority 1
bd update "guest-sdk-m5-host-log-fault-plan" --priority 1
bd update "guest-sdk-m5-host-mutation-log-audit" --priority 1
bd update "guest-sdk-m5-channel-reattach-checkpoint" --priority 1
bd update "guest-sdk-m5-vm-inject-roundtrip" --priority 1

echo ""
echo "Reviewer amendments applied."
echo "Useful checks:"
echo "  bd ready"
echo "  bd blocked"
echo "  bd dep cycles"
