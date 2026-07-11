//! Ms5 live `determinism_replay` gate.
//!
//! The authoritative equality surfaces are final guest RAM, complete drained
//! event bytes, drop counters, and workload-echoed inject decisions. S1–S4
//! mutation digests remain diagnostics.
//!
//! Two tiers:
//!
//! - **Ungated self-tests** (plain `#[test]`, no KVM, no env): the fixture
//!   round trip (record with `TableFaultPlan`, replay with `LogFaultPlan`,
//!   digests equal on all four surfaces, zero divergences), four
//!   deliberate-mismatch negatives (one per surface — a comparison that
//!   cannot fail is not evidence), and a seed-variation self-test (guards
//!   against a digest that hashes nothing). The phases track re-runs these
//!   from a clean checkout with `cargo test -p detguest-vmtest`.
//! - **The gated in-VM leg**: house double-gate (`#[ignore]` +
//!   `DETGUEST_VM_TESTS=1`), lane invocation
//!   `DETGUEST_VM_TESTS=1 cargo test -p detguest-vmtest --test
//!   determinism_replay -- --ignored --test-threads=1`. Seeded, chunked,
//!   resumable via explicit `DETGUEST_REPLAY_START_ITER` plus
//!   `DETGUEST_REPLAY_ITER_COUNT`; durable runs also use the evidence
//!   manifest as the only continuation cursor.

use std::path::{Path, PathBuf};
use std::process::Command as Proc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use detguest_host::{
    Channel, DropCounters, FaultRule, GuestEvent, InjectResponder, LogFaultPlan, LoggedDecision,
    MockGuestMem, RecordingSink, SinkOp, TableFaultPlan,
};
use detguest_vmtest::harness::snapshot::VmSnapshot;
use detguest_vmtest::harness::{HarnessFaultPlan, StopReason, VmConfig, VmHarness};
use detguest_vmtest::replay::{
    assert_digests_equal, digest_from_trace, drop_counter_hash, inject_log_hash,
    raw_event_stream_hash, RunDigest,
};
use detguest_wire::events::{encode_event, encoded_event_len, Command, EventPayload};
use detguest_wire::header::{ChannelHeader, CHANNEL_SIZE, OFF_RESERVED};
use detguest_wire::{FaultDecision, RingId};

// ---------------------------------------------------------------------------
// Ungated self-test fixture: a host-only channel driven through a scripted,
// seed-varied mixed workload (commands pushed, guest events incl. inject
// queries and input-burst log lines, rings drained, injects answered).
// ---------------------------------------------------------------------------

const BASE: u64 = 0x1000_0000;

/// Deterministic seed-derived stream (xorshift32; seed 0 is remapped).
struct Rng(u32);

impl Rng {
    fn new(seed: u32) -> Rng {
        Rng(seed.wrapping_mul(0x9e37_79b9).wrapping_add(1))
    }
    fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
}

const POINT_NAMES: [&str; 4] = ["io_read", "io_write", "net_send", "disk_flush"];

/// Build a channel whose rings A/W already carry the scripted guest-produced
/// records (written into guest memory before `attach` — an external crate
/// has, correctly, no mutable access to an attached channel's guest memory).
fn channel_with_guest_events(
    a_events: &[EventPayload],
    w_events: &[EventPayload],
) -> Channel<MockGuestMem> {
    use detguest_host::GuestMem;
    let mut gm = MockGuestMem::with_zeroed(BASE, CHANNEL_SIZE);
    let mut hdr = [0u8; OFF_RESERVED];
    ChannelHeader::canonical().write_to(&mut hdr).unwrap();
    gm.write(BASE, &hdr).unwrap();
    for (ring, events) in [(RingId::A, a_events), (RingId::W, w_events)] {
        let desc = ring.canonical_desc();
        let mut off = 0u32;
        let mut buf = [0u8; 4096];
        for (seq, ev) in events.iter().enumerate() {
            let n = encode_event(&mut buf, seq as u32, 1, 0, ev).unwrap();
            assert_eq!(n, encoded_event_len(ev));
            gm.write(BASE + desc.offset as u64 + off as u64, &buf[..n])
                .unwrap();
            off += n as u32;
        }
        gm.write(BASE + ring.prod_offset() as u64, &off.to_le_bytes())
            .unwrap();
    }
    Channel::attach(gm, BASE).unwrap()
}

/// One leg's raw observables (digest inputs plus the recorded decisions and
/// the replay divergence count).
struct Leg {
    ops: Vec<SinkOp>,
    events: Vec<GuestEvent>,
    drops: DropCounters,
    /// `(iseq, name_id, decision)` in answer order — the record leg's output,
    /// the replay leg's input.
    decisions: Vec<LoggedDecision>,
    divergences: usize,
}

