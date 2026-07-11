# Big Change Planning with Beads

## Agent Instructions

You are an expert software architect creating a comprehensive task breakdown for a change to an existing codebase. This task graph will be executed by AI agents working in parallel, coordinated through MCP Agent Mail with file reservations to prevent conflicts.

<quality_expectations>
Create a thorough, production-ready task graph. Include all necessary analysis, preparation, implementation, testing, and documentation tasks. Go beyond the basics - consider edge cases, error handling, security considerations, backwards compatibility, and integration points. Each task should be specific enough for an agent to execute independently without ambiguity.
</quality_expectations>

<critical_constraint>
You must NOT implement any of the changes yourself. Your ONLY output is a bash shell script containing `bd create` and `bd dep add` commands. Do NOT use `bd add` - the correct command is `bd create`. Do not write code. Do not create files other than the shell script. Do not modify existing files. Read and analyze the codebase, then produce the script.
</critical_constraint>

## Change Information

### Change Type
NEW_FEATURE

### Description
Plan the `guest-sdk` in-guest Phase 3 chain from `~/.agents/projects/determinism/phases/phase-3-workload-in-the-box.md`:

1. Milestone 3 - `detguest-sdk` end-to-end events from a real workload. Depends on hypervisor M9 (Linux guest).
2. Milestone 4 - memory publication usable by the platform: `mlock` plus pagemap GVA-to-GPA translation, seqlock manifest updates, and kernel-config pinning for no compaction, migration, KSM, THP, or swap. Depends on Milestone 3.
3. Milestone 5 - `inject_point` plus input-log round trip plus determinism proof through the bit-identical `determinism_replay` CI gate. Depends on Milestone 4.

This extends the existing M0-M2 guest execution stack into the first real workload path: the workload runs in the Linux guest under the agent, links a new `detguest-sdk` crate, emits SDK events from the workload process, publishes RAM/framebuffer regions that the platform can read by name, accepts scripted controller input through the pv-pad latch via `poll_input()`/`frame_mark()`, and proves fault-injection replay equivalence through `inject_point`.

### Links to Relevant Documentation
- `~/.agents/projects/determinism/`
- `~/.agents/projects/determinism/phases/phase-3-workload-in-the-box.md`
- Local planning context: `docs/prompts/phase-1-deterministic-execution-guest-sdk.md`
- Existing repo context: `prompts/docs/guest-sdk/README.md`, `prompts/docs/guest-sdk/ARCHITECTURE.md`, `prompts/docs/guest-sdk/API.md`, `prompts/docs/guest-sdk/IMPLEMENTATION-PLAN.md`, `prompts/docs/guest-sdk/INTEGRATION.md`

