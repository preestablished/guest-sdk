//! Game-materialization acceptance (plan `phase3-game-device-materialization`
//! package 04; bead guest-sdk-doq; request
//! `.agents/requests/phase3-game-device-materialization/`):
//!
//! > a test where the agent materializes a known game image via pv-blk and
//! > the unit reads it back byte-exact (checksummed), plus a loud distinct
//! > fault when pv-blk is absent/corrupt.
//!
//! One image (agent + `game-load-check` + the `boot.toml.game-mat` fixture
//! with `game_source = "pv-blk"`) boots three ways:
//!
//! 1. **Positive**: harness pv-blk backed by the 32 KiB shared pattern →
//!    boot reaches `Ready` through the production shape (materialize →
//!    control leg → region gate), the workload's `meta` region carries the
//!    byte-exact checksum, and the stdout LogLine names the same numbers.
//! 2. **Device absent** (the `boot_probe` situation): no pv-blk attached →
//!    the agent's `pv-blk:`-named boot fault, no `Ready`, no unit spawned —
//!    and NOT the harness's cannot-read-path fault this request replaced.
//! 3. **Corrupt/truncated**: backing truncated to a non-512 multiple → the
//!    tail silently truncates at sector granularity (undetectable in-guest
//!    by design), the workload's embedded expectation catches it, and the
//!    boot fault carries the workload's Fault detail. No `Ready`.
//!
//! Guard-reverted check (ecosystem convention): with the workload's
//! checksum comparison skipped, case 3's "no Ready" + fault-text assertions
//! fail; with the agent's verify pass skipped, the `pvblk` unit tier's
//! drift test fails first. Recorded here so the negatives stay honest.

use std::path::{Path, PathBuf};
use std::process::Command as Proc;
use std::sync::OnceLock;
use std::time::Duration;

use detguest_host::OwnedPayload;
use detguest_vmtest::harness::{StopReason, VmConfig, VmHarness};

/// The shared 32 KiB test pattern (the contract with `game-load-check`,
/// which regenerates it in-guest and Faults on any divergence).
const GAME_LEN: usize = 32 * 1024;

fn pattern() -> Vec<u8> {
    (0..GAME_LEN).map(|i| ((i * 7) ^ (i >> 8)) as u8).collect()
}

/// `detguest-agent` `pvblk::checksum_fold` reimplemented (the crates don't
/// link); [`GAME_MAT_PATTERN_CHECKSUM`] pins both against drift.
const CHECKSUM_SEED: u64 = 0x7062_6c6b_5f69_6f31;

fn checksum(bytes: &[u8]) -> u64 {
    let mut sum = CHECKSUM_SEED;
    for (i, byte) in bytes.iter().enumerate() {
        sum = sum.rotate_left(5) ^ u64::from(*byte).wrapping_add(i as u64);
    }
    sum
}

/// Same golden as `detguest-agent::pvblk::tests::checksum_matches_pinned_golden`.
const GAME_MAT_PATTERN_CHECKSUM: u64 = 0x59ac_17a5_2dff_da9c;

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

/// Same staging recipe as `m4_acceptance.rs`.
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
        let dir = build.join("game-mat-stage");
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
        let out_path = build.join("initramfs-game-mat.cpio");
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

fn config() -> VmConfig {
    let a = artifacts();
    VmConfig::new(a.bzimage.clone(), a.initramfs.clone())
}

fn has_ready(vm: &VmHarness) -> bool {
    vm.observed
        .events
        .iter()
        .any(|e| matches!(e.payload, OwnedPayload::Ready { .. }))
}