impl Leg {
    fn digest(&self) -> RunDigest {
        digest_from_trace(&self.ops, &self.events, &self.drops)
    }
}

/// Drive the scripted workload once. `replay_from: None` is the record leg
/// (seed-derived `TableFaultPlan`, the synthesizer stand-in); `Some(log)` is
/// the replay leg (`LogFaultPlan` only — synthesizer absent).
fn run_leg(seed: u32, replay_from: Option<Vec<LoggedDecision>>) -> Leg {
    let mut rng = Rng::new(seed);
    let n_names = 2 + (rng.next() % 3) as usize; // 2..=4
    let n_injects = 2 + rng.next() % 3; // 2..=4
    let n_frames = 2 + rng.next() % 4; // 2..=5

    // Guest-produced fixture: interns, seed-varied input-burst log lines
    // per frame, frame marks, inject queries.
    let burst_msgs: Vec<String> = (0..n_frames)
        .map(|f| format!("input:{f}:{:#010x}", rng.next()))
        .collect();
    let inject_pairs: Vec<(u32, u32)> = (1..=n_injects)
        .map(|iseq| (iseq, 1 + rng.next() % n_names as u32))
        .collect();
    let mut w_events: Vec<EventPayload> = POINT_NAMES[..n_names]
        .iter()
        .enumerate()
        .map(|(i, name)| EventPayload::NameIntern {
            name_id: i as u32 + 1,
            name: name.as_bytes(),
        })
        .collect();
    for (f, msg) in burst_msgs.iter().enumerate() {
        w_events.push(EventPayload::LogLine {
            stream: 1,
            level: 2,
            msg: msg.as_bytes(),
        });
        w_events.push(EventPayload::FrameMark {
            frame_index: f as u32 + 1,
        });
    }
    for &(iseq, name_id) in &inject_pairs {
        w_events.push(EventPayload::InjectQuery { iseq, name_id });
    }

    let mut ch = channel_with_guest_events(
        &[EventPayload::Hello {
            proto_version: 1,
            agent_version: 0x100,
            capabilities: 0,
        }],
        &w_events,
    );

    // Host side: seed-varied command pushes, drain, answer the injects.
    let mut sink = RecordingSink::default();
    for _ in 0..(1 + rng.next() % 3) {
        ch.push_command(&Command::SetLogMask { mask: rng.next() }, &mut sink)
            .unwrap();
    }
    let events = ch.drain_events(&mut sink).unwrap();

    let (decisions, divergences) = match replay_from {
        None => {
            // Record leg: seed-derived varied decisions, one rule per name.
            let rules = POINT_NAMES[..n_names]
                .iter()
                .map(|name| FaultRule {
                    name_glob: (*name).to_string(),
                    occurrence: None,
                    decision: match rng.next() % 3 {
                        0 => FaultDecision::Proceed,
                        1 => FaultDecision::Platform {
                            kind: 1 + (rng.next() % 63) as u8,
                            arg: rng.next() & 0x00ff_ffff,
                        },
                        _ => FaultDecision::Workload {
                            kind: 64 + (rng.next() % 192) as u8,
                            arg: rng.next() & 0x00ff_ffff,
                        },
                    },
                })
                .collect();
            let mut responder = InjectResponder::new(TableFaultPlan::new(rules));
            for &(iseq, _) in &inject_pairs {
                responder.answer(&mut ch, iseq, &mut sink);
            }
            let decisions = responder
                .plan_mut()
                .decisions
                .iter()
                .zip(&inject_pairs)
                .map(|(&(iseq, decision), &(piseq, name_id))| {
                    assert_eq!(iseq, piseq);
                    LoggedDecision {
                        iseq,
                        name_id,
                        decision,
                    }
                })
                .collect();
            (decisions, 0)
        }
        Some(log) => {
            // Replay leg: the recorded log is the only decision source.
            let mut responder = InjectResponder::new(LogFaultPlan::new(log));
            for &(iseq, _) in &inject_pairs {
                responder.answer(&mut ch, iseq, &mut sink);
            }
            let plan = responder.plan_mut();
            (Vec::new(), plan.divergences().len() + plan.remaining())
        }
    };

    let drops = ch.drop_counters().unwrap();
    Leg {
        ops: sink.ops,
        events,
        drops,
        decisions,
        divergences,
    }
}