### Affected Areas
- `crates/detguest-wire/`: event, command, workload-control, manifest, ring, and port definitions. Existing relevant surfaces include `EventPayload::{RegionRegister, RegionUpdate, InjectQuery, FrameMark, Ready}`, manifest seqlock helpers, record encoding/decoding, golden fixtures, proptest, loom, and fuzz coverage.
- `crates/detguest-sdk/` and `Cargo.toml`: new SDK crate and workspace wiring. Plan beads for `init()` channel-fd inheritance, `iopl(3)`, pv-pad MMIO mapping, stats-region auto-registration, intern table, ring-W producer ownership, ring-I control consumer, critical-vs-droppable event policy, doorbell retry, `assert_always`, `expect_reachable`/`declare_reachable`, `coverage_beacon`, `log_line`, `poll_input`, `frame_mark`, `inject_point`, `quiesce_check`, and `register_region`.
- `crates/detguest-agent/`: PID 1 runtime, boot manifest handling, command dispatch, workload supervision, pagemap translation, agent IPC socket (`SOCK_SEQPACKET`, for example `/run/detguest/agent.sock`), `RegisterRegion`/unregister request handling, extent coalescing, manifest seqlock writer path, `RegionRegister`/`RegionUpdate` emission, `ReverifyRegions`, `[unit.control]` handling, and READY gating on expected regions.
- `crates/detguest-host/`: `Channel`, `GuestMem`, `ChannelWriteSink`, `read_manifest`, `read_region`, `InjectResponder`, `FaultPlan`, real `LogFaultPlan` replay integration point, intern-table tracking, pending-inject state, channel base GPA reattach after restore, producer/consumer sequence checkpointing, and all host mutation recording: ring C/I pushes, ring A/W consumer bumps, and `pio_answer`.
- `tests/vm/`: KVM harness, memslot-backed `GuestMem`, detcall PIO handling, pv-pad MMIO latch, retired-instruction counter, real workload boot/run fixtures, snapshot/restore continuity hooks, producer-sequence restore tests, and ignored Intel-only acceptance tests.
- `tests/vm/workloads/`: existing trivial workloads plus `testload` exercising every SDK API, overflow/drop behavior, controller input via pv-pad, `frame_mark` ordering, `inject_point`, and integration handoff with `reference-workload` for the emulator image and first-room scripted input.
- `image/`: initramfs assembly, `boot.toml` fixtures with `expected_region` and `[unit.control]`, `image/kernel.config`, kernel source pinning/build flow, and final `.config` assertions. Required pins include no `SMP`, `NUMA`, `COMPACTION`, `MIGRATION`, `KSM`, `TRANSPARENT_HUGEPAGE`, `SWAP`, or `RANDOMIZE_BASE`; required enabled capabilities include `HUGETLBFS`, `PROC_PAGE_MONITOR`, `DEVMEM` with unrestricted `/dev/mem`, `X86_IOPL_IOPERM`, `UNIX`, deterministic timer configuration, pagemap PFN visibility for the agent, `CAP_SYS_RAWIO`, and unlimited `RLIMIT_MEMLOCK`.
- `.github/workflows/ci.yaml` and `scripts/intel-preflight.sh`: hosted lanes remain host-only; Intel self-hosted in-VM lane gains real workload, memory-publication, pv-pad input, `inject_point`, and `determinism_replay` gates with exact repeat counts.
- `docs/ci/intel-runner.md`, `docs/prompts/`, and any as-built docs needed to record cross-repo assumptions with `determinism-hypervisor` M9, the hypervisor capture engine, and `reference-workload` M3/M4/M5.
- No `docs/specs/` or `docs/adr/` directory exists in this repo at planning time; the task graph should explicitly inspect the local `prompts/docs/guest-sdk/` docs and file follow-up documentation beads rather than inventing missing cross-repo contracts.
- Cross-repo handoff surfaces: `determinism-hypervisor` owns Linux guest M9, the canonical kernel cmdline including `hugepages=N`, input-log format, `CaptureSpec`, `ReadGuestMemory(region)`, `feature_bytes`, `fb_lz4`, layout-version failure semantics, framebuffer metadata, and manifest/channel reattach after restore. `reference-workload` owns region names (`wram`, `framebuffer`, optional `vram`), the control protocol (`Hello -> LoadGame -> Start`), the emulator image, and the full M5 suite. The guest-sdk task graph should create compatibility and contract-test beads, not assign implementation work in those repos.

### Success Criteria
The change is complete when the relevant `guest-sdk` work in `~/.agents/projects/determinism/phases/phase-3-workload-in-the-box.md` is implemented and verified:

- Milestone 3: a real workload running inside the Linux guest emits end-to-end `detguest-sdk` events through the agent and host channel, with the host observing the expected workload lifecycle and SDK records from the KVM harness.
- Milestone 3 SDK API: `testload` links `crates/detguest-sdk` and produces the exact expected golden event-stream hash: interns, one `AssertViolation` with details, first-hit `Reachable` and `Beacon`, `LogLine`s, `poll_input` reads, `FrameMark`s, and `WorkloadExited`. Overflow tests cover `--spam-logs` and `--spam-asserts`: producer and host drop counters match, zero critical events are lost, and critical doorbell-retry does not deadlock.
- Milestone 3 input path: scripted controller input is `InjectInputs(PAD_SET @ at_frame)` into the hypervisor-owned pv-pad latch, read by the SDK only through `poll_input()`. Ring I is control-only. Tests assert one latch read per simulated frame and one `FrameMark` record before the matching `FRAME_COUNTER` MMIO write.
- Milestone 4 region publication: SDK `register_region` performs `mlock2`/prefault, handles PFN-hidden/non-present/swapped failures, and sends a register request over the agent IPC socket. The agent walks `/proc/<pid>/pagemap`, coalesces extents, validates manifest capacity and names, writes the seqlock manifest with the required fences, emits `RegionRegister`, supports unregister/dead entries, and services `ReverifyRegions` with `RegionUpdate`.
- Milestone 4 READY gate: with reference-workload control enabled, the agent drives `Hello -> LoadGame -> Start`, withholds ring-A `Ready` until `Start` succeeds and all `boot.toml` `expected_region` entries match their pinned `layout_version`, and fails loud before READY on protocol or region errors.
- Milestone 4 platform readability: emulator RAM and framebuffer regions are published by name through the manifest, backed by pinned pages, translated from GVA to GPA using pagemap, readable by the host/platform through `detguest-host`, and stable across 100 snapshot/restore cycles. Fork 100 children from a snapshot, run each for 60 guest frames with different inputs, and verify each child's `read_region` works without a guest round trip. A 10-minute churn workload followed by `ReverifyRegions` reports zero moved extents.
- Milestone 4 kernel acceptance: kernel config and image build assertions pin the no-memory-movement and capability requirements listed above, fail loudly if the final `.config` drifts, and document the hypervisor-owned cmdline handoff for `hugepages=N`.
- Milestone 5 replay: `inject_point` queries round trip through the host PIO path and replay path, the input log captures every deterministic host mutation (`ChannelWriteSink` ring C/I pushes, ring A/W consumer bumps, and `pio_answer`), C/I producer sequences and needed host state are checkpointed/restored, and replay returns the same decisions at the same inject sequence points with the synthesizer absent.
- CI: the bit-identical `determinism_replay` gate passes on the Intel in-VM lane for 1000 seeded guest-sdk iterations with varied fault plans and input bursts. The reference-workload full suite remains a separate 20 consecutive zero-flake gate from the phase doc.
- Cross-repo readiness: the guest-sdk graph records and respects external blockers for `determinism-hypervisor` M9 Linux guest support plus Linux re-runs of prior gates, hypervisor capture-engine integration, `reference-workload` M3 mock-agent protocol, `reference-workload` M4 image handoff, and `reference-workload` M5 suite, without implementing or assuming those repos' owned work.

