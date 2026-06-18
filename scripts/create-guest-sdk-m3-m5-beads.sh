#!/usr/bin/env bash
# Project: guest-sdk
# Change: guest-sdk in-guest chain milestones 3-5
# Generated: 2026-06-18
#
# This script creates the planning-only Beads task graph requested by
# docs/prompts/guest-sdk-in-guest-chain-milestones-3-5.md. It intentionally
# creates issues and dependencies only; it does not implement product code.

set -euo pipefail

if [ ! -d ".beads" ]; then
  bd init
fi

create() {
  local id="$1"
  local title="$2"
  local priority="$3"
  local labels="$4"
  local type="$5"
  local description="$6"

  bd create \
    --id "$id" \
    --title "$title" \
    --priority "$priority" \
    --labels "$labels" \
    --type "$type" \
    --description "$description" \
    --silent
}

dep() {
  local child="$1"
  local parent="$2"

  bd dep add "$child" "$parent"
}

echo "Creating guest-sdk M3-M5 bead graph..."

EXT_HYP_M9="external:determinism-hypervisor:m9-linux-guest"
EXT_HYP_CAPTURE="external:determinism-hypervisor:capture-engine-read-guest-memory-region"
EXT_HYP_INPUT_LOG="external:determinism-hypervisor:pad-set-channel-dev-event-input-log"
EXT_HYP_REPLAY="external:determinism-hypervisor:determinism-replay-linux-guest-gate"
EXT_REF_M3="external:reference-workload:m3-mock-agent-protocol"
EXT_REF_M4="external:reference-workload:m4-image-handoff"
EXT_REF_M5="external:reference-workload:m5-full-suite"

# ========================================
# Program Epics
# ========================================

ROOT=$(create "guest-sdk-m3m5-root" "Plan and deliver guest-sdk in-guest chain milestones 3-5" "0" "m3-m5,epic" "epic" "Tracks the guest-sdk Phase 3 chain from docs/prompts/guest-sdk-in-guest-chain-milestones-3-5.md. Scope is guest-sdk owned work only: SDK crate, agent integration, host APIs, VM tests, image and CI. Cross-repo work is represented as external blockers.")
M3_EPIC=$(create "guest-sdk-m3-real-workload-epic" "Milestone 3: real workload emits detguest-sdk events end to end" "0" "m3,epic" "epic" "Deliver a workload linked against a new crates/detguest-sdk crate, running under crates/detguest-agent in the Linux guest, with host observation through crates/detguest-host and tests/vm.")
M4_EPIC=$(create "guest-sdk-m4-region-publication-epic" "Milestone 4: pinned memory publication and READY gate" "0" "m4,epic" "epic" "Deliver SDK register_region, agent pagemap translation and manifest writes, platform-readable regions by name, kernel config pinning, unit.control handoff, and snapshot-stable read_region verification.")
M5_EPIC=$(create "guest-sdk-m5-inject-replay-epic" "Milestone 5: inject_point input-log replay determinism" "0" "m5,epic" "epic" "Deliver inject_point round trip, replay-mode fault decisions, host mutation capture audit, sequence checkpointing, and the determinism_replay CI gate.")

# ========================================
# Analysis and Preparation
# ========================================

