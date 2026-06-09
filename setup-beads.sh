#!/bin/bash
# Project: guest-sdk — Phase 1 Deterministic Execution
# Generated: 2026-06-09
# Covers: M0 (detguest-wire), M1 (detguest-host), M2 (detguest-agent + image + harness)
# Tracks: wire → host || agent || image || workloads || harness; join at in-VM acceptance

set -euo pipefail

if ! command -v bd &>/dev/null; then
    echo "ERROR: 'bd' (beads) CLI not found on PATH." >&2
    exit 1
fi

if [ ! -d ".beads" ]; then
    bd init
fi

# Guard against duplicate graph creation on re-run
if bd list 2>/dev/null | grep -q .; then
    echo "ERROR: beads already exist in this repo. Run 'bd list' to review." >&2
    echo "       Delete .beads/ and re-run to recreate the graph from scratch." >&2
    exit 1
fi

echo "Creating Phase 1 task graph for guest-sdk..."
echo ""

# ============================================================
# Track 0: M0 — repo corrections + detguest-wire formats
# Prerequisite track. Everything else is a descendant of M0.
# ============================================================
echo "--- Track 0: M0 — detguest-wire ---"

WIRE_CORRECTION=$(bd create "Fix spec-contradicting skeleton API + relax unsafe policy" \
  -d "Delete the ad-hoc READY_RECORD=1/FRAME_MARK_RECORD=2/EVENT_RECORD=3 constants and 9-byte FrameMark encoding in crates/detguest-wire/src/lib.rs. Replace with spec-correct kinds per API.md §3.1 (Pad=0, NameIntern=2, FrameMark=13, Ready=14) using 16-byte record headers. Update detguest_agent::ready_record() consumer in crates/detguest-agent/src/lib.rs. Relax crate-level #![forbid(unsafe_code)] to module-scoped policy in BOTH crates: unsafe permitted only in wire::ring (detguest-wire) and in agent::translate + other documented modules (detguest-agent) per IMPLEMENTATION-PLAN M6 permitted-unsafe list. Files: crates/detguest-wire/src/lib.rs, crates/detguest-agent/src/lib.rs." \
  -p 0 -l wire --silent)

WIRE_LAYOUT=$(bd create "Implement ChannelHeader + ring descriptors + drop counters" \
  -d "Add crates/detguest-wire/src/header.rs: ChannelHeader (magic 0x5453_4555_4754_4544, proto_version u32, header_flags u32, ring_desc[4] of {offset: u32, size: u32}), drop counters at exact offsets per ARCHITECTURE.md §2 (ringA_dropped_records/bytes at 0x040/0x048, ringW_dropped_records/bytes at 0x050/0x058, ringW_dropped_by_kind[16] at 0x060). Ring producer/consumer index cache lines at 0x100..0x2C0. Const offset assertions (static_assert-style) so layout drift fails compilation. File: crates/detguest-wire/src/header.rs." \
  -p 0 -l wire --silent)
bd dep add "$WIRE_LAYOUT" "$WIRE_CORRECTION"

WIRE_RING=$(bd create "Implement wire::ring SPSC producer/consumer module" \
  -d "Add crates/detguest-wire/src/ring.rs: producer/consumer halves over free-running u32 indices (power-of-two ring sizes, mask = size-1). Memory ordering: producer Release-stores new index after writing record bytes; consumer Acquire-loads producer index, reads records, Release-stores new consumer index. Wrap/pad rule: if record does not fit in bytes remaining before ring end, write a Pad record (kind=0) covering the whole tail, start real record at offset 0. This is the ONLY permitted-unsafe module in detguest-wire (ring pointer arithmetic). Consumed by M1 loopback simulator, agent ring producers, and miri/loom CI jobs." \
  -p 0 -l wire --silent)
bd dep add "$WIRE_RING" "$WIRE_LAYOUT"

WIRE_RECORDS=$(bd create "Implement RecordHeader + all Event/Command/WorkloadCtrl payloads" \
  -d "Add crates/detguest-wire/src/record.rs + src/events.rs: RecordHeader (16 bytes: len u16, kind u8, flags u8, seq u32, vnanos u64). All EventKind variants 0..14 per API.md §3.1 (Pad, Hello, NameIntern, AssertViolation, Reachable, Beacon, InjectQuery, RegionRegister, RegionUpdate, WorkloadStarted, WorkloadExited, LogLine, QuiesceReady, FrameMark, Ready). CommandKind variants 1..6 (StartWorkload, Quiesce, Resume, Shutdown, SetLogMask, ReverifyRegions). WorkloadCtrlKind variants 2..3 (QuiesceReq, Resume). Encode/decode with no-wrap + Pad framing via wire::ring. All payload layouts per API.md §3.2/§3.3/§3.4." \
  -p 0 -l wire --silent)
bd dep add "$WIRE_RECORDS" "$WIRE_LAYOUT"
bd dep add "$WIRE_RECORDS" "$WIRE_RING"

WIRE_MANIFEST=$(bd create "Implement RegionManifest read/write + seqlock helpers" \
  -d "Add crates/detguest-wire/src/manifest.rs: ManifestHeader at channel offset 0x1000 (magic 0x4644_5444, manifest_version=1, region_capacity=64, generation u64 seqlock). RegionEntry[64] at 0x1020 (96 bytes each: region_id, name_id, layout_version, flags, gva, len, extent_off, extent_n, name[56]). ExtentPool[1024] at 0x2820 (16 bytes each: gpa u64, len u64). Seqlock writer discipline (generation +=1 to odd, full fence, mutate, full fence, +=1 to even). Reader discipline (retry while odd or generation changes). Total < 0x8000 per API.md §4.1." \
  -p 0 -l wire --silent)
bd dep add "$WIRE_MANIFEST" "$WIRE_LAYOUT"

