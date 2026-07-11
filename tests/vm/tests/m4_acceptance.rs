//! Ms4 platform-readability acceptance (plan package 06 §C; bead
//! `guest-sdk-m4-platform-readability-vm`; Phase 3 exit gate item 2):
//!
//! > emulator RAM region readable from the host and stable across 100×
//! > snapshot/restore.
//!
//! One root VM boots the m4-regions fixture (Ready gated on `wram` +
//! `framebuffer` at the D7 length + `meta` publishing through the real
//! agent-IPC registration path), warms up, and is snapshotted. N children
//! (default 100, `DETGUEST_M4_CHILDREN` override for local iteration — the
//! evidence file records the actual count) each:
//!
//!   1. restore and prove **restore fidelity**: all three regions read
//!      bit-identical to the root baseline BEFORE the child runs;
//!   2. run 60 frames with a child-specific pv-pad input schedule
//!      (children `2k`/`2k+1` share a seed → determinism pairs; different
//!      seeds must steer wram apart);
//!   3. prove the reads are real: meta frame counter and the workload's
//!      FNV-1a input-history hash match host-side recomputation;
//!   4. exercise `ReverifyRegions` on the restored guest: one RegionUpdate
//!      echo per live region, zero P0 alarms.
//!
//! All region reads go through the real host path (`detguest-host`
//! `read_manifest`/`read_region` over guest memory) and never send a
//! command or run the vCPU — that is the point of the milestone.
//!
//! A final fork-of-fork leg proves readability holds one restore level
//! deeper. Durable evidence (region dumps, SHA-256 table, environment)
//! lands under `target/m4-acceptance-<UTC>Z/` — same discipline as the
//! hypervisor's M9 acceptance artifacts.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command as Proc, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use detguest_host::OwnedPayload;
use detguest_vmtest::harness::snapshot::VmSnapshot;
use detguest_vmtest::harness::{StopReason, VmConfig, VmHarness};

/// Warm-up boundary: the root runs to this FRAME_COUNTER value before the
/// root snapshot is taken.
const WARMUP_FRAME: u32 = 8;
/// Frames each child runs under its input schedule.
const CHILD_FRAMES: u32 = 60;
/// Frames the grandchild (fork-of-fork) runs.
const GRANDCHILD_FRAMES: usize = 10;
/// D7 layout_version 1 framebuffer contract.
const FRAMEBUFFER_LEN: usize = 229_376;
/// Live regions the fixture publishes (drives the reverify echo count).
const LIVE_REGIONS: usize = 3;

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn gated() -> bool {
    if !detguest_vmtest::vm_tests_enabled() {
        eprintln!("skipping: DETGUEST_VM_TESTS != 1");
        return false;
    }
    assert!(
        detguest_vmtest::kvm_available(),
        "DETGUEST_VM_TESTS=1 but /dev/kvm not accessible"
    );
    true
}

fn child_count() -> usize {
    std::env::var("DETGUEST_M4_CHILDREN")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100)
}