ANALYZE_DOCS=$(create "guest-sdk-m3m5-analyze-docs" "Analyze local architecture docs and record missing docs/specs docs/adr directories" "0" "analysis,m3-m5" "task" "Inspect docs/prompts/guest-sdk-in-guest-chain-milestones-3-5.md, docs/prompts/phase-1-deterministic-execution-guest-sdk.md, prompts/docs/guest-sdk/README.md, ARCHITECTURE.md, API.md, IMPLEMENTATION-PLAN.md, INTEGRATION.md, prompts/docs/MAP.md, and confirm docs/specs and docs/adr are absent. Acceptance: notes on owned contracts, missing contract docs, and no invented docs/specs or docs/adr paths.")
ANALYZE_CODE=$(create "guest-sdk-m3m5-map-current-surfaces" "Map current wire agent host SDK-gap VM image CI surfaces before implementation" "0" "analysis,m3-m5" "task" "Catalog existing interfaces and stubs in crates/detguest-wire/src/events.rs, record.rs, manifest.rs, ring.rs, crates/detguest-agent/src/boot.rs, channel.rs, commands.rs, runtime.rs, supervise.rs, translate.rs, crates/detguest-host/src/channel.rs, commands.rs, drain.rs, inject.rs, manifest.rs, tests/vm, image/build.sh, image/kernel.config, .github/workflows/ci.yaml, and scripts/intel-preflight.sh. Acceptance: file reservation notes for each work area and explicit risks for READY, ReverifyRegions, unit.control, and LogFaultPlan stubs.")
ANALYZE_TESTS=$(create "guest-sdk-m3m5-characterize-test-patterns" "Characterize existing tests and coverage patterns for M3-M5 expansion" "0" "analysis,testing,m3-m5" "task" "Review crates/detguest-wire/tests/golden_fixtures.rs, proptest_roundtrip.rs, loom_ring.rs, crates/detguest-host/tests/loopback.rs, tests/vm/tests/m2_acceptance.rs, and tests/vm/workloads. Acceptance: identify golden fixture policy, host mutation trace expectations, ignored Intel-only test gating, and places where new M3-M5 tests must be added before behavior changes.")
CONTRACT_BLOCKERS=$(create "guest-sdk-m3m5-cross-repo-blockers" "Record external blockers and compatibility contracts for hypervisor and reference-workload" "0" "analysis,contracts,m3-m5" "task" "Create a compatibility matrix for external dependencies without assigning implementation outside this repo: determinism-hypervisor M9 Linux guest, capture engine ReadGuestMemory(region), PAD_SET and channel DEV_EVENT input-log encodings, determinism_replay Linux gate, reference-workload mock-agent protocol, image handoff, and M5 suite. Acceptance: each external item has an owner, required guest-sdk surface, and validation bead that depends on it.")
CHAR_TESTS=$(create "guest-sdk-m3m5-characterization-tests" "Add characterization tests around current M2 stubs before M3-M5 changes" "0" "prep,testing,m3-m5" "task" "Before implementation, add focused tests that lock current M2 behavior: ReverifyRegions is a no-op, non-empty expected_region faults before READY, unit.control faults before M4, LogFaultPlan proceeds, ChannelWriteSink captures current ring pushes and consumer bumps, and pv-pad frame counter drains after FrameMark. File reservations: crates/detguest-agent/src/commands.rs, runtime.rs, crates/detguest-host/src/inject.rs, tests/vm/src/harness/pio.rs, tests/vm/tests.")

dep "$ANALYZE_CODE" "$ANALYZE_DOCS"
dep "$ANALYZE_TESTS" "$ANALYZE_CODE"
dep "$CONTRACT_BLOCKERS" "$ANALYZE_DOCS"
dep "$CHAR_TESTS" "$ANALYZE_TESTS"

# ========================================
# Milestone 3: SDK Events from a Real Workload
# ========================================