// ---------------------------------------------------------------------------
// Ungated self-tests (the phases track re-runs these from a clean checkout).
// ---------------------------------------------------------------------------

/// The fixture round trip: record with `TableFaultPlan`, replay with
/// `LogFaultPlan` seeded from the record leg's decisions, digests equal on
/// all four surfaces, zero divergences.
#[test]
fn fixture_round_trip_is_bit_identical_across_all_surfaces() {
    for seed in [5u32, 6, 7] {
        let record = run_leg(seed, None);
        assert!(
            !record.decisions.is_empty(),
            "record leg produced decisions"
        );
        let replay = run_leg(seed, Some(record.decisions.clone()));
        assert_eq!(replay.divergences, 0, "seed {seed}: replay diverged");
        assert_digests_equal(&record.digest(), &replay.digest())
            .unwrap_or_else(|m| panic!("seed {seed}: {m}"));
    }
}

/// Deliberate-mismatch negatives, one per surface: a comparison that cannot
/// fail is not evidence. Each perturbation must fail naming its surface.
#[test]
fn negative_one_extra_ring_push_fails_naming_s1() {
    let leg = run_leg(11, None);
    let base = leg.digest();
    let mut ops = leg.ops.clone();
    ops.push(SinkOp::RingPush {
        ring: RingId::C,
        bytes: vec![0u8; 24],
        new_prod: 0xdead,
    });
    let err = assert_digests_equal(&base, &digest_from_trace(&ops, &leg.events, &leg.drops))
        .expect_err("an extra ring push must diverge");
    assert_eq!(err.surface, "S1 ring C/I pushes");
}

#[test]
fn negative_altered_cons_bump_fails_naming_s2() {
    let leg = run_leg(11, None);
    let base = leg.digest();
    let mut ops = leg.ops.clone();
    let bump = ops
        .iter_mut()
        .find_map(|op| match op {
            SinkOp::ConsBump { new_cons, .. } => Some(new_cons),
            _ => None,
        })
        .expect("fixture drains at least one ring");
    *bump = bump.wrapping_add(8);
    let err = assert_digests_equal(&base, &digest_from_trace(&ops, &leg.events, &leg.drops))
        .expect_err("an altered cons bump must diverge");
    assert_eq!(err.surface, "S2 ring A/W cons bumps");
}

#[test]
fn negative_flipped_fault_decision_fails_naming_s3() {
    let leg = run_leg(11, None);
    let base = leg.digest();
    let mut ops = leg.ops.clone();
    let answer = ops
        .iter_mut()
        .find_map(|op| match op {
            SinkOp::PioAnswer { value, .. } => Some(value),
            _ => None,
        })
        .expect("fixture answers at least one inject");
    *answer ^= 1; // flip Proceed <-> Platform{kind:1}
    let err = assert_digests_equal(&base, &digest_from_trace(&ops, &leg.events, &leg.drops))
        .expect_err("a flipped fault decision must diverge");
    assert_eq!(err.surface, "S3 pio answers");
}

#[test]
fn negative_dropped_sdk_event_fails_naming_s4() {
    let leg = run_leg(11, None);
    let base = leg.digest();
    let mut events = leg.events.clone();
    events.pop().expect("fixture drains events");
    let err = assert_digests_equal(&base, &digest_from_trace(&leg.ops, &events, &leg.drops))
        .expect_err("a dropped SDK event must diverge");
    assert_eq!(err.surface, "S4 SDK events/drops");
}

/// Two different seeds produce different digests — guards against a digest
/// that hashes nothing.
#[test]
fn seed_variation_produces_different_digests() {
    let a = run_leg(1, None).digest();
    let b = run_leg(2, None).digest();
    assert!(
        assert_digests_equal(&a, &b).is_err(),
        "seeds 1 and 2 must not collide: {a:?}"
    );
}

/// Same seed, two independent record legs: bit-identical (the in-VM leg's
/// comparison shape, proven host-only).
#[test]
fn same_seed_record_legs_are_bit_identical() {
    let a = run_leg(9, None);
    let b = run_leg(9, None);
    assert_digests_equal(&a.digest(), &b.digest()).unwrap();
    assert_eq!(a.decisions, b.decisions);
}

// ---------------------------------------------------------------------------
// The gated in-VM leg (house double-gate; single-threaded lane invocation in
// the file header). Boot machinery follows m4_snapshot.rs.
// ---------------------------------------------------------------------------

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

fn env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .map(|v| v.parse().unwrap_or_else(|_| panic!("{name}={v} not a u32")))
        .unwrap_or(default)
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