fn churn_duration() -> Duration {
    Duration::from_secs(
        std::env::var("DETGUEST_M4_CHURN_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(600),
    )
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

struct Artifacts {
    bzimage: PathBuf,
    initramfs_m4: PathBuf,
}

static ARTIFACTS: OnceLock<Artifacts> = OnceLock::new();

/// Same recipe as `m4_snapshot.rs` (each integration-test binary builds its
/// own staging dir; build.sh output is cached by key).
fn artifacts() -> &'static Artifacts {
    ARTIFACTS.get_or_init(|| {
        let root = repo_root();
        run(
            &root,
            "cargo",
            &[
                "build",
                "--release",
                "--target",
                "x86_64-unknown-linux-musl",
                "-p",
                "detguest-agent",
                "-p",
                "detguest-workloads",
            ],
        );
        run(&root, "./image/build.sh", &["kernel"]);

        let musl = root.join("target/x86_64-unknown-linux-musl/release");
        let build = root.join("image/build");
        let dir = build.join("m4-stage-acceptance");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sbin")).unwrap();
        std::fs::create_dir_all(dir.join("opt")).unwrap();
        std::fs::create_dir_all(dir.join("etc/detguest")).unwrap();
        std::fs::copy(musl.join("detguest-agent"), dir.join("sbin/detguest-agent")).unwrap();
        std::fs::copy(musl.join("m4-regions"), dir.join("opt/m4-regions")).unwrap();
        std::fs::copy(
            root.join("image/boot.toml.m4-regions"),
            dir.join("etc/detguest/boot.toml"),
        )
        .unwrap();
        run(
            &root,
            "./image/build.sh",
            &["initramfs", dir.to_str().unwrap()],
        );
        let out_path = build.join("initramfs-m4-acceptance.cpio");
        std::fs::rename(build.join("initramfs.cpio"), &out_path).unwrap();
        Artifacts {
            bzimage: build.join("bzImage"),
            initramfs_m4: out_path,
        }
    })
}

fn run(cwd: &Path, prog: &str, args: &[&str]) {
    let st = Proc::new(prog)
        .args(args)
        .current_dir(cwd)
        .status()
        .unwrap_or_else(|e| panic!("spawn {prog}: {e}"));
    assert!(st.success(), "{prog} {args:?} failed: {st}");
}

fn stdout_of(prog: &str, args: &[&str]) -> String {
    let out = Proc::new(prog).args(args).output().expect(prog);
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn m4_config() -> VmConfig {
    let a = artifacts();
    VmConfig::new(a.bzimage.clone(), a.initramfs_m4.clone())
}

/// SHA-256 via the host `sha256sum` binary (no in-tree hash dependency).
fn sha256(bytes: &[u8]) -> String {
    let mut child = Proc::new("sha256sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn sha256sum");
    child.stdin.as_mut().unwrap().write_all(bytes).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .expect("sha256sum output")
        .to_string()
}

/// Host-side region read: manifest resolve + extent walk over guest memory.
/// Never sends a command or runs the vCPU.
fn read_region_bytes(vm: &VmHarness, name: &str) -> Vec<u8> {
    let ch = vm.channel.as_ref().expect("channel attached");
    let manifest = ch.read_manifest().expect("manifest read");
    let region = manifest
        .resolve(name)
        .unwrap_or_else(|| panic!("region {name:?} not live in the manifest"));
    let mut buf = vec![0u8; region.len as usize];
    ch.read_region(name, 0, &mut buf)
        .unwrap_or_else(|e| panic!("read_region({name:?}): {e:?}"));
    buf
}

const REGION_NAMES: [&str; LIVE_REGIONS] = ["wram", "framebuffer", "meta"];

fn read_all_regions(vm: &VmHarness) -> [Vec<u8>; LIVE_REGIONS] {
    [
        read_region_bytes(vm, "wram"),
        read_region_bytes(vm, "framebuffer"),
        read_region_bytes(vm, "meta"),
    ]
}

/// Deterministic per-(seed, frame) pad value — must match the host-side
/// input-history recomputation below.
fn pad_value(seed: u32, frame: u32) -> u32 {
    seed.wrapping_mul(0x9e37_79b9)
        .wrapping_add(frame.wrapping_mul(0x85eb_ca6b))
}

/// Recompute the workload's FNV-1a input-history hash for polls 0..=`last`:
/// the pv-pad latch starts at 0, persists across frames, and takes the
/// scheduled value for frame F at frame F's poll.
fn recompute_input_hash(schedule: &BTreeMap<u32, u32>, last: u32) -> u64 {
    let mut latch = 0u32;
    let mut hash = FNV_OFFSET_BASIS;
    for frame in 0..=last {
        if let Some(&v) = schedule.get(&frame) {
            latch = v;
        }
        for byte in latch.to_le_bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME);
        }
    }
    hash
}

/// The workload writes meta = [frame_le, _, acc_le, input_hash_le, ...].
fn meta_frame(meta: &[u8]) -> u32 {
    u32::from_le_bytes(meta[0..4].try_into().unwrap())
}

fn meta_input_hash(meta: &[u8]) -> u64 {
    u64::from_le_bytes(meta[16..24].try_into().unwrap())
}

fn region_update_count(vm: &VmHarness) -> usize {
    vm.observed
        .events
        .iter()
        .filter(|e| matches!(e.payload, OwnedPayload::RegionUpdate(_)))
        .count()
}