M3_SDK_CRATE=$(create "guest-sdk-m3-sdk-crate" "Scaffold crates/detguest-sdk and workspace wiring" "0" "m3,prep,impl" "task" "Add crates/detguest-sdk to Cargo.toml with std support for in-guest workloads and no dependency on hypervisor repos. Public API must match prompts/docs/guest-sdk/API.md section 1. File reservations: Cargo.toml, Cargo.lock, crates/detguest-sdk/**, tests/vm/workloads/Cargo.toml. Acceptance: cargo check builds the new crate on hosted lanes and standalone no-platform mode is available.")
M3_CHANNEL_INIT=$(create "guest-sdk-m3-sdk-channel-init" "Implement detguest-sdk init channel mapping and detcall privilege setup" "0" "m3,impl" "task" "Implement init() over DETGUEST_CHANNEL_FD: validate ChannelHeader, map the 2 MiB channel, take ring W producer and ring I consumer ownership, set workload_attached if required by the header contract, raise iopl(3), map the pv-pad MMIO window via /dev/mem, and deterministic no-op behavior outside the platform. File reservations: crates/detguest-sdk/src/lib.rs, channel.rs, pio.rs. Acceptance: unit tests cover missing env var, bad header, version mismatch, and idempotent init.")
M3_W_PRODUCER=$(create "guest-sdk-m3-sdk-ring-w-producer-policy" "Implement SDK ring W producer policy with critical retry and droppable counters" "0" "m3,impl" "task" "Use detguest-wire ring Producer for ring W with the critical-vs-droppable policy from prompts/docs/guest-sdk/ARCHITECTURE.md section 3. Droppable Beacon and LogLine bump ringW_dropped counters; critical events doorbell and retry without deadlock. File reservations: crates/detguest-sdk/src/channel.rs, crates/detguest-sdk/src/lib.rs. Acceptance: host-side unit tests force full ring behavior and assert zero critical loss plus exact drop counters by kind.")
M3_INTERN_STATS=$(create "guest-sdk-m3-sdk-intern-stats" "Implement SDK intern table and local stats counters" "1" "m3,impl" "task" "Implement name to name_id interning, NameIntern emission including REACHABLE_DECL, assertion pass and violation counters, reachability hit counters, and beacon counter storage that will later back detsdk.stats registration. File reservations: crates/detguest-sdk/src/intern.rs, beacons.rs, lib.rs. Acceptance: deterministic IDs start at 1, duplicate names do not re-emit, and counters are stable across repeated calls.")
M3_USER_APIS=$(create "guest-sdk-m3-sdk-user-event-apis" "Implement assert_always reachable beacon and log_line SDK APIs" "1" "m3,impl" "task" "Implement assert_always and det_assert_always macro, expect_reachable, declare_reachable, coverage_beacon, and log_line using detguest-wire EventPayload records and the M3 flow-control policy. File reservations: crates/detguest-sdk/src/lib.rs, intern.rs, beacons.rs. Acceptance: exact decoded event sequences cover interns, one AssertViolation with details, first-hit Reachable and Beacon, and LogLine truncation.")
M3_INPUT_FRAME=$(create "guest-sdk-m3-sdk-poll-input-frame-mark" "Implement poll_input and frame_mark over pv-pad MMIO" "1" "m3,impl,input" "task" "Implement poll_input(port) as a read from pv-pad PAD0..PAD3 and frame_mark() as critical FrameMark on ring W followed by FRAME_COUNTER MMIO write. Ring I must remain control-only and carry no pad input. File reservations: crates/detguest-sdk/src/pio.rs, lib.rs. Acceptance: tests assert one latch read per simulated frame and FrameMark producer publication before FRAME_COUNTER write.")
M3_QUIESCE=$(create "guest-sdk-m3-sdk-quiesce-check" "Implement SDK ring I control consumer and quiesce_check" "1" "m3,impl" "task" "Consume ring I WorkloadCtrl records for QuiesceReq and Resume, park deterministically at quiesce_check, emit QuiesceReady, and preserve the invariant that ring I never carries pad input. File reservations: crates/detguest-sdk/src/channel.rs, lib.rs. Acceptance: unit tests cover coop quiesce token matching, resume, unknown control records skipped, and no pad payload type exists.")
M3_AGENT_ATTACH=$(create "guest-sdk-m3-agent-workload-sdk-attach" "Update agent workload spawn path for SDK ownership handoff" "1" "m3,impl" "task" "Verify and extend crates/detguest-agent/src/supervise.rs and channel.rs so workloads inherit DETGUEST_CHANNEL_FD, RLIMIT_MEMLOCK is unlimited, workload attach state is visible in ChannelHeader if required, stdout/stderr LogLine behavior remains unchanged, and WorkloadExited is critical. Acceptance: unit tests or VM tests prove existing M2 autostart and StartWorkload behavior still pass.")
M3_TESTLOAD=$(create "guest-sdk-m3-testload-workload" "Add tests/vm/workloads testload covering all M3 SDK APIs" "1" "m3,impl,testing" "task" "Add a testload binary under tests/vm/workloads that links detguest-sdk and exercises init, assert_always, expect_reachable, declare_reachable, coverage_beacon, log_line, poll_input, frame_mark, quiesce_check, and controlled exit. File reservations: tests/vm/workloads/**, image staging helpers in tests/vm/tests. Acceptance: hosted builds include testload and VM image staging can select it without lab ROMs.")
M3_EVENT_HASH=$(create "guest-sdk-m3-golden-event-stream-hash" "Add exact M3 SDK event-stream hash assertions" "1" "m3,testing" "task" "Extend host or VM tests to compute the expected M3 testload event stream hash: interns, one AssertViolation with details, first-hit Reachable and Beacon, LogLines, poll_input observations, FrameMarks, and WorkloadExited. File reservations: tests/vm/tests, crates/detguest-host tests. Acceptance: the hash changes only when the intended wire event sequence changes.")
M3_OVERFLOW=$(create "guest-sdk-m3-overflow-drop-tests" "Test SDK overflow and critical-event no-loss behavior" "1" "m3,testing" "task" "Add --spam-logs and --spam-asserts testload modes plus host assertions that producer and host drop counters match, droppable events are lost deterministically, critical events are never lost, and doorbell retry does not deadlock. File reservations: crates/detguest-sdk tests, tests/vm/workloads, tests/vm/tests. Acceptance: both unit-level full-ring tests and ignored KVM tests cover the policy.")
M3_VM_E2E=$(create "guest-sdk-m3-vm-real-workload-e2e" "Add Intel KVM M3 real workload end-to-end acceptance test" "0" "m3,testing,ci" "task" "Extend tests/vm/tests with an ignored DETGUEST_VM_TESTS=1 acceptance test that boots Linux, runs testload under detguest-agent, drains SDK events through detguest-host, and observes the expected lifecycle. Depends on hypervisor M9 Linux guest readiness for the canonical lane, but the local harness should cover the repo-owned path. Acceptance: failure dumps serial and decoded events.")
M3_INPUT_ACCEPT=$(create "guest-sdk-m3-input-path-acceptance" "Verify pv-pad scripted input path and frame ordering" "0" "m3,testing,input" "task" "Extend tests/vm/src/harness/pio.rs and tests/vm/tests so scripted PAD_SET at_frame values land in the pv-pad latch, poll_input reads exactly once per simulated frame, ring I contains only control records, and each FrameMark record precedes the matching FRAME_COUNTER write. Acceptance: deterministic frame-to-input trace is asserted over multiple frames and input bursts.")
M3_DOCS=$(create "guest-sdk-m3-docs-as-built" "Update guest-sdk docs for M3 SDK event and input behavior" "2" "m3,docs" "task" "Update prompts/docs/guest-sdk/API.md, ARCHITECTURE.md, INTEGRATION.md, and docs/prompts as-built notes for the SDK crate, ring W event policy, pv-pad input path, testload, and known external blockers. Acceptance: docs cite actual paths and avoid assigning hypervisor or reference-workload implementation to this repo.")