/// No-autostart image containing the canonical inject-calling workload. The
/// root snapshot is taken at Ready; each child launches unit 0 independently.
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
        let dir = build.join("m5-stage-replay-live");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sbin")).unwrap();
        std::fs::create_dir_all(dir.join("opt")).unwrap();
        std::fs::create_dir_all(dir.join("etc/detguest")).unwrap();
        std::fs::copy(musl.join("detguest-agent"), dir.join("sbin/detguest-agent")).unwrap();
        std::fs::copy(musl.join("testload"), dir.join("opt/testload")).unwrap();
        std::fs::write(
            dir.join("etc/detguest/boot.toml"),
            "boot_toml_version = 1\n\n[[unit]]\nid = 0\nexec = \"/opt/testload\"\nargs = [\"--inject-roundtrip\"]\n",
        )
        .unwrap();
        run(
            &root,
            "./image/build.sh",
            &["initramfs", dir.to_str().unwrap()],
        );
        let out_path = build.join("initramfs-m5-replay.cpio");
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

fn m5_config() -> VmConfig {
    let a = artifacts();
    VmConfig::new(a.bzimage.clone(), a.initramfs.clone())
}

fn boot_to_ready() -> VmHarness {
    let cfg = m5_config();
    let mut vm = VmHarness::new(&cfg).expect("harness build");
    let reason = vm
        .run_until(Duration::from_secs(120), |o| {
            o.events.iter().any(|event| {
                matches!(
                    event.payload,
                    detguest_host::OwnedPayload::Ready { unit: u32::MAX, .. }
                )
            })
        })
        .expect("run to warm-up");
    assert_eq!(
        reason,
        StopReason::Predicate,
        "expected no-autostart Ready before stop/timeout; serial:\n{}",
        vm.serial_text()
    );
    assert!(vm.channel.is_some(), "channel attached");
    vm
}

/// Seed-derived plan with structural coverage of all decision classes.
fn seed_fault_plan(seed: u32) -> TableFaultPlan {
    let mut rng = Rng::new(seed);
    TableFaultPlan::new(vec![
        FaultRule {
            name_glob: "ms5.frame.*".into(),
            occurrence: None,
            decision: FaultDecision::Proceed,
        },
        FaultRule {
            name_glob: "ms5.io.read".into(),
            occurrence: None,
            decision: FaultDecision::Platform {
                kind: 1 + (rng.next() % 63) as u8,
                arg: rng.next() & 0x00ff_ffff,
            },
        },
        FaultRule {
            name_glob: "ms5.io.write".into(),
            occurrence: None,
            decision: FaultDecision::Workload {
                kind: 64 + (rng.next() % 192) as u8,
                arg: rng.next() & 0x00ff_ffff,
            },
        },
    ])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AuthoritativeSurfaces {
    final_guest_ram: u64,
    drained_events: u64,
    drop_counters: u64,
    inject_decisions: u64,
}

struct VmLeg {
    diagnostics: RunDigest,
    surfaces: AuthoritativeSurfaces,
    queries: Vec<(u32, u32)>,
    decisions: Vec<LoggedDecision>,
    divergences: usize,
    fault_class_counts: [u32; 3],
}

fn assert_surfaces_equal(
    record: AuthoritativeSurfaces,
    replay: AuthoritativeSurfaces,
) -> Result<(), &'static str> {
    for (name, a, b) in [
        (
            "final guest RAM",
            record.final_guest_ram,
            replay.final_guest_ram,
        ),
        (
            "complete drained event stream",
            record.drained_events,
            replay.drained_events,
        ),
        ("drop counters", record.drop_counters, replay.drop_counters),
        (
            "inject decision LogLines",
            record.inject_decisions,
            replay.inject_decisions,
        ),
    ] {
        if a != b {
            return Err(name);
        }
    }
    Ok(())
}

