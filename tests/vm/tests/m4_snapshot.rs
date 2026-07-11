//! Ms4 snapshot/restore validation tests (plan package 05).
//!
//! These de-risk the 100x readability acceptance: restore fidelity is
//! debugged here, not inside the big loop. Gating is identical to
//! `m2_acceptance.rs` (`#[ignore]` + `DETGUEST_VM_TESTS=1` + `/dev/kvm`).
//!
//! Frame accounting: children start with a fresh `Observed`, so child
//! predicates count deltas since child start (`frame_counter_writes.len()`),
//! never absolute totals from the root's history.

use std::path::{Path, PathBuf};
use std::process::Command as Proc;
use std::sync::OnceLock;
use std::time::Duration;

use detguest_host::SinkOp;
use detguest_vmtest::harness::snapshot::VmSnapshot;
use detguest_vmtest::harness::{Observed, StopReason, VmConfig, VmHarness};
use detguest_wire::events::{decode_command, decode_workload_ctrl};
use detguest_wire::{Command, RingId, WorkloadCtrl};

/// Warm-up boundary: the root runs to this FRAME_COUNTER value before the
/// snapshot is taken.
const WARMUP_FRAME: u32 = 8;
/// Frames each child runs after restore.
const CHILD_FRAMES: usize = 10;
/// D7 layout_version 1 framebuffer contract.
const FRAMEBUFFER_LEN: usize = 229_376;

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

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

struct Artifacts {
    bzimage: PathBuf,
    /// Initramfs with the m4-regions boot.toml (autostart + 3 expected
    /// regions gating Ready).
    initramfs_m4: PathBuf,
}

static ARTIFACTS: OnceLock<Artifacts> = OnceLock::new();

/// Build everything once: musl agent+workloads, the pinned kernel (cached),
/// and the m4 initramfs variant (same recipe as m2's `artifacts()`).
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
        let dir = build.join("m4-stage-regions");
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
        let out_path = build.join("initramfs-m4-regions.cpio");
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

fn m4_config() -> VmConfig {
    let a = artifacts();
    VmConfig::new(a.bzimage.clone(), a.initramfs_m4.clone())
}

/// Boot the m4 fixture and run until the guest has written FRAME_COUNTER
/// `WARMUP_FRAME` (Ready is gated on the three regions being live, and the
/// frame loop only starts after publication, so the manifest is populated).
fn boot_to_warmup() -> VmHarness {
    let cfg = m4_config();
    let mut vm = VmHarness::new(&cfg).expect("harness build");
    let reason = vm
        .run_until(Duration::from_secs(120), |o| {
            o.frame_counter_writes
                .last()
                .is_some_and(|&v| v >= WARMUP_FRAME)
        })
        .expect("run to warm-up");
    assert_eq!(
        reason,
        StopReason::Predicate,
        "expected frame {WARMUP_FRAME} before stop/timeout; serial:\n{}",
        vm.serial_text()
    );
    assert!(vm.channel.is_some(), "channel attached");
    vm
}

/// Host-side region read: manifest resolve + extent walk over guest memory.
/// Never sends a command or runs the vCPU (the point of the milestone).
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

fn read_all_regions(vm: &VmHarness) -> [Vec<u8>; 3] {
    [
        read_region_bytes(vm, "wram"),
        read_region_bytes(vm, "framebuffer"),
        read_region_bytes(vm, "meta"),
    ]
}

/// Run a child for `frames` more FRAME_COUNTER writes (delta since child
/// start — children have a fresh `Observed`).
fn run_child_frames(child: &mut VmHarness, frames: usize) {
    let reason = child
        .run_until(Duration::from_secs(60), |o: &Observed| {
            o.frame_counter_writes.len() >= frames
        })
        .expect("child run");
    assert_eq!(
        reason,
        StopReason::Predicate,
        "child must advance {frames} frames; serial:\n{}",
        child.serial_text()
    );
}

/// Build a child, schedule `frames` of seed-derived inputs on pad 0, and run
/// it. Scheduled frames start at the first frame whose FRAME_COUNTER write
/// happens inside the child (snapshot frame + 1) so the schedule can latch.
fn run_scheduled_child(snap: &VmSnapshot, seed: u32, frames: usize) -> VmHarness {
    let cfg = m4_config();
    let mut child = VmHarness::from_snapshot(&cfg, snap).expect("child build");
    let base = child.pvpad().frame_counter;
    for i in 0..frames as u32 {
        let frame = base + 1 + i;
        // Deterministic per-(seed, frame) value; any fixed mix works.
        let value = seed
            .wrapping_mul(0x9e37_79b9)
            .wrapping_add(frame.wrapping_mul(0x85eb_ca6b));
        child.pvpad().schedule(frame, 0, value);
    }
    run_child_frames(&mut child, frames);
    child
}