dep "$M3_SDK_CRATE" "$CHAR_TESTS"
dep "$M3_CHANNEL_INIT" "$M3_SDK_CRATE"
dep "$M3_W_PRODUCER" "$M3_CHANNEL_INIT"
dep "$M3_INTERN_STATS" "$M3_CHANNEL_INIT"
dep "$M3_USER_APIS" "$M3_W_PRODUCER"
dep "$M3_USER_APIS" "$M3_INTERN_STATS"
dep "$M3_INPUT_FRAME" "$M3_CHANNEL_INIT"
dep "$M3_QUIESCE" "$M3_CHANNEL_INIT"
dep "$M3_AGENT_ATTACH" "$CHAR_TESTS"
dep "$M3_TESTLOAD" "$M3_USER_APIS"
dep "$M3_TESTLOAD" "$M3_INPUT_FRAME"
dep "$M3_TESTLOAD" "$M3_QUIESCE"
dep "$M3_EVENT_HASH" "$M3_TESTLOAD"
dep "$M3_OVERFLOW" "$M3_W_PRODUCER"
dep "$M3_OVERFLOW" "$M3_TESTLOAD"
dep "$M3_VM_E2E" "$M3_AGENT_ATTACH"
dep "$M3_VM_E2E" "$M3_EVENT_HASH"
dep "$M3_VM_E2E" "$M3_OVERFLOW"
dep "$M3_VM_E2E" "$EXT_HYP_M9"
dep "$M3_INPUT_ACCEPT" "$M3_INPUT_FRAME"
dep "$M3_INPUT_ACCEPT" "$M3_TESTLOAD"
dep "$M3_INPUT_ACCEPT" "$EXT_HYP_INPUT_LOG"
dep "$M3_DOCS" "$M3_VM_E2E"
dep "$M3_DOCS" "$M3_INPUT_ACCEPT"

# ========================================
# Milestone 4: Memory Publication
# ========================================