WIRE_PORTS=$(bd create "Implement detcall port constants + FaultDecision pack/unpack" \
  -d "Add crates/detguest-wire/src/ports.rs: PIO port constants IDENT=0xD370, INIT_LO=0xD374, INIT_HI=0xD378, INIT_GO=0xD37C, DOORBELL=0xD380, INJECT=0xD384, QUIESCE_ACK=0xD388. FaultDecision enum (Proceed / Platform{kind: u8, arg: u32} / Workload{kind: u8, arg: u32}) with pack (bits 0..7 kind, bits 8..31 arg) and unpack. Golden round-trip values per IMPLEMENTATION-PLAN M0 acceptance: Proceed=0x0, Platform{2,512}=0x00020002, Workload{200,0xFFFFFF}." \
  -p 0 -l wire --silent)
bd dep add "$WIRE_PORTS" "$WIRE_CORRECTION"

WIRE_GOLDEN=$(bd create "Add golden binary fixtures + byte-exact encode/decode assertions" \
  -d "Create tests/golden/*.bin: one checked-in binary fixture per record kind with byte-exact assertions. Required by IMPLEMENTATION-PLAN M0 acceptance: truncated AssertViolation details, Pad at ring tail, max-size LogLine, dead manifest entry, packed FaultDecision values {Proceed, Platform{2,512}=0x00020002, Workload{200,0xFFFFFF}}. Tests assert decode(fixture)==expected struct and encode(struct)==fixture byte-for-byte. Directory: crates/detguest-wire/tests/golden/." \
  -p 0 -l testing --silent)
bd dep add "$WIRE_GOLDEN" "$WIRE_RECORDS"
bd dep add "$WIRE_GOLDEN" "$WIRE_MANIFEST"
bd dep add "$WIRE_GOLDEN" "$WIRE_PORTS"

WIRE_PROPTEST=$(bd create "Add proptest round-trip property tests for all wire types" \
  -d "Add proptest dev-dependency; write crates/detguest-wire/tests/proptest.rs: decode(encode(x))==x for all EventKind/CommandKind/WorkloadCtrlKind variants; decoder never panics on arbitrary byte input; all FaultDecision pack/unpack round-trips. Cover RegionManifest seqlock layout, all payload structs. These are the M0 acceptance round-trip gate." \
  -p 0 -l testing --silent)
bd dep add "$WIRE_PROPTEST" "$WIRE_RECORDS"
bd dep add "$WIRE_PROPTEST" "$WIRE_MANIFEST"
bd dep add "$WIRE_PROPTEST" "$WIRE_PORTS"

WIRE_FUZZ=$(bd create "Add cargo fuzz decode_record target (30-minute CI gate)" \
  -d "Create fuzz/fuzz_targets/decode_record.rs: feed arbitrary bytes to the record decoder, assert no panics and no UB. IMPLEMENTATION-PLAN M0 acceptance: cargo fuzz run decode_record for 30 minutes clean. Note: adding fuzz/ forces a workspace member-vs-excluded decision — this bead depends on HARNESS_WS_MECH (workspace mechanics) before it can be committed without breaking hosted CI lanes." \
  -p 0 -l testing --silent)
bd dep add "$WIRE_FUZZ" "$WIRE_RECORDS"
# WIRE_FUZZ also depends on HARNESS_WS_MECH — added in the forward-ref fix block below

WIRE_NOSTD=$(bd create "Verify no_std build: cargo test -p detguest-wire --no-default-features" \
  -d "Ensure detguest-wire compiles and tests pass with --no-default-features (no_std, no alloc feature). Guards against accidental std imports. Run: cargo test -p detguest-wire --no-default-features. This is the M0 acceptance no_std gate. Must not regress after any wire change." \
  -p 0 -l testing --silent)
bd dep add "$WIRE_NOSTD" "$WIRE_RECORDS"
bd dep add "$WIRE_NOSTD" "$WIRE_MANIFEST"
bd dep add "$WIRE_NOSTD" "$WIRE_PORTS"

WIRE_MIRI=$(bd create "Add miri tests for wire::ring index arithmetic" \
  -d "Write unit tests in crates/detguest-wire/src/ring.rs (or tests/) that exercise ring pointer arithmetic, wrap-at-boundary, and acquire/release fence placement. Run under miri (cargo miri test -p detguest-wire) to catch UB. Tests: producer advances past ring end → Pad emitted, consumer wraps correctly, index overflow (free-running u32) handled. Miri catches missing fences and pointer provenance violations." \
  -p 1 -l testing --silent)
bd dep add "$WIRE_MIRI" "$WIRE_RING"

WIRE_LOOM=$(bd create "Add loom tests for SPSC producer/consumer interleavings" \
  -d "Add loom dev-dependency; write crates/detguest-wire/tests/loom_ring.rs: model-check all possible producer/consumer interleavings via loom. Exercise: concurrent push+drain, wrap-boundary races, Pad-record visibility. Run: RUSTFLAGS='--cfg loom' cargo test -p detguest-wire --test loom_ring. Catches missing memory barriers. IMPLEMENTATION-PLAN testing strategy: 'loom for producer/consumer interleavings'." \
  -p 1 -l testing --silent)
bd dep add "$WIRE_LOOM" "$WIRE_RING"

echo ""
echo "--- Track 1: M1 — detguest-host (parallel with M2 after M0) ---"

# ============================================================
# Track 1: M1 — detguest-host
# Parallel with M2 sub-tracks after M0.
# CARVE-OUT: HOST_CRATE is also a parent of HARNESS_GUESTMEM.
# ============================================================

HOST_CRATE=$(bd create "Create detguest-host crate: GuestMem + ChannelWriteSink traits + mock" \
  -d "Scaffold crates/detguest-host/ as new workspace member (crates/* glob covers it — add to Cargo.toml workspace.members if needed). Must depend ONLY on detguest-wire; must NOT take determinism-proto dependency per ARCHITECTURE.md §1. Define: GuestMem trait (fn read(&self, gpa: u64, buf: &mut [u8])->Result<(),MemError>; fn write(&mut self, gpa: u64, buf: &[u8])->Result<(),MemError>), ChannelWriteSink trait (ring_push, cons_bump, pio_answer), Vec<u8>-backed MockGuestMem, RingId enum (C/I/A/W), error types (AttachError, WireError, PushError, RegionReadError, MemError). Files: crates/detguest-host/src/lib.rs, crates/detguest-host/src/guestmem.rs." \
  -p 0 -l host --silent)