fn p0_agent_loglines(vm: &VmHarness) -> Vec<String> {
    vm.observed
        .events
        .iter()
        .filter_map(|e| match &e.payload {
            OwnedPayload::LogLine { stream, level, msg }
                if *stream == detguest_wire::events::log_stream::AGENT && *level == 0 =>
            {
                Some(String::from_utf8_lossy(msg).into_owned())
            }
            _ => None,
        })
        .collect()
}

/// Run one child: restore-fidelity check, scheduled 60-frame run, content
/// proofs, then a ReverifyRegions echo pass. Returns (region hashes, the
/// child itself for optional fork-of-fork use).
fn run_child(
    snap: &VmSnapshot,
    baseline: &[Vec<u8>; LIVE_REGIONS],
    index: usize,
    seed: u32,
) -> ([String; LIVE_REGIONS], VmHarness) {
    let cfg = m4_config();
    let mut child = VmHarness::from_snapshot(&cfg, snap)
        .unwrap_or_else(|e| panic!("child {index}: from_snapshot: {e}"));

    // (1) Restore fidelity: before the child runs a single instruction, the
    // regions must read bit-identical to the root baseline.
    let fresh = read_all_regions(&child);
    for (i, name) in REGION_NAMES.iter().enumerate() {
        assert_eq!(
            fresh[i], baseline[i],
            "child {index}: {name} diverged from the root baseline immediately after restore"
        );
    }

    // (2) Schedule 60 frames of seed-derived inputs and run to the exact
    // frame boundary. `base` equals the warm-up counter for every child
    // (fresh restore, nothing has run), so schedules are identical for a
    // seed — the determinism pairs depend on it.
    let base = child.pvpad().frame_counter;
    assert_eq!(
        base, WARMUP_FRAME,
        "child {index}: unexpected restore point"
    );
    let start = base + 1;
    let mut schedule = BTreeMap::new();
    for f in start..start + CHILD_FRAMES {
        let v = pad_value(seed, f);
        schedule.insert(f, v);
        child.pvpad().schedule(f, 0, v);
    }
    let target = start + CHILD_FRAMES;
    let reason = child
        .run_until(Duration::from_secs(60), |o| {
            o.frame_counter_writes.last().is_some_and(|&v| v >= target)
        })
        .unwrap_or_else(|e| panic!("child {index}: scheduled run: {e}"));
    assert_eq!(
        reason,
        StopReason::Predicate,
        "child {index} must reach frame {target}; serial:\n{}",
        child.serial_text()
    );
    assert_eq!(
        *child.observed.frame_counter_writes.last().unwrap(),
        target,
        "child {index}: frame-boundary stop must be exact for pair determinism"
    );

    // (3) Content proofs: the reads reflect real, current guest state.
    let regions = read_all_regions(&child);
    assert_eq!(
        regions[1].len(),
        FRAMEBUFFER_LEN,
        "child {index}: framebuffer must be exactly {FRAMEBUFFER_LEN} bytes"
    );
    let last_frame = target - 1; // meta written before the counter bump
    assert_eq!(
        meta_frame(&regions[2]),
        last_frame,
        "child {index}: meta frame counter must reflect the 60 scheduled frames"
    );
    assert_eq!(
        meta_input_hash(&regions[2]),
        recompute_input_hash(&schedule, last_frame),
        "child {index}: workload input-history hash must match the host-side \
         recomputation (seed {seed})"
    );
    // The reads changed vs. the baseline (a fake/no-op registration path
    // could not pass this + the fidelity check simultaneously).
    assert_ne!(
        regions[0], baseline[0],
        "child {index}: wram must have advanced after 60 frames"
    );

    // (4) ReverifyRegions on the restored guest: echoes only, no P0 alarms.
    let before_updates = region_update_count(&child);
    child.push_command(&detguest_wire::Command::ReverifyRegions);
    let reason = child
        .run_until(Duration::from_secs(30), |o| {
            o.events
                .iter()
                .filter(|e| matches!(e.payload, OwnedPayload::RegionUpdate(_)))
                .count()
                >= before_updates + LIVE_REGIONS
        })
        .unwrap_or_else(|e| panic!("child {index}: reverify run: {e}"));
    assert_eq!(
        reason,
        StopReason::Predicate,
        "child {index}: expected {LIVE_REGIONS} RegionUpdate echoes; serial:\n{}",
        child.serial_text()
    );
    let alarms = p0_agent_loglines(&child);
    assert!(
        alarms.is_empty(),
        "child {index}: ReverifyRegions raised P0 alarms on a healthy restore: {alarms:?}"
    );

    let hashes = [
        sha256(&regions[0]),
        sha256(&regions[1]),
        sha256(&regions[2]),
    ];
    (hashes, child)
}