### Constraints
- Do not commit operator-supplied game ROMs, proprietary images, or lab-only goldens. Repo CI must use synthetic fixtures and `testload`; real game images are supplied by the operator in the lab.
- Respect the clean-room source boundary from `MAP.md`: use project docs and public references only. File documentation beads for missing cross-repo contracts rather than filling gaps from external deterministic-platform source material.
- Keep hosted CI host-only. In-VM KVM, real workload, snapshot/restore, and replay gates run only on the Intel self-hosted lane or explicit human-gated verification beads.

---

## Your Task

Analyze this codebase change and create a comprehensive **Beads task graph** using the `bd` CLI. Beads provides dependency-aware, conflict-free task management for multi-agent execution.

Before creating the task graph, you MUST first analyze the affected areas of the codebase:

1. Check `docs/specs/` and `docs/adr/` for existing architectural decisions
2. Examine the directory/module structure of the affected areas listed above
3. Identify key interfaces, APIs, and integration points that must be preserved
4. Note existing test patterns and coverage in the affected areas
5. Assess risk areas where changes could break existing functionality

Use your analysis to make each bead specific - reference actual file paths, module names, and patterns you observed.

Then generate a shell script that creates the complete task graph.

**IMPORTANT: Your ONLY deliverable is a bash shell script with `bd create` commands. Not an implementation plan. Not a design document. Not a code review. A runnable `.sh` script.**

---

## Output Format

Generate a shell script that creates the full task graph. The script should:

1. **Initialize Beads** (if not already initialized)
2. **Create all beads** with appropriate priorities
3. **Establish dependencies** between beads
4. **Add labels** for phase grouping

### Example Output

```bash
#!/bin/bash
# Project: guest-sdk
# Change: Refactor auth middleware for compliance
# Generated: 2026-06-18

set -e

# Initialize beads if needed
if [ ! -d ".beads" ]; then
    bd init
fi

echo "Creating change beads..."

# ========================================
# Phase 1: Analysis & Preparation
# ========================================

ANALYZE_CURRENT=$(bd create "Analyze current auth middleware implementation in src/auth/ - document all session token storage patterns and consumer dependencies" -p 0 --label analysis --silent)

IDENTIFY_DEPS=$(bd create "Map all modules importing from src/auth/ and catalog their usage patterns" -p 0 --label analysis --silent)

CHAR_TESTS=$(bd create "Add characterization tests capturing current auth middleware behavior before refactoring" -p 0 --label prep --silent)
bd dep add $CHAR_TESTS $ANALYZE_CURRENT

# ========================================
# Phase 2: Core Implementation
# ========================================

IMPL_NEW_STORAGE=$(bd create "Implement compliant session token storage in src/auth/session.ts replacing in-memory store" -p 0 --label impl --silent)
bd dep add $IMPL_NEW_STORAGE $CHAR_TESTS
bd dep add $IMPL_NEW_STORAGE $IDENTIFY_DEPS

IMPL_MIGRATION=$(bd create "Create migration script for existing session data to new storage format" -p 1 --label impl --silent)
bd dep add $IMPL_MIGRATION $IMPL_NEW_STORAGE

UPDATE_CONSUMERS=$(bd create "Update all consumer modules to use new auth middleware API surface" -p 1 --label impl --silent)
bd dep add $UPDATE_CONSUMERS $IMPL_NEW_STORAGE

# ========================================
# Phase 3: Testing & Validation
# ========================================

UNIT_TESTS=$(bd create "Add unit tests for new session storage implementation" -p 1 --label testing --silent)
bd dep add $UNIT_TESTS $IMPL_NEW_STORAGE

INTEGRATION_TESTS=$(bd create "Add integration tests for auth flow end-to-end with new middleware" -p 1 --label testing --silent)
bd dep add $INTEGRATION_TESTS $UPDATE_CONSUMERS

REGRESSION_CHECK=$(bd create "Run full regression suite and verify characterization tests still pass" -p 0 --label testing --silent)
bd dep add $REGRESSION_CHECK $INTEGRATION_TESTS
bd dep add $REGRESSION_CHECK $UNIT_TESTS

# ========================================
# Phase 4: Cleanup & Documentation
# ========================================

UPDATE_DOCS=$(bd create "Update auth middleware documentation and API reference" -p 2 --label docs --silent)
bd dep add $UPDATE_DOCS $REGRESSION_CHECK

CLEANUP=$(bd create "Remove deprecated session storage code and update changelog" -p 3 --label cleanup --silent)
bd dep add $CLEANUP $REGRESSION_CHECK

echo ""
echo "Bead graph created! View with:"
echo "  bd ready              # List unblocked tasks"
```