bd dep add "$HOST_CRATE" "$WIRE_RECORDS"
bd dep add "$HOST_CRATE" "$WIRE_MANIFEST"

HOST_ATTACH=$(bd create "Implement Channel::attach + header validation paths" \
  -d "Add crates/detguest-host/src/channel.rs: Channel<M: GuestMem> struct. Channel::attach(gm: M, base_gpa: u64)->Result<Self, AttachError>: read ChannelHeader via GuestMem::read, validate magic==0x5453_4555_4754_4544, proto_version==1, each ring descriptor within [0, 2MiB), ring size is power of two. Return specific AttachError variants (BadMagic, BadVersion, RingOutOfBounds, BadRingSize, AlreadyAttached) — the PIO INIT_GO handler turns these into nonzero init status for the guest's IN 0xD37C response." \
  -p 1 -l host --silent)
bd dep add "$HOST_ATTACH" "$HOST_CRATE"

HOST_DRAIN=$(bd create "Implement drain_events over rings A and W" \
  -d "Add crates/detguest-host/src/drain.rs: Channel::drain_events(&mut self, sink: &mut dyn ChannelWriteSink)->Result<Vec<GuestEvent>, WireError>. Drain all complete records from rings A and W in (ring, seq) order. Stop at last complete record (partial records mid-write tolerated by stopping there — do not partially decode). Skip Pad records (kind=0). Skip unknown kinds by advancing len bytes. Bump consumer indices through ChannelWriteSink::cons_bump after draining each ring. GuestEvent fields: ring, seq, vnanos, truncated (flags bit0), payload (EventPayload enum)." \
  -p 1 -l host --silent)
bd dep add "$HOST_DRAIN" "$HOST_ATTACH"

HOST_PUSH=$(bd create "Implement push_command + push_workload_ctrl with ChannelWriteSink" \
  -d "Add crates/detguest-host/src/commands.rs: Channel::push_command(&mut self, cmd: &Command, sink: &mut dyn ChannelWriteSink)->Result<(), PushError> and push_workload_ctrl for ring I. Encode CommandKind/WorkloadCtrlKind payloads per API.md §3.3/§3.4; host-produced records have vnanos=0. Report write through ChannelWriteSink::ring_push so the hypervisor can append it to the input log. Return PushError::RingFull if no space — host may retry at next pause, never spins guest. NORMATIVE: never push pad input on ring I; that travels exclusively via pv-pad MMIO latch (ARCHITECTURE.md §2)." \
  -p 1 -l host --silent)
bd dep add "$HOST_PUSH" "$HOST_ATTACH"

HOST_MANIFEST=$(bd create "Implement read_manifest with seqlock retry" \
  -d "Add crates/detguest-host/src/manifest.rs: Channel::read_manifest(&self)->Result<RegionManifest, WireError>. Seqlock-consistent read: read generation (retry if odd), copy header + RegionEntry[64] + ExtentPool[1024] via GuestMem::read, re-read generation; retry on change. resolve(&self, name: &str)->Option<(Vec<Extent>, layout_version)>. After snapshot restore the manifest is immediately valid (guest RAM) — no event replay needed." \
  -p 1 -l host --silent)
bd dep add "$HOST_MANIFEST" "$HOST_ATTACH"

HOST_REGION=$(bd create "Implement read_region extent walk" \
  -d "Add Channel::read_region(&self, name: &str, offset: u64, buf: &mut [u8])->Result<(), RegionReadError>: call read_manifest, resolve name to Vec<Extent>, walk extents concatenating via GuestMem::read per extent. Errors: NameNotFound, ExtentOutOfBounds, GuestMemError. M1 acceptance requirement: correctly stitches a 3-extent region across a discontiguous mock layout (GPA ranges non-contiguous in MockGuestMem)." \
  -p 1 -l host --silent)
bd dep add "$HOST_REGION" "$HOST_MANIFEST"

HOST_INJECT=$(bd create "Implement InjectResponder + FaultPlan + TableFaultPlan + LogFaultPlan" \
  -d "Add crates/detguest-host/src/inject.rs: FaultPlan trait (fn decide(&mut self, iseq: u32, name_id: u32, name: Option<&str>)->FaultDecision); InjectResponder<P: FaultPlan> holds last-drained InjectQuery records; InjectResponder::answer(iseq, sink)->u32 matches the query, calls plan.decide, packs the FaultDecision, reports via ChannelWriteSink::pio_answer. TableFaultPlan impl (rule table for tests: match by name glob + occurrence index). LogFaultPlan skeleton (replay mode — reads from input log; final wiring in determinism-hypervisor; skeleton returns Proceed with a TODO note)." \
  -p 1 -l host --silent)
bd dep add "$HOST_INJECT" "$HOST_DRAIN"

HOST_INTERN=$(bd create "Implement intern-table maintenance from NameIntern events" \
  -d "In crates/detguest-host/src/drain.rs or channel.rs: fold NameIntern events (kind=2) into a name_id->String table per Channel. Table must be checkpointed alongside hypervisor per-branch state (reconstructible from event stream, caching avoids re-scans). Expose Channel::intern_name(id: u32)->Option<&str>. Handle REACHABLE_DECL flag (flags bit1 per API.md §3.2 — mark the entry as a declared-but-not-yet-hit reachability target)." \
  -p 1 -l host --silent)
bd dep add "$HOST_INTERN" "$HOST_DRAIN"

HOST_LOOPBACK=$(bd create "M1 acceptance: pure-host loopback test with 10^5 mixed events" \
  -d "Write crates/detguest-host/tests/loopback.rs: guest-side simulator using detguest-wire producer code against MockGuestMem produces 10^5 mixed events including ring wrap, Pad records, drops, and a NameIntern registration. Assertions (all from IMPLEMENTATION-PLAN M1 acceptance): drain_events recovers exactly the non-dropped sequence; drop counters match simulator bookkeeping; every host mutation appears exactly once in the recorded ChannelWriteSink trace. Separate assertion: read_region correctly stitches a 3-extent region across a discontiguous mock layout." \
  -p 0 -l testing --silent)