M4_IPC=$(create "guest-sdk-m4-agent-ipc-protocol" "Define SDK to agent region-registration IPC protocol" "0" "m4,prep,contracts" "task" "Define the local AF_UNIX SOCK_SEQPACKET protocol for /run/detguest/agent.sock covering RegisterRegion, UnregisterRegion, response codes, pid binding, layout_version, flags, and deterministic error mapping. File reservations: crates/detguest-sdk/src/regions.rs, crates/detguest-agent/src region IPC module, prompts/docs/guest-sdk/API.md. Acceptance: protocol tests cover malformed requests and no serde or random hashing is introduced in guest hot paths.")
M4_SDK_REGISTER=$(create "guest-sdk-m4-sdk-register-region" "Implement detguest-sdk register_region mlock prefault and IPC client" "0" "m4,impl" "task" "Implement unsafe register_region, RegionFlags, RegionHandle, unregister-on-drop, mlock2 or mlock fallback, deterministic prefaulting, NameTooLong, NotPinned, ManifestFull, TooManyExtents, and AgentUnavailable mapping. File reservations: crates/detguest-sdk/src/regions.rs, lib.rs. Acceptance: unit tests cover success, unregister, hidden PFN or not-present simulated failures, and no relocation-prone safe wrapper is exposed.")
M4_AGENT_IPC=$(create "guest-sdk-m4-agent-ipc-server" "Add agent IPC socket listener and region request dispatch" "0" "m4,impl" "task" "Add a single-threaded IPC listener under /run/detguest/agent.sock integrated into the PID1 epoll loop. Bind requests to the supervised workload pid, reject unknown pids, and dispatch register and unregister without nondeterministic blocking. File reservations: crates/detguest-agent/src/runtime.rs, supervise.rs, new IPC module. Acceptance: host unit tests cover request lifecycle and existing stdout stderr command polling still passes.")
M4_TRANSLATE_PID=$(create "guest-sdk-m4-agent-pagemap-pid-extents" "Extend pagemap translation to arbitrary workload pid and coalesced extents" "0" "m4,impl" "task" "Extend crates/detguest-agent/src/translate.rs from /proc/self/pagemap to /proc/<pid>/pagemap, walk a GVA range page by page, detect present, swapped, PFN-hidden, and non-contiguous pages, and coalesce adjacent GPAs into manifest extents. Acceptance: unit tests cover coalescing, partial first and last pages, hidden PFN, not-present, swapped, overflow, and deterministic error strings.")
M4_MANIFEST_WRITE=$(create "guest-sdk-m4-agent-manifest-writer" "Implement seqlock manifest writer for register and unregister" "0" "m4,impl" "task" "Create the agent-owned manifest writer over the channel manifest area using detguest-wire manifest helpers and required fences: generation odd, mutate, generation even. Enforce 64 region slots, 1024 extent slots, 56-byte names, dead entries on unregister, and stable region ids. File reservations: crates/detguest-agent/src/channel.rs, new manifest writer module. Acceptance: tests assert byte layout, capacity failures, dead entries, generation monotonicity, and no torn host read under forced retry.")
M4_REGION_EVENTS=$(create "guest-sdk-m4-region-events-reverify" "Emit RegionRegister RegionUpdate and implement ReverifyRegions" "1" "m4,impl" "task" "After manifest updates, emit ring A RegionRegister or RegionUpdate with name_id, layout_version, and manifest_generation. Implement ReverifyRegions by rewalking live regions, reporting moved extents or dead entries, and emitting RegionUpdate. File reservations: crates/detguest-agent/src/commands.rs, channel.rs, manifest writer module. Acceptance: unit tests replace the current ReverifyRegions no-op and assert generation/event ordering.")
M4_READY_GATE=$(create "guest-sdk-m4-ready-gate-expected-regions" "Implement READY gate for expected_region liveness and layout versions" "0" "m4,impl" "task" "Replace the current expected_regions boot fault in crates/detguest-agent/src/runtime.rs with a deterministic wait for live manifest entries matching boot.toml expected_region names and layout_version. Fail loud before Ready on missing, duplicate, or mismatched regions. Acceptance: VM tests cover success, missing region, and layout mismatch; Ready includes region_count and even manifest_generation.")
M4_UNIT_CONTROL=$(create "guest-sdk-m4-unit-control-reference-handoff" "Implement unit.control Hello LoadGame Start handoff before READY" "1" "m4,impl,contracts" "task" "Implement the guest-sdk-owned side of boot.toml [unit.control] for the reference-workload protocol over the configured fd: drive Hello, LoadGame, and Start; withhold Ready until Start succeeds and expected regions match; fail loud before Ready on protocol errors. File reservations: crates/detguest-agent/src/boot.rs, runtime.rs, supervise.rs, new control module. Acceptance: synthetic protocol tests run without proprietary ROMs.")
M4_HOST_REGION_TESTS=$(create "guest-sdk-m4-host-read-region-restore-tests" "Broaden host read_manifest read_region and restore coverage" "1" "m4,testing" "task" "Extend crates/detguest-host manifest tests and tests/vm harness hooks to cover reading named regions across discontiguous extents, dead entry rejection, layout_version checks at caller boundaries, channel base GPA reattach after restore, and no guest round trip. File reservations: crates/detguest-host/src/manifest.rs, tests/vm/src/harness. Acceptance: MockGuestMem and KVM memslot-backed tests cover the same public API.")
M4_PLATFORM_READ=$(create "guest-sdk-m4-platform-readability-vm" "Verify published regions are readable across 100 snapshot restore branches" "0" "m4,testing,ci" "task" "Add an Intel-only VM acceptance test that publishes RAM and framebuffer test regions, takes a root snapshot, forks 100 children, runs each for 60 guest frames with different inputs, and verifies detguest-host read_region works from each child without a guest round trip. File reservations: tests/vm/src/harness, tests/vm/tests, tests/vm/workloads. Acceptance: no lab ROMs or proprietary images are required.")
M4_CHURN=$(create "guest-sdk-m4-reverify-churn-test" "Add 10-minute churn workload and ReverifyRegions zero-move acceptance" "1" "m4,testing" "task" "Add a synthetic workload that churns writes inside registered fixed mappings for 10 minutes on the Intel lane, then sends ReverifyRegions and asserts zero moved extents and expected RegionUpdate behavior. File reservations: tests/vm/workloads, tests/vm/tests. Acceptance: test is ignored and env-gated outside the Intel runner.")
M4_KERNEL=$(create "guest-sdk-m4-kernel-config-pins" "Pin final kernel config for no memory movement and required capabilities" "0" "m4,impl,image" "task" "Extend image/kernel.config and image/build.sh REQUIRED_SET assertions for no SMP, NUMA, COMPACTION, MIGRATION, KSM, THP, SWAP, RANDOMIZE_BASE, STRICT_DEVMEM disabled, plus HUGETLBFS, PROC_PAGE_MONITOR, DEVMEM, X86_IOPL_IOPERM, UNIX, deterministic timer config, CAP_SYS_RAWIO viability, and unlimited RLIMIT_MEMLOCK path. Acceptance: final .config drift fails loudly and docs mention hypervisor-owned hugepages=N cmdline.")
M4_IMAGE_FIXTURES=$(create "guest-sdk-m4-image-boot-fixtures" "Add boot.toml and initramfs fixtures for expected_region and unit.control" "1" "m4,impl,image" "task" "Add synthetic boot.toml fixtures and staging helpers for expected_region success and failure cases plus [unit.control] without committing operator ROMs or proprietary images. File reservations: image/boot*.toml, tests/vm test staging code, image/build.sh if needed. Acceptance: fixtures are deterministic and usable by the Intel VM tests.")
M4_DOCS=$(create "guest-sdk-m4-docs-contracts" "Document M4 memory publication contracts and cross-repo assumptions" "2" "m4,docs,contracts" "task" "Update prompts/docs/guest-sdk/API.md, ARCHITECTURE.md, INTEGRATION.md, image/KERNEL.md, docs/ci/intel-runner.md, and docs/prompts with as-built register_region, manifest, READY, kernel pin, hypervisor cmdline, ReadGuestMemory(region), and reference-workload handoff details. Acceptance: missing hypervisor or reference-workload contracts become beads or external blockers, not invented implementation text.")