/// Plan 05 validation test 1: snapshot -> restore -> the guest still runs.
#[test]
#[ignore = "KVM tier: Intel runner only (DETGUEST_VM_TESTS=1)"]
fn snapshot_restore_guest_still_runs() {
    if !gated() {
        return;
    }
    let mut root = boot_to_warmup();
    let root_last = *root.observed.frame_counter_writes.last().unwrap();
    // The root folded NameIntern events while draining; remember one so the
    // child's re-seeded intern map can be checked below.
    let root_interns = root.channel.as_ref().unwrap().interns();
    assert!(
        !root_interns.is_empty(),
        "m4 fixture must have interned at least one name before warm-up"
    );
    let snap = root.snapshot().expect("snapshot");
    drop(root);

    let cfg = m4_config();
    let mut child = VmHarness::from_snapshot(&cfg, &snap).expect("child build");
    // The child never drained an event: intern_name resolving here proves
    // from_snapshot re-seeded the channel's intern map directly (guest-sdk-4bc),
    // not via manifest name bytes.
    let probe = &root_interns[0];
    assert_eq!(
        child.channel.as_ref().unwrap().intern_name(probe.name_id),
        Some(probe.name.as_str()),
        "child channel must resolve intern {} without any drain",
        probe.name_id
    );
    run_child_frames(&mut child, CHILD_FRAMES);

    // The frame counter continues from the snapshot point (no reboot, no
    // reset — write values pick up exactly where the root stopped).
    assert_eq!(
        child.observed.frame_counter_writes[0],
        root_last + 1,
        "child frame counter must continue from the snapshot point"
    );
    let serial = child.serial_text();
    for bad in ["panic", "Oops", "BUG:"] {
        assert!(
            !serial.contains(bad),
            "guest {bad} after restore; serial:\n{serial}"
        );
    }
}

/// Host-produced C/I sequences live outside guest RAM. Restored branches
/// must continue both streams without a reset, duplicate, or gap, and must
/// emit byte-identical mutation traces from the same root checkpoint.
#[test]
#[ignore = "KVM tier: Intel runner only (DETGUEST_VM_TESTS=1)"]
fn restored_branches_continue_channel_sequences_without_duplicates() {
    if !gated() {
        return;
    }

    let mut root = boot_to_warmup();
    let command = Command::SetLogMask { mask: 0x5a5a_1234 };
    let control = WorkloadCtrl::QuiesceReq { token: 0x1020_3040 };
    root.channel
        .as_mut()
        .unwrap()
        .push_command(&command, &mut root.sink)
        .expect("root ring-C push");
    root.channel
        .as_mut()
        .unwrap()
        .push_workload_ctrl(&control, &mut root.sink)
        .expect("root ring-I push");
    let checkpoint = root.channel.as_ref().unwrap().producer_seqs();
    assert!(checkpoint.ring_c > 0 && checkpoint.ring_i > 0);
    let mut snap = root.snapshot().expect("channel checkpoint snapshot");
    drop(root);

    let push_after_restore = |snapshot: &VmSnapshot| {
        let mut child = VmHarness::from_snapshot(&m4_config(), snapshot).expect("child restore");
        child
            .channel
            .as_mut()
            .unwrap()
            .push_command(&command, &mut child.sink)
            .expect("child ring-C push");
        child
            .channel
            .as_mut()
            .unwrap()
            .push_workload_ctrl(&control, &mut child.sink)
            .expect("child ring-I push");
        child.sink.ops
    };

    let trace_a = push_after_restore(&snap);
    let trace_b = push_after_restore(&snap);
    assert_eq!(trace_a, trace_b, "restored child mutation traces differ");
    assert_eq!(
        trace_a.len(),
        2,
        "later records must be emitted exactly once"
    );

    let c_bytes = match &trace_a[0] {
        SinkOp::RingPush {
            ring: RingId::C,
            bytes,
            ..
        } => bytes,
        other => panic!("expected ring-C push first, got {other:?}"),
    };
    let (c_header, decoded) = decode_command(c_bytes).expect("decode restored command");
    assert_eq!(decoded, command);
    assert_eq!(
        c_header.seq, checkpoint.ring_c,
        "ring-C restored sequence mismatch"
    );

    let i_bytes = match &trace_a[1] {
        SinkOp::RingPush {
            ring: RingId::I,
            bytes,
            ..
        } => bytes,
        other => panic!("expected ring-I push second, got {other:?}"),
    };
    let (i_header, decoded) = decode_workload_ctrl(i_bytes).expect("decode restored control");
    assert_eq!(decoded, control);
    assert_eq!(
        i_header.seq, checkpoint.ring_i,
        "ring-I restored sequence mismatch"
    );

    snap.corrupt_ring_c_producer_seq_for_test(1);
    let corrupt_trace = push_after_restore(&snap);
    let corrupt_bytes = match &corrupt_trace[0] {
        SinkOp::RingPush {
            ring: RingId::C,
            bytes,
            ..
        } => bytes,
        other => panic!("expected corrupt ring-C push first, got {other:?}"),
    };
    let corrupt_seq = decode_command(corrupt_bytes)
        .expect("decode corrupt command")
        .0
        .seq;
    let mismatch = (corrupt_seq != checkpoint.ring_c)
        .then_some("ring-C restored sequence mismatch")
        .expect("corrupt checkpoint must be detected");
    assert_eq!(mismatch, "ring-C restored sequence mismatch");
}