bd dep add "$HOST_LOOPBACK" "$HOST_DRAIN"
bd dep add "$HOST_LOOPBACK" "$HOST_PUSH"
bd dep add "$HOST_LOOPBACK" "$HOST_REGION"
bd dep add "$HOST_LOOPBACK" "$HOST_INJECT"
bd dep add "$HOST_LOOPBACK" "$HOST_INTERN"
bd dep add "$HOST_LOOPBACK" "$WIRE_RING"

echo ""
echo "--- Track 2a: M2 — detguest-agent binary ---"

# ============================================================
# Track 2a: M2 — detguest-agent (parallel with M1 + other M2 tracks)
# ============================================================

AGENT_INIT=$(bd create "Agent: static musl binary scaffold + init mounts + hugetlbfs alloc" \
  -d "Scaffold crates/detguest-agent/src/main.rs as PID 1 entry point. Set up .cargo/config.toml with [target.x86_64-unknown-linux-musl] for the static musl cross-build. Init sequence (ARCHITECTURE.md §4 steps 1-3): mount /proc, /sys, devtmpfs; mount hugetlbfs at /dev/hugepages; open hugetlbfs file, ftruncate 2 MiB, mmap MAP_SHARED, memset 0. Write ChannelHeader (magic, proto_version=1, ring descriptors for C/I/A/W at offsets per ARCHITECTURE.md §2 channel layout). PID 1 entry is via initramfs /init shim exec'ing /sbin/detguest-agent — no other init binary exists in any image (IMPLEMENTATION-PLAN dependency notes)." \
  -p 1 -l agent --silent)
bd dep add "$AGENT_INIT" "$WIRE_LAYOUT"

AGENT_DETCALL=$(bd create "Agent: pagemap GVA->GPA + CHANNEL_INIT detcall + Hello + doorbell" \
  -d "Add crates/detguest-agent/src/translate.rs (unsafe module, permitted by WIRE_CORRECTION unsafe relaxation): read /proc/self/pagemap for the 2 MiB hugetlb mmap to obtain PFN; GPA = pfn << 12 (identity by construction of the VM memory map). CHANNEL_INIT detcall sequence (ARCHITECTURE.md §4 steps 4-6): OUT 0xD374 (gpa_lo), OUT 0xD378 (gpa_hi), OUT 0xD37C (size=512 pages), IN 0xD37C; nonzero status = boot fault (power off). On status=0: set header_flags.agent_ready=1; write Hello{proto_version=1, agent_version, capabilities} to ring A + Release-store prod index; OUT 0xD380 (doorbell, ring A mask=0x1). Inline asm for iopl(3), OUT/IN wrappers in src/pio.rs." \
  -p 1 -l agent --silent)
bd dep add "$AGENT_DETCALL" "$AGENT_INIT"
bd dep add "$AGENT_DETCALL" "$WIRE_PORTS"
bd dep add "$AGENT_DETCALL" "$WIRE_RECORDS"
bd dep add "$AGENT_DETCALL" "$WIRE_RING"

AGENT_BOOT=$(bd create "Agent: boot.toml parsing + boot fault path (API.md §7)" \
  -d "Parse /etc/detguest/boot.toml (TOML, toml crate): boot_toml_version (reject unknown major — boot fault), [[unit]] entries (id, exec, args, log_mask, optional [unit.control]{protocol, proto_version, game_dev}), [autostart]{unit}, [[expected_region]]{name, layout_version}. Validation per API.md §7.2: dense unique ids, absolute exec paths, duplicate region names = boot fault. Boot fault path (API.md §7.3): emit LogLine(stream=3, level=0) with error detail, never emit Ready, reboot(RB_POWER_OFF). Scaffold src/commands.rs with ring-C consumer loop stub." \
  -p 1 -l agent --silent)
bd dep add "$AGENT_BOOT" "$AGENT_DETCALL"
bd dep add "$AGENT_BOOT" "$WIRE_RECORDS"

AGENT_READY=$(bd create "Agent: boot-manifest autostart + deterministic READY-point emission" \
  -d "Implement ARCHITECTURE.md §4 step 7: if [autostart] configured, start unit locally using same code path as StartWorkload (no ring-C command — autostart is agent-local so no host input precedes READY); wait until every [[expected_region]] is live in manifest at its pinned layout_version; emit Ready{unit, region_count, manifest_generation} on ring A + doorbell. With no [autostart]: emit Ready immediately after Hello with region_count=0. ARCHITECTURE.md §4.1 READY-point contract: icount at the Ready doorbell exit is a pure function of the WorkloadImage — bit-reproducible across boots of the same image. M2 acceptance: trivial autostart with empty expected_regions → Ready arrives + bit-identical icount across 10 consecutive boots." \
  -p 1 -l agent --silent)
bd dep add "$AGENT_READY" "$AGENT_BOOT"

AGENT_SUPERVISE=$(bd create "Agent: supervise loop — LogLine drain + waitpid + WorkloadExited" \
  -d "Implement crates/detguest-agent/src/supervise.rs: single-threaded epoll loop (pipes + signalfd). ARCHITECTURE.md §4 steps 9-10: drain workload stdout/stderr pipes into LogLine events on ring A (stream=1 stdout, stream=2 stderr; droppable). On pipe EOF / SIGCHLD: waitpid -> WorkloadExited{guest_pid, exit_code, term_signal} (critical, ring A + doorbell). Emit WorkloadStarted{guest_pid, unit} after fork+exec. M2 acceptance: 'WorkloadExited semantics verified with trivial baked-in workload that prints to stdout — host receives LogLine events with correct stream/level framing' — uncheckable without this supervise loop." \
  -p 1 -l agent --silent)
bd dep add "$AGENT_SUPERVISE" "$AGENT_READY"