#[test]
#[ignore = "KVM tier: Intel runner only (DETGUEST_VM_TESTS=1)"]
fn regions_readable_and_stable_across_100_snapshot_restore_branches() {
    if !gated() {
        return;
    }
    let started = Instant::now();
    let n = child_count();
    assert!(n >= 2, "need at least one determinism pair");

    // Root: boot to warm-up through the real registration path (Ready is
    // gated on the three expected regions), snapshot, baseline.
    let cfg = m4_config();
    let mut root = VmHarness::new(&cfg).expect("root harness");
    let reason = root
        .run_until(Duration::from_secs(120), |o| {
            o.frame_counter_writes
                .last()
                .is_some_and(|&v| v >= WARMUP_FRAME)
        })
        .expect("root warm-up");
    assert_eq!(
        reason,
        StopReason::Predicate,
        "root must reach frame {WARMUP_FRAME}; serial:\n{}",
        root.serial_text()
    );
    let snap = root.snapshot().expect("root snapshot");
    let baseline = read_all_regions(&root);
    assert_eq!(baseline[1].len(), FRAMEBUFFER_LEN);

    // Negative control (a no-op registration path cannot pass this): the
    // manifest entry is real — nonzero extents at a GPA that is not the
    // channel page, and wram is being written by the guest.
    {
        let ch = root.channel.as_ref().unwrap();
        let manifest = ch.read_manifest().unwrap();
        let wram = manifest.resolve("wram").expect("wram live");
        assert!(!wram.extents.is_empty(), "wram must have real extents");
        assert!(
            wram.extents.iter().all(|e| e.gpa != 0),
            "wram extents must carry real GPAs"
        );
    }
    drop(root);

    // The 100 branches. Sequential: one live child at a time bounds RSS.
    let mut per_child: Vec<[String; LIVE_REGIONS]> = Vec::with_capacity(n);
    let mut last_child: Option<VmHarness> = None;
    for i in 0..n {
        let seed = (i / 2) as u32;
        let (hashes, child) = run_child(&snap, &baseline, i, seed);
        per_child.push(hashes);
        if i + 1 == n {
            last_child = Some(child);
        }
        if (i + 1) % 20 == 0 {
            eprintln!("[m4-acceptance] {}/{} children done", i + 1, n);
        }
    }

    // Determinism pairs: children 2k and 2k+1 share a seed and must produce
    // bit-identical regions.
    for k in 0..n / 2 {
        let (a, b) = (&per_child[2 * k], &per_child[2 * k + 1]);
        for (i, name) in REGION_NAMES.iter().enumerate() {
            assert_eq!(
                a[i], b[i],
                "pair {k}: {name} must be bit-identical for same-seed children"
            );
        }
    }
    // Different seeds must steer wram apart (no canned trajectory).
    let mut wram_by_seed: BTreeMap<u32, &String> = BTreeMap::new();
    for (i, hashes) in per_child.iter().enumerate() {
        let seed = (i / 2) as u32;
        if let Some(prev) = wram_by_seed.insert(seed, &hashes[0]) {
            assert_eq!(prev, &hashes[0]); // same seed, checked above
        }
    }
    let distinct: std::collections::BTreeSet<&&String> = wram_by_seed.values().collect();
    assert_eq!(
        distinct.len(),
        wram_by_seed.len(),
        "every distinct input seed must produce distinct wram contents"
    );

    // Fork-of-fork: snapshot the last child, restore a grandchild, prove
    // fidelity + progress + a clean reverify one level deeper.
    let mut last_child = last_child.expect("n >= 1");
    let post_run = read_all_regions(&last_child);
    let grand_snap = last_child.snapshot().expect("grandchild snapshot");
    drop(last_child);
    let mut grandchild = VmHarness::from_snapshot(&m4_config(), &grand_snap).expect("grandchild");
    let fresh = read_all_regions(&grandchild);
    for (i, name) in REGION_NAMES.iter().enumerate() {
        assert_eq!(
            post_run[i], fresh[i],
            "grandchild: {name} must match the forked child's state"
        );
    }
    let frames_before = grandchild.observed.frame_counter_writes.len();
    let reason = grandchild
        .run_until(Duration::from_secs(60), |o| {
            o.frame_counter_writes.len() >= frames_before + GRANDCHILD_FRAMES
        })
        .expect("grandchild run");
    assert_eq!(reason, StopReason::Predicate, "grandchild must advance");
    let before_updates = region_update_count(&grandchild);
    grandchild.push_command(&detguest_wire::Command::ReverifyRegions);
    let reason = grandchild
        .run_until(Duration::from_secs(30), |o| {
            o.events
                .iter()
                .filter(|e| matches!(e.payload, OwnedPayload::RegionUpdate(_)))
                .count()
                >= before_updates + LIVE_REGIONS
        })
        .expect("grandchild reverify");
    assert_eq!(reason, StopReason::Predicate, "grandchild reverify echoes");
    assert!(
        p0_agent_loglines(&grandchild).is_empty(),
        "grandchild reverify must not alarm"
    );

    // Durable evidence (same discipline as the hypervisor's M9 acceptance).
    write_evidence(&baseline, &per_child, n, started.elapsed());
}

