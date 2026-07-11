//! Ms5 live `inject_point` record/replay acceptance.
//!
//! The workload is a real Linux process launched by detguest-agent. Every
//! INJECT exit drains its matching ring-W query before answering, and the
//! workload echoes the returned decision through the versioned LogLine
//! contract in `testload --inject-roundtrip`.

use std::path::{Path, PathBuf};
use std::process::Command as Proc;
use std::time::Duration;

use detguest_host::{
    FaultRule, InjectResponder, LogFaultPlan, LoggedDecision, SinkOp, TableFaultPlan,
};
use detguest_vmtest::harness::{HarnessFaultPlan, StopReason, VmConfig, VmHarness};
use detguest_wire::events::Command;
use detguest_wire::{FaultDecision, RingId};

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

fn run(cwd: &Path, prog: &str, args: &[&str]) {
    let status = Proc::new(prog)
        .args(args)
        .current_dir(cwd)
        .status()
        .unwrap_or_else(|e| panic!("spawn {prog}: {e}"));
    assert!(status.success(), "{prog} {args:?} failed: {status}");
}

fn config() -> VmConfig {
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

    let build = root.join("image/build");
    let musl = root.join("target/x86_64-unknown-linux-musl/release");
    let stage = build.join("m5-inject-roundtrip-stage");
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(stage.join("sbin")).unwrap();
    std::fs::create_dir_all(stage.join("opt")).unwrap();
    std::fs::create_dir_all(stage.join("etc/detguest")).unwrap();
    std::fs::copy(
        musl.join("detguest-agent"),
        stage.join("sbin/detguest-agent"),
    )
    .unwrap();
    std::fs::copy(musl.join("testload"), stage.join("opt/testload")).unwrap();
    std::fs::write(
        stage.join("etc/detguest/boot.toml"),
        "boot_toml_version = 1\n\n[[unit]]\nid = 0\nexec = \"/opt/testload\"\nargs = [\"--inject-roundtrip\"]\n",
    )
    .unwrap();
    run(
        &root,
        "./image/build.sh",
        &["initramfs", stage.to_str().unwrap()],
    );
    let initramfs = build.join("initramfs-m5-inject-roundtrip.cpio");
    std::fs::rename(build.join("initramfs.cpio"), &initramfs).unwrap();
    VmConfig::new(build.join("bzImage"), initramfs)
}

fn boot_ready(cfg: &VmConfig) -> VmHarness {
    let mut vm = VmHarness::new(cfg).expect("harness build");
    let reason = vm
        .run_until(Duration::from_secs(60), |o| {
            o.events.iter().any(|e| {
                matches!(
                    e.payload,
                    detguest_host::OwnedPayload::Ready { unit: u32::MAX, .. }
                )
            })
        })
        .expect("boot to Ready");
    assert_eq!(
        reason,
        StopReason::Predicate,
        "no-autostart Ready missing; serial:\n{}",
        vm.serial_text()
    );
    vm
}

fn run_workload(vm: &mut VmHarness) {
    vm.push_command(&Command::StartWorkload {
        unit: 0,
        log_mask: 0x1f,
    });
    let reason = vm
        .run_until(Duration::from_secs(60), |o| {
            o.events.iter().any(|e| {
                matches!(
                    e.payload,
                    detguest_host::OwnedPayload::WorkloadExited {
                        exit_code: 0,
                        term_signal: 0,
                        ..
                    }
                )
            })
        })
        .expect("run inject workload");
    assert_eq!(
        reason,
        StopReason::Predicate,
        "inject workload did not exit cleanly; serial:\n{}",
        vm.serial_text()
    );
}

fn inject_queries(vm: &VmHarness) -> Vec<(u32, u32)> {
    vm.observed
        .events
        .iter()
        .filter_map(|event| match event.payload {
            detguest_host::OwnedPayload::InjectQuery { iseq, name_id }
                if event.ring == RingId::W =>
            {
                Some((iseq, name_id))
            }
            _ => None,
        })
        .collect()
}

fn decision_logs(vm: &VmHarness) -> Vec<String> {
    vm.observed
        .events
        .iter()
        .filter_map(|event| match &event.payload {
            detguest_host::OwnedPayload::LogLine { msg, .. }
                if msg.starts_with(b"ms5.inject.v1 ") =>
            {
                Some(String::from_utf8_lossy(msg).into_owned())
            }
            _ => None,
        })
        .collect()
}

fn answers(vm: &VmHarness) -> Vec<u32> {
    vm.sink
        .ops
        .iter()
        .filter_map(|op| match op {
            SinkOp::PioAnswer { value, .. } => Some(*value),
            _ => None,
        })
        .collect()
}

#[test]
#[ignore = "KVM tier: Intel runner only (DETGUEST_VM_TESTS=1)"]
fn live_inject_decisions_round_trip_and_replay_at_same_sequence_points() {
    if !gated() {
        return;
    }
    let cfg = config();
    let rules = vec![
        FaultRule {
            name_glob: "ms5.io.read".into(),
            occurrence: None,
            decision: FaultDecision::Platform { kind: 2, arg: 512 },
        },
        FaultRule {
            name_glob: "ms5.io.write".into(),
            occurrence: None,
            decision: FaultDecision::Workload { kind: 64, arg: 7 },
        },
    ];

    let mut record = boot_ready(&cfg);
    record.responder = InjectResponder::new(HarnessFaultPlan::Table(TableFaultPlan::new(rules)));
    run_workload(&mut record);
    let queries = inject_queries(&record);
    let record_answers = answers(&record);
    let record_logs = decision_logs(&record);
    assert_eq!(queries.len(), 6, "one query per canonical point");
    assert_eq!(record_answers.len(), 6, "exactly one answer per query");
    assert_eq!(record_logs.len(), 6, "workload echoed every decision");
    assert!(record_answers.contains(&FaultDecision::Proceed.pack()));
    assert!(record_answers.contains(&FaultDecision::Platform { kind: 2, arg: 512 }.pack()));
    assert!(record_answers.contains(&FaultDecision::Workload { kind: 64, arg: 7 }.pack()));
    assert_eq!(
        record.channel.as_ref().unwrap().pending_injects(),
        Vec::new()
    );
    assert_eq!(record.channel.as_ref().unwrap().unmatched_injects, 0);

    let table = record
        .responder
        .plan_mut()
        .table_mut()
        .expect("record table plan");
    let logged: Vec<_> = table
        .decisions
        .iter()
        .zip(&queries)
        .map(|(&(iseq, decision), &(query_iseq, name_id))| {
            assert_eq!(iseq, query_iseq, "answer/query iseq correlation");
            LoggedDecision {
                iseq,
                name_id,
                decision,
            }
        })
        .collect();

    let mut replay = boot_ready(&cfg);
    replay.responder =
        InjectResponder::new(HarnessFaultPlan::Log(LogFaultPlan::new(logged.clone())));
    run_workload(&mut replay);
    let replay_plan = replay
        .responder
        .plan_mut()
        .log_mut()
        .expect("replay log plan");
    assert!(
        replay_plan.divergences().is_empty(),
        "replay divergences: {:?}",
        replay_plan.divergences()
    );
    assert_eq!(replay_plan.remaining(), 0, "all decoded decisions consumed");
    assert_eq!(inject_queries(&replay), queries, "query sequence changed");
    assert_eq!(answers(&replay), record_answers, "PIO answers changed");
    assert_eq!(
        decision_logs(&replay),
        record_logs,
        "workload returns changed"
    );
    assert_eq!(replay.channel.as_ref().unwrap().unmatched_injects, 0);
}