AGENT_RINGC=$(bd create "Agent: ring-C poll loop + StartWorkload + WorkloadStarted" \
  -d "Implement ring-C consumer in src/commands.rs: poll ring C on every supervise loop pass and on SIGCHLD (deterministic cadence per ARCHITECTURE.md §4 step 8). Handle StartWorkload{unit, log_mask}: set up stdout/stderr pipes, set DETGUEST_CHANNEL_FD env var, set RLIMIT_MEMLOCK=unlimited, fork+exec the boot manifest's [[unit]][id].exec + args, emit WorkloadStarted{guest_pid, unit} on ring A. Apply log_mask. The `unit` field selects among boot manifest's preconfigured unit entries by id — argv is NEVER sent over the wire (ARCHITECTURE.md §4, keeping the wire small and the image immutable)." \
  -p 1 -l agent --silent)
bd dep add "$AGENT_RINGC" "$AGENT_BOOT"
bd dep add "$AGENT_RINGC" "$AGENT_SUPERVISE"

AGENT_SHUTDOWN=$(bd create "Agent: Shutdown + ReverifyRegions stub + reboot(RB_POWER_OFF)" \
  -d "Handle Shutdown{mode} from ring C: graceful: SIGTERM workload, 2s virtual-time grace, SIGKILL, emit WorkloadExited if workload was running, sync(), reboot(RB_POWER_OFF); immediate: skip grace period. M2 acceptance: 'Shutdown{graceful} powers off the VM'. Also add ReverifyRegions handler stub (re-walk /proc/<pid>/pagemap for all live manifest regions, emit RegionUpdate — can be a functional stub for M2 but must not panic on receipt)." \
  -p 1 -l agent --silent)
bd dep add "$AGENT_SHUTDOWN" "$AGENT_RINGC"

echo ""
echo "--- Track 2b: M2 — image/ (kernel config + initramfs builder) ---"

# ============================================================
# Track 2b: M2 — image/
# MAP.md clean-room rule: cmdline owned by hypervisor, must not be invented here.
# ============================================================

DOCS_CMDLINE=$(bd create "File doc issue: canonical kernel cmdline not in this repo's doc set" \
  -d "Per MAP.md clean-room source boundary: the canonical deterministic kernel cmdline (including flags such as norandmaps, kernel.randomize_va_space=0) is owned by determinism-hypervisor ARCHITECTURE.md §2.3, which is NOT in this repo's local doc set (prompts/docs/). Agents working on image/ must not invent cmdline flags from external sources. File a documentation issue requesting the operator supply the canonical cmdline doc or a minimal cross-reference. The image kernel config covers CONFIG_* build options only — not cmdline. This bead is a clean-room compliance gate for image/ work." \
  -p 0 -l docs --silent)

IMAGE_KERNEL_ACQUIRE=$(bd create "Decide kernel source acquisition and version pinning for image/" \
  -d "Make and document an explicit decision in image/KERNEL.md: (1) which kernel version to pin (must support determinism config: COMPACTION=n, MIGRATION=n, KSM=n, no THP, no swap, single CPU, hugetlbfs, perf_event PERF_COUNT_HW_INSTRUCTIONS_RETIRED, devtmpfs, /proc, /sys); (2) where the source comes from (tarball URL + SHA256, or git tag + hash); (3) build artifact caching strategy (rebuild only on config or version change; CI cache key based on kernel version + config hash). This decision feeds both IMAGE_KERNEL_CONFIG (config file) and HARNESS_KVM_BASIC (harness needs to know the pinned kernel). No cmdline flags set here — owned by hypervisor (DOCS_CMDLINE)." \
  -p 0 -l image --silent)

IMAGE_KERNEL_CONFIG=$(bd create "Write image/kernel.config: minimal deterministic kernel config" \
  -d "Create image/kernel.config at the pinned kernel version from IMAGE_KERNEL_ACQUIRE. Required options per IMPLEMENTATION-PLAN M2 + ARCHITECTURE.md §5: CONFIG_COMPACTION=n, CONFIG_MIGRATION=n, CONFIG_KSM=n, CONFIG_TRANSPARENT_HUGEPAGE=n, no swap (CONFIG_SWAP=n), CONFIG_NUMA=n, single CPU. Also required: CONFIG_HUGETLBFS=y, CONFIG_PERF_EVENTS=y (for retired-instruction counter in tests/vm/), CONFIG_DEVTMPFS=y, CONFIG_PROC_FS=y, CONFIG_SYSFS=y. Coordinate with reference-workload (which bakes its emulator into this same image per IMPLEMENTATION-PLAN M2). NORMATIVE: cmdline flags (norandmaps etc.) are NOT here — owned by determinism-hypervisor ARCHITECTURE.md §2.3." \
  -p 1 -l image --silent)
bd dep add "$IMAGE_KERNEL_CONFIG" "$IMAGE_KERNEL_ACQUIRE"
bd dep add "$IMAGE_KERNEL_CONFIG" "$DOCS_CMDLINE"

IMAGE_INITRAMFS=$(bd create "Write image/build.sh: kernel build + initramfs cpio + /init shim" \
  -d "Create image/build.sh (or cargo xtask image): download kernel source at pinned version (IMAGE_KERNEL_ACQUIRE), apply image/kernel.config, build bzImage. Build initramfs cpio layout: /init (tiny static shim that exec's /sbin/detguest-agent as PID 1 — the image's only init binary; no dh-init exists anywhere), /sbin/detguest-agent (cross-compiled x86_64-unknown-linux-musl static binary), /etc/detguest/boot.toml (M2 fixture: [autostart] unit=0, [[unit]] id=0 exec=/usr/bin/autostart, empty [[expected_region]] list), test workloads in /usr/bin/ from WORKLOADS_AUTOSTART and WORKLOADS_STDOUT. Output: image/bzImage + image/initramfs.cpio.gz. This is the ONLY kernel build in this repo; tests/vm/ consumes these outputs." \
  -p 1 -l image --silent)
bd dep add "$IMAGE_INITRAMFS" "$IMAGE_KERNEL_CONFIG"
# IMAGE_INITRAMFS also depends on WORKLOADS_AUTOSTART + WORKLOADS_STDOUT — added below

echo ""
echo "--- Track 2c: M2 — test workloads (tests/vm/workloads/) ---"

# ============================================================
# Track 2c: M2 — test workloads
# ============================================================