#[test]
#[ignore = "10-minute KVM tier: Intel runner only (DETGUEST_VM_TESTS=1)"]
fn ten_minute_write_churn_keeps_every_pinned_extent_stable() {
    if !gated() {
        return;
    }
    let duration = churn_duration();
    assert!(!duration.is_zero(), "churn duration must be positive");
    let cfg = m4_config();
    let mut vm = VmHarness::new(&cfg).expect("churn harness");
    let reason = vm
        .run_until(Duration::from_secs(120), |o| {
            o.frame_counter_writes
                .last()
                .is_some_and(|&v| v >= WARMUP_FRAME)
        })
        .expect("churn warm-up");
    assert_eq!(
        reason,
        StopReason::Predicate,
        "serial:\n{}",
        vm.serial_text()
    );

    let before = vm
        .channel
        .as_ref()
        .expect("channel attached")
        .read_manifest()
        .expect("baseline manifest");
    let live_before: BTreeMap<u32, _> = before
        .entries
        .iter()
        .take(before.header.region_count as usize)
        .filter(|entry| entry.is_live())
        .map(|entry| {
            let name = std::str::from_utf8(entry.name_bytes()).expect("region name utf8");
            let region = before.resolve(name).expect("resolve live entry");
            (region.region_id, region)
        })
        .collect();
    assert!(live_before.len() >= 4, "stats + wram + framebuffer + meta");
    let initial_wram = read_region_bytes(&vm, "wram");
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        let slice = (deadline - Instant::now()).min(Duration::from_secs(2));
        let reason = vm.run_until(slice, |_| false).expect("write churn slice");
        assert_eq!(reason, StopReason::Timeout);
    }
    assert_ne!(
        read_region_bytes(&vm, "wram"),
        initial_wram,
        "churn wrote wram"
    );

    let before_updates = region_update_count(&vm);
    vm.push_command(&detguest_wire::Command::ReverifyRegions);
    let expected = live_before.len();
    let reason = vm
        .run_until(Duration::from_secs(30), |o| {
            o.events
                .iter()
                .filter(|e| matches!(e.payload, OwnedPayload::RegionUpdate(_)))
                .count()
                >= before_updates + expected
        })
        .expect("post-churn reverify");
    assert_eq!(
        reason,
        StopReason::Predicate,
        "serial:\n{}",
        vm.serial_text()
    );
    assert!(
        p0_agent_loglines(&vm).is_empty(),
        "no moved/dead extent alarms"
    );

    let updates: BTreeMap<u32, _> = vm
        .observed
        .events
        .iter()
        .filter_map(|event| match event.payload {
            OwnedPayload::RegionUpdate(region) => Some((region.region_id, region)),
            _ => None,
        })
        .collect();
    assert_eq!(updates.len(), expected, "one echo per live region");
    for (id, baseline) in &live_before {
        let update = updates.get(id).expect("region update by id");
        assert_eq!(update.layout_version, baseline.layout_version);
        assert_eq!(
            u64::from(update.manifest_generation),
            before.header.generation
        );
    }
    let after = vm
        .channel
        .as_ref()
        .unwrap()
        .read_manifest()
        .expect("post-churn manifest");
    assert_eq!(
        after.header.generation, before.header.generation,
        "no manifest rewrite"
    );
    let live_after: BTreeMap<u32, _> = after
        .entries
        .iter()
        .take(after.header.region_count as usize)
        .filter(|entry| entry.is_live())
        .map(|entry| {
            let name = std::str::from_utf8(entry.name_bytes()).expect("region name utf8");
            let region = after.resolve(name).expect("resolve live entry");
            (region.region_id, region)
        })
        .collect();
    assert_eq!(live_after, live_before, "all extents remain bit-identical");
    eprintln!("[m4-churn] stable for {} seconds", duration.as_secs());
}

