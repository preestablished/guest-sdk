//! No-timer boot reproducer, tier 1 (plan
//! `phase3-boot-scheduling-deadlock` package 01; request
//! `.agents/requests/phase3-boot-scheduling-deadlock/`):
//!
//! The deterministic worker's guest has **no armed interrupt source** — the
//! irqchip machinery exists but nothing delivers, so `jiffies` never advance
//! and scheduling is cooperative-only. The first real boot wedged silently
//! at the post-registration `GameLoaded` handshake and burned to the 10 B
//! icount HARD_CAP. This test boots the full production shape (pv-blk
//! materialize → control leg → region gate → Ready) with timer-interrupt
//! delivery suppressed (GSI routing minus GSIs 0/2; irqchip + PIT kept so
//! TSC calibration cannot hang) plus the worker's timerless cmdline flags,
//! and requires the boot to reach and HOLD `Ready`.
//!
//! Red against the sched_yield-spinning agent, green with the
//! epoll-blocking boot waits (Fix A) — see package 03's guard-reversion
//! record in this header once verified.
//!
//! Gated on `DETGUEST_VM_TESTS=1` like the other VM suites.

use std::path::{Path, PathBuf};
use std::process::Command as Proc;
use std::sync::OnceLock;
use std::time::Duration;

use detguest_host::OwnedPayload;
use detguest_vmtest::harness::{StopReason, VmConfig, VmHarness};

/// The shared 32 KiB test pattern (contract with `game-load-check`).
const GAME_LEN: usize = 32 * 1024;

fn pattern() -> Vec<u8> {
    (0..GAME_LEN).map(|i| ((i * 7) ^ (i >> 8)) as u8).collect()
}

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
    initramfs: PathBuf,
}

static ARTIFACTS: OnceLock<Artifacts> = OnceLock::new();

/// Same staging recipe as `game_materialization.rs` (agent +
/// `game-load-check` + `boot.toml.game-mat`), distinct stage dir + output
/// name so the suites cannot clobber each other's cpio.
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
        let dir = build.join("no-timer-stage");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sbin")).unwrap();
        std::fs::create_dir_all(dir.join("opt")).unwrap();
        std::fs::create_dir_all(dir.join("etc/detguest")).unwrap();
        std::fs::copy(musl.join("detguest-agent"), dir.join("sbin/detguest-agent")).unwrap();
        std::fs::copy(
            musl.join("game-load-check"),
            dir.join("opt/game-load-check"),
        )
        .unwrap();
        std::fs::copy(
            root.join("image/boot.toml.game-mat"),
            dir.join("etc/detguest/boot.toml"),
        )
        .unwrap();
        run(
            &root,
            "./image/build.sh",
            &["initramfs", dir.to_str().unwrap()],
        );
        let out_path = build.join("initramfs-no-timer.cpio");
        std::fs::rename(build.join("initramfs.cpio"), &out_path).unwrap();
        Artifacts {
            bzimage: build.join("bzImage"),
            initramfs: out_path,
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

fn workload_exited(vm: &VmHarness) -> Option<String> {
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

#[test]
fn no_timer_boot_reaches_and_holds_ready() {
    if !gated() {
        return;
    }
    let a = artifacts();
    let mut cfg = VmConfig::new(a.bzimage.clone(), a.initramfs.clone());
    cfg.timer_interrupts = false;
    cfg.cmdline = cfg.timerless_cmdline();

    let mut vm = VmHarness::new(&cfg).expect("harness boots");
    vm.attach_pv_blk(pattern());

    // Phase 1: the full production boot shape must reach Ready with no
    // interrupt delivery — the agent's own I/O readiness is the only wake.
    let stop = vm
        .run_until(Duration::from_secs(60), |o| {
            o.events
                .iter()
                .any(|e| matches!(e.payload, OwnedPayload::Ready { .. }))
        })
        .expect("run loop");
    vm.drain();
    assert_eq!(
        stop,
        StopReason::Predicate,
        "no-timer boot must reach Ready (the worker-environment wedge); serial:\n{}",
        vm.serial_text()
    );

    // Phase 2: Ready must HOLD — workload alive for 3 more seconds. Only
    // absence-of-death is asserted (no meta-frame check: game-load-check's
    // post-Ready behavior is not the refwork frame loop, and frame pacing
    // may itself be tick-dependent — out of this request's scope).
    let _ = vm
        .run_until(Duration::from_secs(3), |o| {
            o.events
                .iter()
                .any(|e| matches!(e.payload, OwnedPayload::WorkloadExited { .. }))
        })
        .expect("run loop");
    vm.drain();
    if let Some(death) = workload_exited(&vm) {
        panic!("workload died after Ready: {death}");
    }
    let faults = p0_agent_faults(&vm);
    assert!(faults.is_empty(), "no P0 agent fault after Ready: {faults:?}");
}