WORKLOADS_AUTOSTART=$(bd create "Write + cross-compile trivial autostart workload" \
  -d "Create tests/vm/workloads/autostart/main.rs: minimal Rust binary that exits 0 immediately. Cross-compile x86_64-unknown-linux-musl (static). This workload exercises the READY-point reproducibility contract: boot.toml fixture has [autostart] unit=0, [[unit]] id=0 exec=/usr/bin/autostart, empty [[expected_region]] list (so Ready fires right after the unit starts — testing the zero-region path). M2 acceptance: bit-identical icount across 10 consecutive boots of the same image." \
  -p 1 -l workloads --silent)
bd dep add "$WORKLOADS_AUTOSTART" "$WIRE_RECORDS"
# WORKLOADS_AUTOSTART also depends on HARNESS_WS_MECH — added below

WORKLOADS_STDOUT=$(bd create "Write + cross-compile stdout/stderr printing workload" \
  -d "Create tests/vm/workloads/stdout_printer/main.rs: minimal binary that writes a known line to stdout, a known line to stderr, then exits with code 0. Cross-compile x86_64-unknown-linux-musl (static). Purpose: exercise agent supervise loop LogLine stream/level framing (stream=1 stdout, stream=2 stderr per API.md §3.2 LogLine payload) and WorkloadExited emission. M2 acceptance: host harness receives LogLine events with correct stream/level framing and WorkloadExited with expected exit_code=0." \
  -p 1 -l workloads --silent)
bd dep add "$WORKLOADS_STDOUT" "$WIRE_RECORDS"
# WORKLOADS_STDOUT also depends on HARNESS_WS_MECH — added below

echo ""
echo "--- Track 2d: M2 — tests/vm/ KVM test harness ---"

# ============================================================
# Track 2d: M2 — tests/vm/ KVM test harness
# Parallel with M1/M2; HARNESS_GUESTMEM has carve-out dep on HOST_CRATE.
# ============================================================

HARNESS_WS_MECH=$(bd create "Workspace mechanics: add tests/vm/ + fuzz/ without breaking CI" \
  -d "Make explicit decisions and implement them BEFORE any harness or fuzz commit — without this bead the first harness commit breaks every hosted CI lane (IMPLEMENTATION-PLAN specific requirement). Decide and configure: (1) tests/vm/ membership — exclude from workspace [members] or use a feature gate / DETGUEST_KVM_TESTS env gate to keep KVM-requiring tests out of hosted ubuntu-latest lanes; (2) fuzz/ membership — cargo fuzz crates are typically workspace-excluded; (3) how miri/loom tests are run without the KVM binary pulling in Linux-specific deps. Document the decision in Cargo.toml comments and update .github/workflows/ci.yaml workspace-level steps accordingly." \
  -p 0 -l harness --silent)
bd dep add "$HARNESS_WS_MECH" "$WIRE_CORRECTION"

# Fix the forward-referenced deps now that HARNESS_WS_MECH is defined
bd dep add "$WIRE_FUZZ" "$HARNESS_WS_MECH"
bd dep add "$WORKLOADS_AUTOSTART" "$HARNESS_WS_MECH"
bd dep add "$WORKLOADS_STDOUT" "$HARNESS_WS_MECH"
bd dep add "$IMAGE_INITRAMFS" "$WORKLOADS_AUTOSTART"
bd dep add "$IMAGE_INITRAMFS" "$WORKLOADS_STDOUT"

HARNESS_RUNNER_PROVISION=$(bd create "Provision Intel-box self-hosted GitHub Actions runner" \
  -d "Set up a self-hosted GitHub Actions runner on the VT-x Intel box: install runner agent, add to kvm group (or set /dev/kvm permissions), verify perf_event_paranoid<=1 (required for retired-instruction counting), install Rust toolchain + x86_64-unknown-linux-musl target, label the runner with 'self-hosted', 'intel', 'kvm'. Document provisioning steps in docs/ or .github/. This runner is the gate for the entire in-VM test tier. Long lead time — start immediately, independent of all code work." \
  -p 0 -l ci --silent)

HARNESS_PREFLIGHT=$(bd create "Intel-box preflight verification script (phase entry gate)" \
  -d "Write scripts/preflight-intel.sh (or tests/vm/src/bin/preflight.rs): verify (1) pinned kernel version is booted (uname -r matches IMAGE_KERNEL_ACQUIRE decision); (2) perf_event_paranoid<=1 (/proc/sys/kernel/perf_event_paranoid); (3) KVM capabilities: KVM_CAP_USER_MEMORY, KVM_CAP_HLT, KVM_CAP_IRQCHIP accessible on /dev/kvm; (4) /dev/kvm readable/writable by runner user. Phase 1 entry requirement per phase-1-deterministic-execution.md: 'Intel box preflight passed — pinned kernel, perf_event access, KVM caps'. Run as the first step of every Intel-box CI job; the entire in-VM tier depends on this." \
  -p 0 -l harness --silent)
bd dep add "$HARNESS_PREFLIGHT" "$HARNESS_RUNNER_PROVISION"
bd dep add "$HARNESS_PREFLIGHT" "$IMAGE_KERNEL_ACQUIRE"

HARNESS_KVM_BASIC=$(bd create "tests/vm: minimal KVM runner + PIO handler for detcall ports" \
  -d "Create tests/vm/src/lib.rs: minimal KVM VM setup using kvm-ioctls crate or raw /dev/kvm ioctls. Steps: KVM_CREATE_VM, add userspace memslot covering the 2 GiB physical address space (mmap'd), load bzImage + initramfs.cpio.gz from image/ output, set up vCPU registers (long mode, GDT, initial RSP/RIP), vCPU run loop. PIO handler: intercept KVM_EXIT_IO on ports 0xD370–0xD388 and dispatch to channel protocol handlers. Consumes image/bzImage + image/initramfs.cpio.gz — this is the join point where the one kernel build in this repo (image/) meets the harness." \
  -p 1 -l harness --silent)
bd dep add "$HARNESS_KVM_BASIC" "$HARNESS_WS_MECH"
bd dep add "$HARNESS_KVM_BASIC" "$HARNESS_PREFLIGHT"
bd dep add "$HARNESS_KVM_BASIC" "$IMAGE_KERNEL_ACQUIRE"
bd dep add "$HARNESS_KVM_BASIC" "$IMAGE_INITRAMFS"

