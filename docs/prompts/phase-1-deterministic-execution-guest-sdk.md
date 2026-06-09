# Project Planning with Beads

## Agent Instructions

You are an expert software architect creating a comprehensive task breakdown. This task graph will be executed by AI agents working in parallel, coordinated through MCP Agent Mail with file reservations to prevent conflicts.

<quality_expectations>
Create a thorough, production-ready task graph. Include all necessary setup, implementation, testing, and documentation tasks. Go beyond the basics - consider edge cases, error handling, security considerations, and integration points. Each task should be specific enough for an agent to execute independently without ambiguity.
</quality_expectations>

## Project Information

### Links to Relevant Documentation

Local copies (canonical for this work — read `prompts/docs/MAP.md` first; note its clean-room source boundary):

- `prompts/docs/MAP.md` — system map, mission, core principles, clean-room source boundary
- `prompts/docs/phase-1-deterministic-execution.md` — Phase 1 scope, cross-repo ordering, exit gate
- `prompts/docs/guest-sdk/README.md` — repo overview
- `prompts/docs/guest-sdk/ARCHITECTURE.md` — channel layout, ring discipline, agent boot path (§4), READY-point contract (§4.1)
- `prompts/docs/guest-sdk/API.md` — record kinds, detcall ports, manifest format, boot.toml (§7)
- `prompts/docs/guest-sdk/IMPLEMENTATION-PLAN.md` — milestone-by-milestone work items and acceptance criteria (M1 and M2 are this phase's scope)
- `prompts/docs/guest-sdk/INTEGRATION.md` — cross-repo contracts

Originals live in `/Users/punk1290/.agents/projects/determinism/` (docs/guest-sdk/, phases/, docs/MAP.md).

### Project Description

Implement Phase 1 of the determinism program for the `guest-sdk` repo, exactly as scoped by `prompts/docs/phase-1-deterministic-execution.md` ("guest-sdk — parallel track"):

- **Milestone 1 — `detguest-host` over a mock `GuestMem`** (host-side protocol crate; zero hypervisor dependency): `GuestMem` trait + `Vec<u8>`-backed mock; `Channel::attach` validation paths; `drain_events` (both rings, ordering, partial records mid-write tolerated by stopping at the last complete record); `push_command` / `push_workload_ctrl`; `read_manifest` with seqlock retry; `read_region` extent walk; `ChannelWriteSink` plumbed through every mutation; `InjectResponder` + `FaultPlan` trait with `TableFaultPlan` (tests) and `LogFaultPlan` skeleton; intern-table maintenance from `NameIntern` events.
- **Milestone 2 — `detguest-agent` boots as PID 1, channel up, logs flow**: static musl binary; init mounts; hugetlbfs channel alloc; pagemap self-translation; CHANNEL_INIT detcall; Hello; `boot.toml` parsing + boot fault path; boot-manifest autostart + `Ready` emission (the deterministic READY point); ring-C poll loop; `StartWorkload` handling + `WorkloadStarted` emission (the autostart path reuses it); the supervise loop (`src/supervise.rs`, ARCHITECTURE.md §4 steps 9–10: drain workload stdout/stderr pipes into `LogLine` events with correct stream/level framing, `waitpid` → `WorkloadExited` — the quoted M2 acceptance is uncheckable without this); Shutdown. Plus three sub-tracks the milestone implies:
  - **`image/`** — minimal deterministic kernel config + initramfs builder script; init path is the initramfs `/init` shim exec'ing `/sbin/detguest-agent`. The kernel config file must be coordinated with `reference-workload`, which bakes its emulator into this same image (IMPLEMENTATION-PLAN M2). Kernel cmdline flags such as `norandmaps` come from the canonical cmdline owned by `determinism-hypervisor` ARCHITECTURE.md §2.3 — that doc is NOT in this repo's local doc set, so the task graph must include either an operator-supplied doc-import step or a "file a documentation issue" task per the MAP.md clean-room rule; do not invent the cmdline. The builder also needs an explicit kernel source acquisition and version-pinning decision (where the kernel tree comes from, which version is pinned, whether build artifacts are cached). There is exactly **one kernel build** in this repo: `image/` owns the config and the build; `tests/vm/` consumes the built kernel at the join (the implementation plan's "own kernel build" means *this repo's* build as opposed to the hypervisor's — not a second sibling build). The source-acquisition/pinning bead feeds both consumers.
  - **Test workloads** — the two trivial workloads M2 acceptance requires must be written, cross-compiled (musl), and baked into the initramfs alongside `boot.toml` fixtures: an autostart workload with an empty expected-regions list, and a baked-in workload that prints to stdout (exercising `LogLine` stream/level framing and `WorkloadExited`). They live in `tests/vm/workloads/` (the same home IMPLEMENTATION-PLAN M3 gives `testload`), carry the `workloads` label, and feed the in-VM acceptance join.
  - **`tests/vm/`** — the repo's own minimal KVM test harness: boots the `image/`-built kernel, PIO handler, an implementation of `detguest-host`'s `GuestMem` trait over the memslot (this bead depends on the early M1 trait bead — see the parallel-tracks carve-out in Specific Requirements), trivial pv-pad MMIO latch stub, a perf_event/PMU-based **retired-instruction counter** (the M2 icount gate is "measured by the harness's retired-instruction counter" per IMPLEMENTATION-PLAN M2 acceptance and is uncheckable without it — this is the hardest harness work item and deserves its own dedicated task(s)), and a guest-time measurement for the < 1 s boot criterion.

  No hypervisor dependency: per the phase doc and the implementation plan's dependency notes, M2 boots under this repo's own harness, not `determinism-hypervisor`.

Entry dependency: Milestone 0 (`detguest-wire` formats + golden tests) is Phase 0 exit-gate work and is **effectively unbuilt** — the task graph must include the full M0 work list from IMPLEMENTATION-PLAN.md as a prerequisite track: `ChannelHeader` + ring descriptors + drop counters with const offset assertions, the **`wire::ring` SPSC producer/consumer module** (`src/ring.rs`: producer/consumer halves, free-running u32 indices, acquire/release discipline, wrap/pad rules — consumed by M1's loopback simulator, the agent's ring producers, and the miri/loom CI jobs; it needs its own bead), `RecordHeader` + all EventKind/CommandKind/WorkloadCtrlKind payloads (incl. `FrameMark` and `Ready`) with no-wrap + `Pad` framing, `RegionManifest` read/write with seqlock helpers, detcall port constants + `FaultDecision` pack/unpack, golden binary fixtures, proptest round-trips, the `decode_record` fuzz target, and the no_std build check. Two repo-state corrections are mandatory M0 work: (a) the existing skeleton API in `crates/detguest-wire/src/lib.rs` **contradicts the spec** (`READY_RECORD=1`, `FRAME_MARK_RECORD=2`, `EVENT_RECORD=3` with an ad-hoc 9-byte FrameMark encoding, vs. API.md §3.1's Pad=0, NameIntern=2, FrameMark=13, Ready=14 with 16-byte record headers) and must be replaced, including its consumer `detguest_agent::ready_record()`; (b) the crate-level `#![forbid(unsafe_code)]` must be relaxed to a module-scoped policy — unsafe is permitted only in the documented `wire::ring` ring-pointer module, and an agent following the existing crate attribute cannot write that module. The same correction applies to `crates/detguest-agent/src/lib.rs`, whose crate-level `#![forbid(unsafe_code)]` blocks M2 work (hugetlbfs mmap, PIO inline asm for detcalls, pagemap self-translation): unsafe there is permitted only in the documented modules (e.g. `agent::translate`, per IMPLEMENTATION-PLAN M6's permitted-unsafe list).

### Technical Stack

- Rust, cargo workspace (edition 2021, resolver 2), existing crates `detguest-wire`, `detguest-agent`, `m0-proto-client`; new crate `detguest-host`
- `detguest-wire`: `#![no_std]` + `alloc` feature; zero unsafe outside documented ring-pointer code (`wire::ring`)
- `detguest-agent`: static musl binary (x86_64-unknown-linux-musl), runs as PID 1 in a minimal initramfs image
- `tests/vm/`: minimal KVM runner (raw KVM ioctls or `kvm-ioctls`/`kvm-bindings`), own minimal kernel build (determinism config: `CONFIG_COMPACTION=n`, `CONFIG_MIGRATION=n`, `CONFIG_KSM=n`, no THP, no swap, single CPU)
- Testing: golden binary fixtures, `proptest` round-trips, `cargo fuzz` decoder target, `miri` for ring index logic, `loom` for producer/consumer interleavings, loopback simulator over mock `GuestMem`
- CI tiering: wire+host tests run everywhere (including aarch64); in-VM tiers gated to the Intel box (KVM)
- Cross-repo dependency constraint: the workspace `Cargo.toml` carries a path dependency `determinism-proto = { path = "../control-plane/crates/determinism-proto" }`, consumed only by `m0-proto-client` (a Phase 0 artifact — keep it as-is, no Phase 1 work on it). Every build/test/CI task therefore requires the sibling `control-plane` checkout (the existing `.github/workflows/ci.yaml` already does a dual checkout — preserve it). Per ARCHITECTURE.md §1 ("`sdk`, `agent`, `host` all depend on `wire`. Nothing else."), the new `detguest-host` crate must depend only on `detguest-wire` and must NOT take the `determinism-proto` dependency.

### Specific Requirements

Follow `prompts/docs/phase-1-deterministic-execution.md` for guest-sdk, with milestone acceptance criteria from `prompts/docs/guest-sdk/IMPLEMENTATION-PLAN.md`:

- **Phase 1 exit gate (guest-sdk item):** the agent boots in-guest and streams log events host-ward.
- **M1 acceptance:** pure-host loopback test — a guest-side simulator (using `detguest-wire` producer code against the same mock memory) produces 10⁵ mixed events incl. wrap, pad, drops, and a registration; `drain_events` recovers exactly the non-dropped sequence; drop counters match the simulator's bookkeeping; every host mutation appears exactly once in the recorded `ChannelWriteSink` trace. `read_region` correctly stitches a 3-extent region across a discontiguous mock layout.
- **M2 acceptance (in-VM, Intel box):** VM boots to agent in < 1 s guest time; host sees IDENT, INIT_GO status 0, Hello with `proto_version 1`. With a trivial autostart workload (empty expected-regions list): `Ready` arrives and its doorbell exit lands at a bit-identical icount across 10 consecutive boots of the same image. `Shutdown{graceful}` powers off the VM; `WorkloadExited` semantics verified with a trivial baked-in workload that prints to stdout (host receives `LogLine` events with correct stream/level framing).
- Determinism failures anywhere are P0 per MAP.md.
- `ChannelWriteSink` is a required parameter on every mutating host call — no mutate-without-sink API may exist.
- Respect the clean-room source boundary in MAP.md: project docs + public references only; file a documentation issue rather than filling spec gaps from external sources.
- **CI must be decomposed into explicit tasks**, not asserted as policy. Current CI is a single `ubuntu-latest` fmt/build/test job. The acceptance criteria imply concrete jobs the graph must create: `cargo test --no-default-features` (no_std build, M0 acceptance), the 30-minute `cargo fuzz run decode_record` job, `miri` on the ring index logic, `loom` interleaving tests, the `x86_64-unknown-linux-musl` cross-build of the agent, an aarch64 runner lane for wire+host tests, and an Intel-box self-hosted runner (provisioning, labeling, KVM access) gating the in-VM tier.
- **In-VM acceptance gates cannot run on the dev machine (macOS).** Every in-VM acceptance criterion must be carried by either a self-hosted Intel-runner CI task or an explicit human-gated verification bead — no M2 bead may claim "done" on host-only checks.
- **Parallel tracks and join points**: after the M0 prerequisite track, M1 (`detguest-host`) and the M2 implementation sub-tracks (agent binary, `image/`, test workloads, `tests/vm/` harness) can run in parallel; the in-VM M2 acceptance joins M1 and all M2 sub-tracks. Parallel work shares the M0 ancestor rather than depending on sibling tracks, with **one explicit carve-out**: the harness beads that implement `GuestMem` over the memslot and drive the channel depend on the early M1 bead that creates the `detguest-host` crate and `GuestMem` trait (API.md §2 / ARCHITECTURE.md §1 place the trait in `detguest-host`, not in the harness); the rest of M1 (drain/inject/manifest) stays parallel to the harness.
- **Workspace mechanics need their own bead**: workspace members are `crates/*` and CI runs `cargo test --workspace` on hosted runners. Adding `tests/vm/` and the fuzz crate forces explicit decisions — workspace member vs. excluded, and how KVM-requiring tests are kept out of the default hosted lanes (feature gate, `#[ignore]`, separate binary, or env gate). Without this bead the first harness commit breaks every hosted CI lane.
- **Intel-box preflight verification**: the Phase 1 entry requirement is "Intel box preflight passed — pinned kernel, perf_event access, KVM caps" (phase doc). The retired-instruction counter and the bit-identical-icount gate depend directly on perf_event access. Add a preflight-verification bead that the entire in-VM tier depends on, alongside the self-hosted-runner provisioning bead.

---

## Your Task

Analyze this project and create a comprehensive **Beads task graph** using the `bd` CLI. Beads provides dependency-aware, conflict-free task management for multi-agent execution.

---

<critical_constraint>
Your ONLY output is a bash shell script. Do NOT use `bd add` — the correct command to create a bead is `bd create`. Use `bd dep add` for dependencies. Do not implement anything yourself.
</critical_constraint>

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
# Generated: 2026-06-09

set -e

# Initialize beads if needed
if [ ! -d ".beads" ]; then
    bd init
fi

echo "Creating project beads..."

# ========================================
# Track 0: M0 prerequisite — detguest-wire
# ========================================

WIRE_LAYOUT=$(bd create "Replace detguest-wire skeleton with spec ChannelHeader + ring descriptors" \
  -d "Delete the spec-conflicting skeleton API (READY_RECORD=1 etc.); implement API.md/ARCHITECTURE.md §2 layout with const offset assertions. Relax forbid(unsafe_code) to module-scoped (wire::ring only)." \
  -p 0 --label wire --silent)

WIRE_RECORDS=$(bd create "Implement RecordHeader + all Event/Command/WorkloadCtrl payloads" \
  -d "API.md §3 kinds incl. FrameMark(13) and Ready(14), no-wrap + Pad framing. Update detguest_agent::ready_record() consumer." \
  -p 0 --label wire --silent)
bd dep add $WIRE_RECORDS $WIRE_LAYOUT

WIRE_GOLDEN=$(bd create "Golden fixtures + proptest round-trips + decode_record fuzz target" \
  -p 0 --label testing --silent)
bd dep add $WIRE_GOLDEN $WIRE_RECORDS

# ========================================
# Track 1: M1 — detguest-host (parallel with Track 2 after M0)
# ========================================

HOST_GUESTMEM=$(bd create "Create detguest-host crate: GuestMem trait + Vec-backed mock" \
  -d "Depends only on detguest-wire — must NOT take the determinism-proto dependency (ARCHITECTURE.md §1)." \
  -p 0 --label host --silent)
bd dep add $HOST_GUESTMEM $WIRE_RECORDS

HOST_DRAIN=$(bd create "Channel::attach validation + drain_events over both rings" \
  -p 0 --label host --silent)
bd dep add $HOST_DRAIN $HOST_GUESTMEM

# ========================================
# Track 2: M2 — agent / image / tests-vm (parallel with Track 1)
# ========================================

HARNESS_ICOUNT=$(bd create "tests/vm: perf_event retired-instruction counter" \
  -d "Required to check the M2 bit-identical-icount gate mechanically. Intel box only." \
  -p 0 --label harness --silent)
bd dep add $HARNESS_ICOUNT $WIRE_RECORDS

# ... continue for all tracks; in-VM M2 acceptance joins Track 1 + Track 2 ...

echo ""
echo "Bead graph created! View with:"
echo "  bd ready              # List unblocked tasks"
```

---

## Bead Creation Guidelines

### Priority Levels
- `-p 0` = Critical (blocking other work)
- `-p 1` = High (important but not blocking)
- `-p 2` = Medium (standard work)
- `-p 3` = Low (nice to have)

### Labels (Phase Grouping)
Use `--label` to group beads by track:
- `wire` - M0 prerequisite: `detguest-wire` formats
- `host` - M1: `detguest-host` crate
- `agent` - M2: `detguest-agent` binary
- `image` - M2: kernel config + initramfs builder (`image/`)
- `harness` - M2: `tests/vm/` KVM runner (incl. retired-instruction counter)
- `workloads` - M2: trivial test workloads baked into the initramfs (`tests/vm/workloads/`)
- `testing` - Golden vectors, proptest, fuzz, miri, loom, loopback simulator
- `ci` - CI lanes: no_std, fuzz, miri/loom, musl cross-build, aarch64, Intel-box self-hosted in-VM tier
- `docs` - Documentation issues (e.g., canonical kernel cmdline ownership) and as-built updates

### Dependency Rules
1. Never create cycles
2. Every bead should have a clear dependency chain back to setup tasks
3. Use `bd dep add CHILD PARENT` (child depends on parent completing first)
4. Parallel work should share a common ancestor, not depend on each other

### Task Granularity
- Each bead should be completable in **under 750 lines of code**
- Tasks should be atomic enough for one agent to complete without coordination
- If a task requires multiple file areas, consider splitting by file area

---

## File Reservation Planning

For each major work area, note the file patterns that will need exclusive reservation:

```bash
# Reservation notes (add as bead descriptions)
# Wire crate (M0):    crates/detguest-wire/**, fuzz/**
# Host crate (M1):    crates/detguest-host/**
# Agent binary (M2):  crates/detguest-agent/**
# Image build (M2):   image/**
# VM harness (M2):    tests/vm/** (excluding workloads/)
# Test workloads (M2): tests/vm/workloads/**
# CI lanes:           .github/**
# Workspace manifest: Cargo.toml (coordinate — touched by crate-creation beads)
```

This helps agents claim appropriate file surfaces when they start work. The track structure: M0 (`wire`) is the shared ancestor; `host` (M1) and the four M2 surfaces (`agent`, `image`, `workloads`, `harness`) proceed in parallel after it — except the harness's `GuestMem`-impl/channel-driving beads, which also depend on the early M1 `GuestMem`-trait bead (see Specific Requirements). The in-VM M2 acceptance beads join `host` + `agent` + `image` + `workloads` + `harness`.

---

## Context Documentation

Place any important context in `prompts/docs/` for agents to reference. This includes:
- Architecture decisions
- API documentation
- Design system specs
- External service integration guides

---

## Verification Steps

After generating the script:

1. **Run it**: `chmod +x setup-beads.sh && ./setup-beads.sh`
2. **Check ready work**: `bd ready` should show initial setup tasks

---

## Completeness Checklist

Ensure your task graph includes:

- [ ] All setup and configuration tasks
- [ ] Core architecture and shared utilities
- [ ] Feature implementation tasks (broken into small units)
- [ ] Error handling and edge cases
- [ ] Unit and integration tests for each feature
- [ ] API documentation
- [ ] Security considerations (input validation, auth checks)
- [ ] Performance considerations where relevant
- [ ] CI/CD and deployment tasks
- [ ] Clear dependency chains with no cycles