dep "$M4_IPC" "$M3_VM_E2E"
dep "$M4_IPC" "$M3_INPUT_ACCEPT"
dep "$M4_SDK_REGISTER" "$M4_IPC"
dep "$M4_AGENT_IPC" "$M4_IPC"
dep "$M4_TRANSLATE_PID" "$M4_AGENT_IPC"
dep "$M4_MANIFEST_WRITE" "$M4_TRANSLATE_PID"
dep "$M4_REGION_EVENTS" "$M4_MANIFEST_WRITE"
dep "$M4_READY_GATE" "$M4_REGION_EVENTS"
dep "$M4_UNIT_CONTROL" "$M4_READY_GATE"
dep "$M4_UNIT_CONTROL" "$EXT_REF_M3"
dep "$M4_UNIT_CONTROL" "$EXT_REF_M4"
dep "$M4_HOST_REGION_TESTS" "$M4_MANIFEST_WRITE"
dep "$M4_PLATFORM_READ" "$M4_SDK_REGISTER"
dep "$M4_PLATFORM_READ" "$M4_UNIT_CONTROL"
dep "$M4_PLATFORM_READ" "$M4_HOST_REGION_TESTS"
dep "$M4_PLATFORM_READ" "$EXT_HYP_CAPTURE"
dep "$M4_CHURN" "$M4_REGION_EVENTS"
dep "$M4_KERNEL" "$ANALYZE_CODE"
dep "$M4_IMAGE_FIXTURES" "$M4_KERNEL"
dep "$M4_IMAGE_FIXTURES" "$M4_READY_GATE"
dep "$M4_DOCS" "$M4_PLATFORM_READ"
dep "$M4_DOCS" "$M4_CHURN"