HARNESS_GUESTMEM=$(bd create "tests/vm: implement GuestMem trait over KVM memslot mapping" \
  -d "Add tests/vm/src/guest_mem.rs: implement the detguest-host GuestMem trait over the KVM VM's userspace memory mapping (the mmap used for the memslot). GuestMem::read translates GPA to host VA via (host_base + gpa), bounds-checks against memslot size, memcpy to buf. GuestMem::write similarly. EXPLICIT CARVE-OUT DEPENDENCY: depends on HOST_CRATE (M1 early bead) because the GuestMem trait is defined in detguest-host, not in the harness (ARCHITECTURE.md §1: 'sdk, agent, host all depend on wire. Nothing else.') — the harness plugs this impl into Channel::attach to drive the full host-side channel protocol." \
  -p 1 -l harness --silent)
bd dep add "$HARNESS_GUESTMEM" "$HOST_CRATE"
bd dep add "$HARNESS_GUESTMEM" "$HARNESS_WS_MECH"
bd dep add "$HARNESS_GUESTMEM" "$HARNESS_KVM_BASIC"

HARNESS_MMIO=$(bd create "tests/vm: trivial pv-pad MMIO latch stub" \
  -d "Handle KVM_EXIT_MMIO for the pv-pad MMIO range (base GPA 0xD000_1000, registers PAD0..PAD3 at base+0x08+4*port, FRAME_COUNTER at base+0x00 per determinism-hypervisor ARCHITECTURE.md §6.4 — cite that doc, do not invent addresses). Implement a latch stub: store written values per register, return last-written value on read. M2 stub can be minimal — just must not panic on MMIO exits to this range. Required for M3's poll_input/frame_mark tests. Latch address is owned by determinism-hypervisor; cite it." \
  -p 1 -l harness --silent)
bd dep add "$HARNESS_MMIO" "$HARNESS_KVM_BASIC"

HARNESS_ICOUNT=$(bd create "tests/vm: perf_event retired-instruction counter" \
  -d "Implement tests/vm/src/icount.rs: perf_event-based retired-instruction counter for the vCPU thread. Open perf_event_open(PERF_TYPE_HARDWARE, PERF_COUNT_HW_INSTRUCTIONS_RETIRED, pid=vCPU_thread, cpu=-1, group=-1, PERF_FLAG_FD_CLOEXEC); read counter at VM exits via ioctl(fd, PERF_EVENT_IOC_READ). M2 icount gate is UNCHECKABLE without this: IMPLEMENTATION-PLAN M2 acceptance requires 'Ready doorbell exit lands at a bit-identical icount across 10 consecutive boots — measured by the harness retired-instruction counter'. Intel box only. perf_event access requires perf_event_paranoid<=1 (verified by HARNESS_PREFLIGHT). This is the hardest harness work item — consider splitting into (a) fd management, (b) vCPU thread pinning." \
  -p 0 -l harness --silent)
bd dep add "$HARNESS_ICOUNT" "$HARNESS_KVM_BASIC"

HARNESS_GUEST_TIME=$(bd create "tests/vm: guest-time measurement for < 1s boot criterion" \
  -d "Implement tests/vm/src/guest_time.rs: measure elapsed guest time from VM power-on to the Ready doorbell PIO exit. Method: record host wall time at VM start and at Ready PIO exit (sufficient proxy for guest time in a single-CPU, no-wait boot; or use vcpu run time from KVM_GET_VCPU_EVENTS). M2 acceptance: 'VM boots to agent in < 1 s guest time'. Report measurement in test output. Fail the test if > 1s to catch regressions from oversized initramfs or misconfigured kernel." \
  -p 1 -l harness --silent)
bd dep add "$HARNESS_GUEST_TIME" "$HARNESS_KVM_BASIC"

echo ""
echo "--- Join: M2 in-VM acceptance (Intel box only) ---"

# ============================================================
# Join: M2 in-VM acceptance gate
# Joins M1 (via HOST_LOOPBACK) + all M2 sub-tracks.
# CANNOT be claimed done on host-only checks (spec requirement).
# ============================================================

HARNESS_VM_ACCEPTANCE=$(bd create "M2 in-VM acceptance test suite (Intel box; cannot verify on macOS)" \
  -d "Write tests/vm/tests/m2_acceptance.rs covering ALL M2 acceptance criteria from IMPLEMENTATION-PLAN — IN-VM, Intel box only. Per spec: in-VM acceptance gates CANNOT claim done on host-only checks. Criteria: (1) VM boots to agent in <1s guest time (HARNESS_GUEST_TIME); (2) host sees IN 0xD370 = 0xD37E0001 (IDENT), INIT_GO status=0, Hello with proto_version=1; (3) trivial autostart workload (empty expected_regions): Ready arrives + doorbell exit at bit-identical icount across 10 consecutive boots (HARNESS_ICOUNT — gate is uncheckable without this); (4) Shutdown{graceful} powers off the VM; (5) stdout_printer workload: host receives LogLine events with correct stream/level framing (stream=1 stdout, stream=2 stderr) + WorkloadExited with exit_code=0. Joins M1 (HOST_LOOPBACK) + all M2 sub-tracks." \
  -p 0 -l harness --silent)
bd dep add "$HARNESS_VM_ACCEPTANCE" "$HARNESS_GUESTMEM"
bd dep add "$HARNESS_VM_ACCEPTANCE" "$HARNESS_MMIO"
bd dep add "$HARNESS_VM_ACCEPTANCE" "$HARNESS_ICOUNT"
bd dep add "$HARNESS_VM_ACCEPTANCE" "$HARNESS_GUEST_TIME"
bd dep add "$HARNESS_VM_ACCEPTANCE" "$AGENT_READY"
bd dep add "$HARNESS_VM_ACCEPTANCE" "$AGENT_SUPERVISE"
bd dep add "$HARNESS_VM_ACCEPTANCE" "$AGENT_SHUTDOWN"
bd dep add "$HARNESS_VM_ACCEPTANCE" "$IMAGE_INITRAMFS"
bd dep add "$HARNESS_VM_ACCEPTANCE" "$HOST_LOOPBACK"