/// Restore and execute one record or decoded-log replay child.
fn vm_leg(
    cfg: &VmConfig,
    snap: &VmSnapshot,
    seed: u32,
    replay_log: Option<Vec<LoggedDecision>>,
) -> VmLeg {
    let mut child = VmHarness::from_snapshot(cfg, snap).expect("child build");
    let mut rng = Rng::new(seed);
    child.pvpad().set_pad(0, rng.next());
    child.pvpad().schedule(1, 0, rng.next());
    child.responder = match replay_log {
        Some(log) => InjectResponder::new(HarnessFaultPlan::Log(LogFaultPlan::new(log))),
        None => InjectResponder::new(HarnessFaultPlan::Table(seed_fault_plan(seed))),
    };
    child.push_command(&Command::StartWorkload {
        unit: 0,
        log_mask: 0x1f,
    });
    let reason = child
        .run_until(Duration::from_secs(60), |o| {
            o.events.iter().any(|event| {
                matches!(
                    event.payload,
                    detguest_host::OwnedPayload::WorkloadExited {
                        exit_code: 0,
                        term_signal: 0,
                        ..
                    }
                )
            })
        })
        .expect("child run");
    assert_eq!(
        reason,
        StopReason::Predicate,
        "child workload must exit; serial:\n{}",
        child.serial_text()
    );
    let drops = child.channel.as_ref().unwrap().drop_counters().unwrap();
    let queries: Vec<_> = child
        .observed
        .events
        .iter()
        .filter_map(|event| match event.payload {
            detguest_host::OwnedPayload::InjectQuery { iseq, name_id } => Some((iseq, name_id)),
            _ => None,
        })
        .collect();
    let diagnostics = digest_from_trace(&child.sink.ops, &child.observed.events, &drops);
    let surfaces = AuthoritativeSurfaces {
        final_guest_ram: child.guest_ram_hash().expect("hash all guest RAM"),
        drained_events: raw_event_stream_hash(&child.observed.events),
        drop_counters: drop_counter_hash(&drops),
        inject_decisions: inject_log_hash(&child.observed.events),
    };
    let (decisions, divergences) = match child.responder.plan_mut() {
        HarnessFaultPlan::Table(table) => {
            let decisions = table
                .decisions
                .iter()
                .zip(&queries)
                .map(|(&(iseq, decision), &(query_iseq, name_id))| {
                    assert_eq!(iseq, query_iseq);
                    LoggedDecision {
                        iseq,
                        name_id,
                        decision,
                    }
                })
                .collect();
            (decisions, 0)
        }
        HarnessFaultPlan::Log(log) => (Vec::new(), log.divergences().len() + log.remaining()),
    };
    assert_eq!(queries.len(), 6, "canonical fixture query count");
    let mut fault_class_counts = [0; 3];
    for op in &child.sink.ops {
        if let SinkOp::PioAnswer { value, .. } = op {
            let class = match FaultDecision::unpack(*value) {
                FaultDecision::Proceed => 0,
                FaultDecision::Platform { .. } => 1,
                FaultDecision::Workload { .. } => 2,
            };
            fault_class_counts[class] += 1;
        }
    }
    VmLeg {
        diagnostics,
        surfaces,
        queries,
        decisions,
        divergences,
        fault_class_counts,
    }
}

#[test]
#[ignore = "KVM tier: Intel runner only (DETGUEST_VM_TESTS=1)"]
fn determinism_replay_seeded_iterations_are_bit_identical() {
    if !gated() {
        return;
    }
    let start = env_u32("DETGUEST_REPLAY_START_ITER", 0);
    let count = env_u32("DETGUEST_REPLAY_ITER_COUNT", 2);
    let seed_base = env_u32("DETGUEST_REPLAY_SEED_BASE", 0);
    let end = start.checked_add(count).expect("iteration range overflow");

    let cfg = m5_config();
    let mut root = boot_to_ready();
    let snap = root.snapshot().expect("snapshot");
    drop(root);

    let campaign_start = Instant::now();
    for i in start..end {
        let iteration_start = Instant::now();
        let seed = seed_base.wrapping_add(i);
        let record = vm_leg(&cfg, &snap, seed, None);
        assert!(!record.decisions.is_empty());
        assert!(record.fault_class_counts.iter().all(|&count| count > 0));
        let replay = vm_leg(&cfg, &snap, seed, Some(record.decisions.clone()));
        assert_eq!(replay.divergences, 0, "iteration {i}: log replay diverged");
        assert_eq!(
            replay.queries, record.queries,
            "iteration {i}: query sequence"
        );
        assert_surfaces_equal(record.surfaces, replay.surfaces).unwrap_or_else(|surface| {
            panic!(
                "iteration {i} seed {seed}: {surface} diverged; resume with \
                 DETGUEST_REPLAY_START_ITER={i} DETGUEST_REPLAY_ITER_COUNT=1"
            )
        });
        assert_digests_equal(&record.diagnostics, &replay.diagnostics)
            .unwrap_or_else(|m| panic!("iteration {i} diagnostic: {m}"));
        eprintln!(
            "determinism_replay: iteration {i} seed {seed} bit-identical in {:?}: {:?}",
            iteration_start.elapsed(),
            record.surfaces
        );
    }
    eprintln!(
        "determinism_replay: range [{start},{end}) completed in {:?}",
        campaign_start.elapsed()
    );
}