# ========================================
# Milestone 5: Inject Point and Replay
# ========================================

M5_SDK_INJECT=$(create "guest-sdk-m5-sdk-inject-point" "Implement SDK inject_point query and detcall round trip" "0" "m5,impl" "task" "Implement detguest_sdk::inject_point: allocate iseq, intern the point name, emit critical InjectQuery on ring W, OUT then IN on PORT_INJECT, decode FaultDecision, and return Proceed in standalone or error cases. File reservations: crates/detguest-sdk/src/inject.rs, pio.rs, lib.rs. Acceptance: unit tests assert ordering of ring W publication before detcall and packed decision decoding.")
M5_LOG_PLAN=$(create "guest-sdk-m5-host-log-fault-plan" "Replace LogFaultPlan skeleton with replay-decision adapter" "0" "m5,impl" "task" "Replace the current Proceed-only LogFaultPlan skeleton in crates/detguest-host/src/inject.rs with a guest-sdk-owned adapter over supplied replay decisions while leaving DHILOG serialization to determinism-hypervisor. Acceptance: tests prove same iseq and name_id produce logged decisions with the synthesizer absent, unmatched entries fail loudly or proceed only where API says so.")
M5_MUTATION_AUDIT=$(create "guest-sdk-m5-host-mutation-log-audit" "Audit every host channel mutation through ChannelWriteSink" "0" "m5,testing" "task" "Add exhaustive tests that every host mutation is reported exactly once: ring C and I pushes, ring A and W consumer index bumps, and pio_answer for inject. Include wrap pads in ring_push spans and failed pushes not logging. File reservations: crates/detguest-host/src/commands.rs, drain.rs, inject.rs, tests. Acceptance: a single ordered trace can replay all host-owned channel mutations.")
M5_REATTACH=$(create "guest-sdk-m5-channel-reattach-checkpoint" "Verify channel reattach and producer consumer sequence checkpointing after restore" "0" "m5,impl,testing" "task" "Extend detguest-host and VM harness support for channel base GPA reattach after snapshot restore, ring C/I producer sequence checkpoint and restore, ring A/W consumer index replay, intern table and pending inject state reconstruction or checkpointing. File reservations: crates/detguest-host/src/channel.rs, drain.rs, tests/vm/src/harness. Acceptance: restored branches continue sequence numbers without duplicate records.")
M5_VM_ROUNDTRIP=$(create "guest-sdk-m5-vm-inject-roundtrip" "Add VM inject_point round-trip tests with varied fault decisions" "0" "m5,testing" "task" "Extend testload and tests/vm so inject_point calls round trip through the PIO handler, host drains matching InjectQuery inside the exit, TableFaultPlan returns varied Platform and Workload decisions, pio_answer is logged, and replay-mode LogFaultPlan returns the same decisions. Acceptance: decisions are observed at the same inject sequence points over repeated runs.")
M5_REPLAY_GATE=$(create "guest-sdk-m5-determinism-replay-ci-gate" "Add bit-identical determinism_replay Intel CI gate for guest-sdk" "0" "m5,testing,ci" "task" "Add or wire the Intel self-hosted CI gate for 1000 seeded guest-sdk iterations with varied fault plans and input bursts. The gate must prove bit-identical determinism_replay with synthesizer absent, including ring C/I pushes, ring A/W consumer bumps, pio answers, and SDK event/drop counter equivalence. Acceptance: hosted CI remains host-only and the in-VM lane is push-only.")
M5_REFERENCE_COMPAT=$(create "guest-sdk-m5-reference-workload-contract-tests" "Record reference-workload M5 compatibility tests without owning that repo" "1" "m5,contracts,testing" "task" "Create guest-sdk-side contract tests or documentation beads for reference-workload region names, control protocol, input mapping, inject decisions, and full-suite handoff. Do not commit ROMs or implement reference-workload-owned work. Acceptance: guest-sdk exposes the needed SDK and host APIs and has external blockers for the reference full suite.")
M5_DOCS=$(create "guest-sdk-m5-docs-replay" "Document M5 inject_point replay and input-log invariants" "2" "m5,docs" "task" "Update prompts/docs/guest-sdk/API.md, ARCHITECTURE.md, INTEGRATION.md, docs/ci/intel-runner.md, and docs/prompts with the final inject_point flow, fault plan ownership, input-log mutation list, replay requirements, and CI gate. Acceptance: docs state every host mutation that must be logged and cite detguest-host API names.")