echo ""
echo "--- Track 3: CI lanes (explicit tasks per spec requirement) ---"

# ============================================================
# Track 3: CI lanes
# Spec: "CI must be decomposed into explicit tasks, not asserted as policy."
# Current CI: single ubuntu-latest fmt/build/test job — must be extended.
# ============================================================

CI_NOSTD=$(bd create "CI job: cargo test -p detguest-wire --no-default-features" \
  -d "Update .github/workflows/ci.yaml: add a dedicated step or job running cargo test -p detguest-wire --no-default-features on ubuntu-latest. This is the M0 no_std acceptance gate. Must run on every PR. Also add to the aarch64 lane (CI_AARCH64) since detguest-wire has zero x86-specific code. The dual-checkout of control-plane (existing ci.yaml pattern) must be preserved." \
  -p 1 -l ci --silent)
bd dep add "$CI_NOSTD" "$WIRE_NOSTD"

CI_FUZZ=$(bd create "CI job: scheduled cargo fuzz run decode_record (30-minute gate)" \
  -d "Add a scheduled workflow (.github/workflows/fuzz.yaml or nightly job): runs cargo fuzz run decode_record -- -max_total_time=1800 on ubuntu-latest. M0 acceptance fuzz gate. Commit fuzz/corpus/ seeds. Requires HARNESS_WS_MECH workspace decision for fuzz/ directory inclusion/exclusion. Consider: fuzz job only on schedule (not every PR) to save CI minutes; block merge if last scheduled run failed." \
  -p 1 -l ci --silent)
bd dep add "$CI_FUZZ" "$WIRE_FUZZ"

CI_MIRI=$(bd create "CI job: miri on wire::ring index logic" \
  -d "Add CI job: MIRIFLAGS='-Zmiri-ignore-leaks' cargo +nightly miri test -p detguest-wire (targets the ring.rs unit tests). Catches UB in ring pointer arithmetic and missing fence operations. ubuntu-latest. Run on every PR touching crates/detguest-wire/src/ring.rs. Requires HARNESS_WS_MECH workspace decisions (miri runs on the workspace; tests/vm/ KVM deps must not be pulled in)." \
  -p 1 -l ci --silent)
bd dep add "$CI_MIRI" "$WIRE_MIRI"
bd dep add "$CI_MIRI" "$HARNESS_WS_MECH"

CI_LOOM=$(bd create "CI job: loom producer/consumer interleaving tests" \
  -d "Add CI job: RUSTFLAGS='--cfg loom' cargo test -p detguest-wire --test loom_ring on ubuntu-latest. Model-checks all SPSC producer/consumer interleavings. May be slow — consider running only on schedule or on changes to src/ring.rs, not every PR. Requires HARNESS_WS_MECH for workspace exclusion decisions (loom tests pull in loom dep; must not conflict with no_std feature)." \
  -p 1 -l ci --silent)
bd dep add "$CI_LOOM" "$WIRE_LOOM"
bd dep add "$CI_LOOM" "$HARNESS_WS_MECH"

CI_MUSL=$(bd create "CI job: x86_64-unknown-linux-musl agent cross-build" \
  -d "Add CI job to .github/workflows/ci.yaml: cargo build --target x86_64-unknown-linux-musl -p detguest-agent on ubuntu-latest with musl-tools (apt-get install musl-tools + rustup target add x86_64-unknown-linux-musl). Verifies the agent compiles as a static musl binary on every PR. Add the target to the existing dtolnay/rust-toolchain@stable step or a separate toolchain step. Must pass before any M2 in-VM work is attempted." \
  -p 1 -l ci --silent)
bd dep add "$CI_MUSL" "$AGENT_INIT"

CI_AARCH64=$(bd create "CI job: aarch64 runner lane for wire + host tests" \
  -d "Add a CI job running on aarch64 (the DGX Spark per MAP.md, or ubuntu-latest-arm): cargo test -p detguest-wire and cargo test -p detguest-host. IMPLEMENTATION-PLAN CI tiering: 'wire+host tests run everywhere including aarch64'. Requires HARNESS_WS_MECH workspace exclusion decisions so tests/vm/ KVM tests are not compiled on aarch64. Update .github/workflows/ci.yaml with a matrix runner or separate job with runs-on: [self-hosted, aarch64] or ubuntu-24.04-arm." \
  -p 1 -l ci --silent)
bd dep add "$CI_AARCH64" "$HOST_LOOPBACK"
bd dep add "$CI_AARCH64" "$HARNESS_WS_MECH"

CI_INTEL_INVM=$(bd create "CI job: Intel-box self-hosted runner gating in-VM test tier" \
  -d "Add CI job to .github/workflows/ci.yaml with runs-on: [self-hosted, intel, kvm]: runs HARNESS_PREFLIGHT script then cargo test -p guest-sdk-vm-tests (or equivalent binary in tests/vm/). Gated to main-branch pushes and release PRs (not every PR — Intel box is a shared resource). Separate from all hosted-runner jobs; its failure must not block non-KVM CI. Wire+host tests still run everywhere; only in-VM tests are Intel-gated per IMPLEMENTATION-PLAN CI tiering." \
  -p 1 -l ci --silent)
bd dep add "$CI_INTEL_INVM" "$HARNESS_VM_ACCEPTANCE"

echo ""
echo "=============================================="
echo "Phase 1 guest-sdk bead graph created!"
echo ""
echo "  bd ready     # Show immediately unblocked tasks"
echo "  bd graph     # Show full dependency graph"
echo "  bd list      # Show all beads"
echo ""
echo "Initial unblocked tasks (no dependencies):"
echo "  - WIRE_CORRECTION   (p0) fix skeleton API + unsafe policy"
echo "  - DOCS_CMDLINE      (p0) file cmdline ownership doc issue"
echo "  - IMAGE_KERNEL_ACQUIRE (p0) kernel version/source decision"
echo "  - HARNESS_RUNNER_PROVISION (p0) provision Intel-box runner"
echo "=============================================="