fn write_evidence(
    baseline: &[Vec<u8>; LIVE_REGIONS],
    per_child: &[[String; LIVE_REGIONS]],
    n: usize,
    elapsed: Duration,
) {
    let stamp = stdout_of("date", &["-u", "+%Y%m%dT%H%M%SZ"]);
    let root_dir = repo_root().join(format!("target/m4-acceptance-{stamp}"));
    let dumps = root_dir.join("root-regions");
    std::fs::create_dir_all(&dumps).unwrap();

    let mut baseline_hashes = Vec::new();
    for (i, name) in REGION_NAMES.iter().enumerate() {
        std::fs::write(dumps.join(format!("{name}.bin")), &baseline[i]).unwrap();
        baseline_hashes.push(format!(
            "    {{\"region\": \"{name}\", \"bytes\": {}, \"sha256\": \"{}\"}}",
            baseline[i].len(),
            sha256(&baseline[i])
        ));
    }
    let child_rows: Vec<String> = per_child
        .iter()
        .enumerate()
        .map(|(i, h)| {
            format!(
                "    {{\"child\": {i}, \"seed\": {}, \"wram\": \"{}\", \"framebuffer\": \"{}\", \"meta\": \"{}\"}}",
                i / 2,
                h[0],
                h[1],
                h[2]
            )
        })
        .collect();
    let evidence = format!(
        "{{\n  \"acceptance\": \"guest-sdk-m4-platform-readability-vm\",\n  \
         \"phase_gate\": \"emulator RAM region readable from the host and stable across 100x snapshot/restore\",\n  \
         \"utc\": \"{stamp}\",\n  \"git_rev\": \"{rev}\",\n  \"host\": \"{host}\",\n  \
         \"kernel\": \"{kernel}\",\n  \"children\": {n},\n  \"warmup_frame\": {WARMUP_FRAME},\n  \
         \"child_frames\": {CHILD_FRAMES},\n  \"wall_seconds\": {secs},\n  \
         \"assertions\": [\n    \"restore fidelity per child (bit-exact vs root baseline)\",\n    \
         \"meta frame counter and FNV-1a input-history hash match host recomputation\",\n    \
         \"determinism pairs 2k/2k+1 bit-identical; distinct seeds produce distinct wram\",\n    \
         \"ReverifyRegions echoes per region, zero P0 alarms (per child + grandchild)\",\n    \
         \"fork-of-fork restore fidelity and progress\"\n  ],\n  \
         \"root_baseline\": [\n{base}\n  ],\n  \"per_child_sha256\": [\n{children}\n  ]\n}}\n",
        rev = stdout_of("git", &["rev-parse", "HEAD"]),
        host = stdout_of("hostname", &[]),
        kernel = stdout_of("uname", &["-r"]),
        secs = elapsed.as_secs(),
        base = baseline_hashes.join(",\n"),
        children = child_rows.join(",\n"),
    );
    std::fs::write(root_dir.join("evidence.json"), evidence).unwrap();
    eprintln!(
        "[m4-acceptance] evidence artifact root: {}",
        root_dir.display()
    );
}