---

## Bead Creation Guidelines

### Priority Levels
- `-p 0` = Critical (blocking other work, or high-risk changes needing early validation)
- `-p 1` = High (important implementation work)
- `-p 2` = Medium (standard work)
- `-p 3` = Low (cleanup, nice-to-haves)

### Labels (Phase Grouping)
Use `--label` to group beads by phase:
- `analysis` - Understanding current state
- `prep` - Preparation work (characterization tests, feature flags, scaffolding)
- `impl` - Core implementation
- `testing` - Test coverage
- `migration` - Data/code migration
- `docs` - Documentation updates
- `cleanup` - Post-rollout cleanup

### Dependency Rules
1. Never create cycles
2. Analysis tasks should complete before implementation begins
3. Characterization tests should exist before changing code
4. Use `bd dep add CHILD PARENT` (child depends on parent completing first)
5. Parallel work should share a common ancestor, not depend on each other

### Task Granularity
- Each bead should be completable in **under 750 lines of code changed**
- Tasks should be atomic enough for one agent to complete without coordination
- If a task requires multiple file areas, consider splitting by file area

---

## Change-Specific Considerations

### For New Features
- Start with analysis of similar existing features
- Consider feature flag for gradual rollout
- Plan for A/B testing if relevant
- Include documentation and changelog updates

### For Refactors
- Add characterization tests first (capture current behavior)
- Consider strangler fig pattern for large changes
- Plan incremental migration path
- Ensure no behavior changes unless intentional

### For Migrations
- Create rollback plan as an explicit task
- Plan data validation checkpoints
- Consider dual-write period if applicable
- Include monitoring/alerting tasks

### For Performance Changes
- Add benchmarks before and after
- Include load testing tasks
- Plan gradual rollout with monitoring
- Have rollback criteria defined

---

## File Reservation Planning

For each major work area, note the file patterns that will need exclusive reservation:

```bash
# Example reservation notes (add as bead descriptions)
# CAUTION: These files have many consumers
# Auth refactor: src/auth/**, tests/auth/** (coordinate with API team)
# Shared utils: src/lib/utils.ts (high contention - keep changes minimal)
```

This helps agents claim appropriate file surfaces when they start work.

---

## Verification Steps

After generating the script:

1. **Run it**: `chmod +x setup-beads.sh && ./setup-beads.sh`
2. **Check ready work**: `bd ready` should show initial analysis/prep tasks

---

## Completeness Checklist

As-built closeout (2026-07-11): M3 consumes five real PAD_SET values decoded by
the pinned upstream `dh-inputlog` path and proves exact guest polls with no ring-I
input. M4 automatically publishes the byte-pinned `detsdk.stats` region, validates
the external capture fixture, passes 100 restore branches, and passes the full
600-second write-churn/reverify gate with zero moved extents. M5 completed the
one-time 1000/1000 campaign and the separate 10-iteration recurring push gate;
guest-sdk consumes decoded decisions while determinism-hypervisor owns DHILOG and
VerifyReplay serialization and services.

Ensure your task graph includes:

- [ ] Analysis of current implementation in affected areas
- [ ] Characterization tests for existing behavior
- [ ] Feature flag or gradual rollout mechanism (if applicable)
- [ ] Core implementation broken into small units
- [ ] Unit tests for new/changed code
- [ ] Integration tests for affected workflows
- [ ] Regression testing plan
- [ ] Documentation updates
- [ ] Migration scripts (if data changes)
- [ ] Rollback plan
- [ ] Cleanup tasks for post-rollout
- [ ] Clear dependency chains with no cycles