/// Plan 05 validation test 2: two children from one root with identical
/// scheduled pv-pad inputs produce bit-identical region bytes.
#[test]
#[ignore = "KVM tier: Intel runner only (DETGUEST_VM_TESTS=1)"]
fn snapshot_restore_is_deterministic() {
    if !gated() {
        return;
    }
    let mut root = boot_to_warmup();
    let snap = root.snapshot().expect("snapshot");
    drop(root);

    let child_a = run_scheduled_child(&snap, 7, CHILD_FRAMES);
    let regions_a = read_all_regions(&child_a);
    drop(child_a);
    let child_b = run_scheduled_child(&snap, 7, CHILD_FRAMES);
    let regions_b = read_all_regions(&child_b);
    drop(child_b);

    assert_eq!(
        regions_a[1].len(),
        FRAMEBUFFER_LEN,
        "framebuffer must be exactly {FRAMEBUFFER_LEN} bytes"
    );
    for (i, name) in ["wram", "framebuffer", "meta"].iter().enumerate() {
        assert_eq!(
            regions_a[i], regions_b[i],
            "{name} must be bit-identical between same-input children"
        );
    }

    // Negative control: a child fed different inputs must diverge (guards
    // against a restore path that pins the guest to a canned trajectory).
    let child_c = run_scheduled_child(&snap, 8, CHILD_FRAMES);
    let regions_c = read_all_regions(&child_c);
    assert_ne!(
        regions_a[0], regions_c[0],
        "different input schedules must steer wram differently"
    );
}

/// Plan 05 validation test 3: running a child never mutates the root
/// snapshot — a later child still starts from identical region bytes.
#[test]
#[ignore = "KVM tier: Intel runner only (DETGUEST_VM_TESTS=1)"]
fn root_snapshot_immutability() {
    if !gated() {
        return;
    }
    let mut root = boot_to_warmup();
    let snap = root.snapshot().expect("snapshot");
    // The root's memory is untouched since the snapshot (vCPU stopped), so
    // reading it now yields the root baseline the snapshot captured.
    let baseline = read_all_regions(&root);
    drop(root);

    // Run (and mutate) one child...
    let child_a = run_scheduled_child(&snap, 3, CHILD_FRAMES);
    let mutated = read_all_regions(&child_a);
    assert_ne!(
        baseline[2], mutated[2],
        "meta must advance in a running child (sanity)"
    );
    drop(child_a);

    // ...then a fresh child from the same root starts from the baseline.
    let cfg = m4_config();
    let child_b = VmHarness::from_snapshot(&cfg, &snap).expect("child build");
    let fresh = read_all_regions(&child_b);
    for (i, name) in ["wram", "framebuffer", "meta"].iter().enumerate() {
        assert_eq!(
            baseline[i], fresh[i],
            "{name}: fresh child must start from the root baseline"
        );
    }
}