fn workload_started(vm: &VmHarness) -> bool {
    vm.observed
        .events
        .iter()
        .any(|e| matches!(e.payload, OwnedPayload::WorkloadStarted { .. }))
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

fn all_loglines(vm: &VmHarness) -> Vec<String> {
    vm.observed
        .events
        .iter()
        .filter_map(|e| match &e.payload {
            OwnedPayload::LogLine { msg, .. } => Some(String::from_utf8_lossy(msg).into_owned()),
            _ => None,
        })
        .collect()
}

/// Host-side region read through the real manifest path (never runs the
/// vCPU) — same discipline as m4_acceptance.
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

#[test]
fn materialized_game_reaches_ready_byte_exact() {
    if !gated() {
        return;
    }
    let mut vm = VmHarness::new(&config()).expect("harness boots");
    vm.attach_pv_blk(pattern());
    let stop = vm
        .run_until(Duration::from_secs(120), |o| {
            o.events
                .iter()
                .any(|e| matches!(e.payload, OwnedPayload::Ready { .. }))
        })
        .expect("run loop");
    assert_eq!(
        stop,
        StopReason::Predicate,
        "must reach Ready; serial:\n{}",
        vm.serial_text()
    );
    assert!(
        p0_agent_loglines(&vm).is_empty(),
        "no P0 agent fault before Ready: {:?}",
        p0_agent_loglines(&vm)
    );

    // Byte-exact evidence through the real host read path: the workload
    // published its computed (checksum, len) in `meta`.
    let meta = read_region_bytes(&vm, "meta");
    let guest_sum = u64::from_le_bytes(meta[0..8].try_into().unwrap());
    let guest_len = u64::from_le_bytes(meta[8..16].try_into().unwrap());
    assert_eq!(guest_len, GAME_LEN as u64, "size-discovery off-by-one");
    assert_eq!(checksum(&pattern()), GAME_MAT_PATTERN_CHECKSUM);
    assert_eq!(guest_sum, GAME_MAT_PATTERN_CHECKSUM, "byte-exact readback");
    // Negative control (a materializer writing the right length from a
    // phantom/zeroed device could not pass this): the pattern checksum is
    // not the all-zeros checksum.
    assert_ne!(
        GAME_MAT_PATTERN_CHECKSUM,
        checksum(&vec![0u8; GAME_LEN]),
        "pattern must be distinguishable from zeros"
    );

    // The workload's stdout line is drained by the supervise loop after
    // Ready; LogLines carry no doorbell, so give the guest a beat and
    // drain manually.
    let mut saw_line = false;
    for _ in 0..20 {
        let _ = vm.run_until(Duration::from_millis(500), |_| false);
        vm.drain();
        if all_loglines(&vm)
            .iter()
            .any(|l| l.contains("game bytes=32768") && l.contains("checksum=0x59ac17a52dffda9c"))
        {
            saw_line = true;
            break;
        }
    }
    assert!(
        saw_line,
        "workload stdout LogLine with bytes+checksum expected; loglines: {:?}",
        all_loglines(&vm)
    );
}

#[test]
fn absent_device_is_a_loud_pv_blk_fault_not_a_path_fault() {
    if !gated() {
        return;
    }
    // The boot_probe situation: same image, no pv-blk attached — the MMIO
    // window reads as zeros, so the agent's bus-MAGIC presence check fails.
    let mut vm = VmHarness::new(&config()).expect("harness boots");
    let stop = vm
        .run_until(Duration::from_secs(120), |_| false)
        .expect("run loop");
    assert_eq!(
        stop,
        StopReason::GuestStopped,
        "agent must power off on the boot fault; serial:\n{}",
        vm.serial_text()
    );
    vm.drain();
    assert!(!has_ready(&vm), "must not reach Ready without the device");
    assert!(
        !workload_started(&vm),
        "materialization faults BEFORE the unit is spawned"
    );
    let faults = p0_agent_loglines(&vm);
    assert!(
        faults
            .iter()
            .any(|l| l.contains("pv-blk") && l.contains("magic")),
        "fault must name pv-blk and the magic mismatch: {faults:?}"
    );
    // Distinct from the fault this request replaced: the harness-side
    // cannot-read-path text must be gone.
    assert!(
        !faults.iter().any(|l| l.contains("cannot read game path")),
        "the old cannot-read-path fault must not reappear: {faults:?}"
    );
}

#[test]
fn truncated_backing_faults_loudly_before_ready() {
    if !gated() {
        return;
    }
    // Non-512-multiple backing: the device only addresses whole sectors, so
    // the materialized file is 32256 bytes (63 sectors) — invisible to the
    // agent by ABI construction; the workload's embedded expectation is
    // what catches it (a workload that skipped its self-check would reach
    // Ready here — the guard-reverted failure mode).
    let mut truncated = pattern();
    truncated.truncate(GAME_LEN - 100);
    let mut vm = VmHarness::new(&config()).expect("harness boots");
    vm.attach_pv_blk(truncated);
    let stop = vm
        .run_until(Duration::from_secs(120), |_| false)
        .expect("run loop");
    assert_eq!(
        stop,
        StopReason::GuestStopped,
        "agent must power off on the workload Fault; serial:\n{}",
        vm.serial_text()
    );
    vm.drain();
    assert!(!has_ready(&vm), "must not reach Ready on a truncated image");
    let faults = p0_agent_loglines(&vm);
    assert!(
        faults
            .iter()
            .any(|l| l.contains("refwork fault after LoadGame") && l.contains("32256")),
        "fault must carry the workload's truncation detail: {faults:?}"
    );
}