dep "$M5_SDK_INJECT" "$M4_PLATFORM_READ"
dep "$M5_LOG_PLAN" "$M4_HOST_REGION_TESTS"
dep "$M5_LOG_PLAN" "$EXT_HYP_INPUT_LOG"
dep "$M5_MUTATION_AUDIT" "$M4_HOST_REGION_TESTS"
dep "$M5_REATTACH" "$M5_MUTATION_AUDIT"
dep "$M5_VM_ROUNDTRIP" "$M5_SDK_INJECT"
dep "$M5_VM_ROUNDTRIP" "$M5_LOG_PLAN"
dep "$M5_VM_ROUNDTRIP" "$M5_REATTACH"
dep "$M5_REPLAY_GATE" "$M5_VM_ROUNDTRIP"
dep "$M5_REPLAY_GATE" "$EXT_HYP_REPLAY"
dep "$M5_REFERENCE_COMPAT" "$M5_VM_ROUNDTRIP"
dep "$M5_REFERENCE_COMPAT" "$EXT_REF_M5"
dep "$M5_DOCS" "$M5_REPLAY_GATE"
dep "$M5_DOCS" "$M5_REFERENCE_COMPAT"

# ========================================
# CI, Quality, and Handoff
# ========================================

CI_HOSTED=$(create "guest-sdk-m3m5-ci-hosted-lanes-stay-host-only" "Keep hosted CI host-only while adding SDK crate coverage" "1" "ci,testing,m3-m5" "task" "Update .github/workflows/ci.yaml so hosted lanes run fmt, clippy, workspace tests, no_std, miri, loom, musl, and aarch64 as appropriate for the new SDK crate without running KVM, real workload, snapshot, or replay gates. Acceptance: fork PRs cannot execute self-hosted jobs.")
CI_INTEL=$(create "guest-sdk-m3m5-ci-intel-vm-lanes" "Update Intel self-hosted lane for M3 M4 M5 gates" "0" "ci,testing,m3-m5" "task" "Extend the push-only Intel self-hosted lane with real workload, memory publication, pv-pad input, inject_point, and determinism_replay gates with exact repeat counts. File reservations: .github/workflows/ci.yaml, scripts/intel-preflight.sh, docs/ci/intel-runner.md. Acceptance: lane remains gated to self-hosted intel kvm and public fork PRs never run on it.")
CI_PREFLIGHT=$(create "guest-sdk-m3m5-intel-preflight-updates" "Extend Intel preflight for M3-M5 kernel and replay prerequisites" "1" "ci,testing,image" "task" "Update scripts/intel-preflight.sh to verify KVM, perf, musl target, 2 MiB hugepages, kernel artifact, required config pins where locally inspectable, replay tool availability, and any pv-pad or snapshot prerequisites exposed by the harness. Acceptance: failures explain the missing host capability.")
QUALITY=$(create "guest-sdk-m3m5-final-quality-gates" "Run final quality gates for guest-sdk M3-M5" "0" "testing,ci,m3-m5" "task" "Run cargo fmt --all --check, cargo clippy --workspace --all-targets -- -D warnings, cargo test --workspace, cargo test -p detguest-wire --no-default-features, miri ring tests, loom ring tests, musl release build, and Intel VM gates. Acceptance: all code changed by the milestone passes or has a filed blocker with exact command output.")
HANDOFF=$(create "guest-sdk-m3m5-handoff-closeout" "Close M3-M5 beads with pushed repo and beads state" "1" "cleanup,m3-m5" "task" "After successful gates, close completed beads, file follow-ups for residual cross-repo gaps, run bd preflight, git status, git add, git commit, git pull --rebase, bd dolt push, git push, and verify git status is up to date with origin. Acceptance: no local bead or code changes remain stranded.")

dep "$CI_HOSTED" "$M3_SDK_CRATE"
dep "$CI_INTEL" "$M3_VM_E2E"
dep "$CI_INTEL" "$M4_PLATFORM_READ"
dep "$CI_INTEL" "$M5_REPLAY_GATE"
dep "$CI_PREFLIGHT" "$M4_KERNEL"
dep "$QUALITY" "$CI_HOSTED"
dep "$QUALITY" "$CI_INTEL"
dep "$QUALITY" "$CI_PREFLIGHT"
dep "$QUALITY" "$M3_DOCS"
dep "$QUALITY" "$M4_DOCS"
dep "$QUALITY" "$M5_DOCS"
dep "$HANDOFF" "$QUALITY"

echo ""
echo "Created guest-sdk M3-M5 bead graph."
echo "Useful checks:"
echo "  bd ready"
echo "  bd blocked"
echo "  bd stats"
