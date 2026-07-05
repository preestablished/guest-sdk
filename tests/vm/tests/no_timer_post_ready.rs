//! No-timer post-Ready frame-boundary guards (plan
//! `phase3-post-ready-no-frame-under-no-tick` package 02).
//!
//! These tests target the post-Ready frame signal itself: ring-W
//! `FrameMark` followed by the pv-pad `FRAME_COUNTER` MMIO write. The cheap
//! fixture is `m4-regions`, not `m9_refwork_contract`: current M9 performs
//! pv-blk write/flush on frame 0 while the harness pv-blk model is
//! intentionally read-only, which would test the fixture rather than the
//! no-timer frame path.

use std::path::{Path, PathBuf};
use std::process::Command as Proc;
use std::sync::OnceLock;
use std::time::Duration;

use detguest_host::OwnedPayload;
use detguest_vmtest::harness::snapshot::VmSnapshot;
use detguest_vmtest::harness::{StopReason, VmConfig, VmHarness};

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
    initramfs_m4: PathBuf,
}

static ARTIFACTS: OnceLock<Artifacts> = OnceLock::new();

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
        let dir = build.join("m4-stage-no-timer-post-ready");
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
        let out_path = build.join("initramfs-m4-no-timer-post-ready.cpio");
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

fn no_timer_m4_config() -> VmConfig {
    let a = artifacts();
    let mut cfg = VmConfig::new(a.bzimage.clone(), a.initramfs_m4.clone());
    cfg.timer_interrupts = false;
    cfg.cmdline = cfg.timerless_cmdline();
    cfg
}

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

fn meta_frame(meta: &[u8]) -> u32 {
    u32::from_le_bytes(meta[0..4].try_into().unwrap())
}

fn p0_agent_faults(vm: &VmHarness) -> Vec<String> {
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

fn workload_death(vm: &VmHarness) -> Option<String> {
    vm.observed.events.iter().find_map(|e| match &e.payload {
        OwnedPayload::WorkloadExited {
            guest_pid,
            exit_code,
            term_signal,
        } => Some(format!(
            "WorkloadExited {{ guest_pid: {guest_pid}, exit_code: {exit_code}, \
             term_signal: {term_signal} }}"
        )),
        _ => None,
    })
}

fn boot_no_timer_to_ready() -> VmHarness {
    let cfg = no_timer_m4_config();
    let mut vm = VmHarness::new(&cfg).expect("harness build");
    let stop = vm
        .run_until(Duration::from_secs(120), |o| {
            o.events
                .iter()
                .any(|e| matches!(e.payload, OwnedPayload::Ready { .. }))
        })
        .expect("run to Ready");
    vm.drain();
    assert_eq!(
        stop,
        StopReason::Predicate,
        "no-timer m4 fixture must reach Ready; serial:\n{}",
        vm.serial_text()
    );
    let (region_count, generation) = vm
        .observed
        .events
        .iter()
        .find_map(|e| match e.payload {
            OwnedPayload::Ready {
                region_count,
                manifest_generation,
                ..
            } => Some((region_count, manifest_generation)),
            _ => None,
        })
        .expect("Ready payload");
    assert_eq!(region_count, 3, "wram + framebuffer + meta");
    assert_eq!(
        generation, 6,
        "three registrations, two manifest writes each"
    );
    vm
}

fn assert_no_death_or_fault(vm: &VmHarness) {
    if let Some(death) = workload_death(vm) {
        panic!("workload died: {death}");
    }
    let faults = p0_agent_faults(vm);
    assert!(faults.is_empty(), "no P0 agent faults: {faults:?}");
}

fn run_one_frame(vm: &mut VmHarness) -> (usize, u32) {
    let event_base = vm.observed.events.len();
    let write_base = vm.observed.frame_counter_writes.len();
    let counter_base = vm.pvpad().frame_counter;
    let stop = vm
        .run_until(Duration::from_secs(30), |o| {
            o.frame_counter_writes.len() > write_base
        })
        .expect("run one frame");
    vm.drain();
    assert_eq!(
        stop,
        StopReason::Predicate,
        "no-timer post-Ready run must reach the next FRAME_COUNTER write; serial:\n{}",
        vm.serial_text()
    );
    assert_no_death_or_fault(vm);
    let frame = *vm
        .observed
        .frame_counter_writes
        .last()
        .expect("predicate observed a frame write");
    assert_eq!(
        frame,
        counter_base.wrapping_add(1),
        "run_until should stop at the first post-baseline frame boundary"
    );
    assert!(
        vm.observed.events[event_base..].iter().any(|e| {
            matches!(
                e.payload,
                OwnedPayload::FrameMark { frame_index } if frame_index == frame
            )
        }),
        "drain at FRAME_COUNTER exit must expose matching FrameMark({frame})"
    );
    (event_base, frame)
}

fn assert_regions_advanced(before: &[Vec<u8>; 3], after: &[Vec<u8>; 3], frame: u32) {
    assert_eq!(
        after[1].len(),
        FRAMEBUFFER_LEN,
        "framebuffer must be exactly {FRAMEBUFFER_LEN} bytes"
    );
    assert_eq!(
        meta_frame(&after[2]),
        frame.wrapping_sub(1),
        "m4 meta stores the completed frame before frame_mark writes FRAME_COUNTER"
    );
    assert!(
        before[0] != after[0] || before[1] != after[1] || before[2] != after[2],
        "at least one live region must mutate after a post-Ready frame"
    );
}

#[test]
fn no_timer_live_boot_produces_post_ready_frame() {
    if !gated() {
        return;
    }
    let mut vm = boot_no_timer_to_ready();
    let before = read_all_regions(&vm);
    let (_event_base, frame) = run_one_frame(&mut vm);
    let after = read_all_regions(&vm);
    assert_regions_advanced(&before, &after, frame);
}

#[test]
fn no_timer_ready_snapshot_restore_produces_next_frame() {
    if !gated() {
        return;
    }
    let mut root = boot_no_timer_to_ready();
    let snap: VmSnapshot = root.snapshot().expect("Ready snapshot");
    drop(root);

    let cfg = no_timer_m4_config();
    let mut child = VmHarness::from_snapshot(&cfg, &snap).expect("child from Ready snapshot");
    let before = read_all_regions(&child);
    let (_event_base, frame) = run_one_frame(&mut child);
    let after = read_all_regions(&child);
    assert_regions_advanced(&before, &after, frame);
}
