//! Ms5 `determinism_replay` gate scaffold
//! (`guest-sdk-m5-determinism-replay-ci-gate`, staged by the
//! phase3-ms5-groundwork-while-blocked round).
//!
//! Surface ownership (see `detguest_vmtest::replay` module doc): the
//! hypervisor's `VerifyReplay` proves RAM/framebuffer bit-identity
//! (DRL-4/DRL-5); this gate owns S1–S4 (ring C/I pushes, ring A/W consumer
//! bumps, pio answers, SDK event/drop-counter equivalence).
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
//!   resumable via env knobs: `DETGUEST_REPLAY_ITERS` (default 2 — lane
//!   smoke; round 2's acceptance raises it to 1000),
//!   `DETGUEST_REPLAY_SEED_BASE` (default 0), `DETGUEST_REPLAY_RESUME_AT`
//!   (default 0).
//!
//! **Stub marker: `MS5-STUB`** — grep for it to enumerate exactly what
//! round 2 (`phase3-ms5-execution-in-vm-closeout`) fills. Each stub panics
//! with its awaited round-2 item + grounding checklist ID, so a stub
//! reached is loud, never silently green. No stub sits on the ungated
//! self-test path. `DETGUEST_REPLAY_EXERCISE_STUBS=<inject|ingest|crosscheck>`
//! reaches the named stub from the gated leg (proving loudness); the
//! default lane run does not reach them.

use std::path::{Path, PathBuf};
use std::process::Command as Proc;
use std::sync::OnceLock;
use std::time::Duration;

use detguest_host::{
    Channel, DropCounters, FaultRule, GuestEvent, InjectResponder, LogFaultPlan, LoggedDecision,
    MockGuestMem, RecordingSink, SinkOp, TableFaultPlan,
};
use detguest_vmtest::harness::snapshot::VmSnapshot;
use detguest_vmtest::harness::{HarnessFaultPlan, Observed, StopReason, VmConfig, VmHarness};
use detguest_vmtest::replay::{assert_digests_equal, digest_from_trace, RunDigest};
use detguest_wire::events::{encode_event, encoded_event_len, Command, EventPayload};
use detguest_wire::header::{ChannelHeader, CHANNEL_SIZE, OFF_RESERVED};
use detguest_wire::{FaultDecision, RingId};

// ---------------------------------------------------------------------------
// Round-2 stubs (MS5-STUB). Fill these in phase3-ms5-execution-in-vm-closeout;
// their call sites in the gated leg are marked.
// ---------------------------------------------------------------------------

/// Workload-side `inject_point` call sites in a VM workload bin: today the
/// m4 fixture workload makes zero inject queries, so the in-VM legs compare
/// an empty S3 surface. Round 2 extends testload and re-points the fixture.
fn stub_workload_inject_call_sites() -> ! {
    panic!(
        "MS5-STUB: workload-side inject_point call sites in a VM workload bin — \
         awaits m5-vm-inject-roundtrip (round-2 item 1); grounded by ILDE-6"
    );
}

/// Real-recorded decision ingestion: DHILOG-decoded decisions into
/// `LogFaultPlan` (decoding stays hypervisor-owned; this repo consumes
/// decoded `LoggedDecision`s).
fn stub_ingest_recorded_decisions() -> ! {
    panic!(
        "MS5-STUB: real-recorded decision ingestion (DHILOG-decoded decisions into \
         LogFaultPlan) — awaits round-2 item 1's DHILOG-backed completion; \
         grounded by ILDE-1..6"
    );
}

/// Cross-check the guest-sdk S1–S4 digests against the hypervisor's
/// `VerifyReplay` end-state hashes for the same run.
fn stub_cross_check_verify_replay() -> ! {
    panic!(
        "MS5-STUB: cross-check against hypervisor VerifyReplay end-state hashes — \
         awaits round-2 item 2 (the 1000-iteration gate execution); grounded by \
         DRL-4/DRL-5"
    );
}

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

const WARMUP_FRAME: u32 = 8;
const CHILD_FRAMES: usize = 10;

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
    initramfs_m4: PathBuf,
}

static ARTIFACTS: OnceLock<Artifacts> = OnceLock::new();

/// Same recipe as `m4_snapshot.rs::artifacts` (the m4-regions fixture is the
/// boot-or-restore substrate until round 2 lands an inject-calling workload).
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
        let dir = build.join("m5-stage-replay");
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
        let out_path = build.join("initramfs-m5-replay.cpio");
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

fn m5_config() -> VmConfig {
    let a = artifacts();
    VmConfig::new(a.bzimage.clone(), a.initramfs_m4.clone())
}

fn boot_to_warmup() -> VmHarness {
    let cfg = m5_config();
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

/// Seed-derived fault plan for a child leg. Installed on every child; zero
/// decisions fire until round 2 lands workload-side inject_point call sites
/// (see `stub_workload_inject_call_sites`).
fn seed_fault_plan(seed: u32) -> TableFaultPlan {
    let mut rng = Rng::new(seed);
    TableFaultPlan::new(
        POINT_NAMES
            .iter()
            .map(|name| FaultRule {
                name_glob: format!("{name}*"),
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
            .collect(),
    )
}

/// One in-VM leg: restore a child from the root snapshot, install the
/// seed-derived fault plan, drive a seed-derived input-burst schedule (the
/// m4 pv-pad scheduling pattern), run frames, fold the digest.
fn vm_leg_digest(cfg: &VmConfig, snap: &VmSnapshot, seed: u32) -> RunDigest {
    let mut child = VmHarness::from_snapshot(cfg, snap).expect("child build");
    child.responder = InjectResponder::new(HarnessFaultPlan::Table(seed_fault_plan(seed)));
    let mut rng = Rng::new(seed);
    let base = child.pvpad().frame_counter;
    for i in 0..CHILD_FRAMES as u32 {
        let value = rng.next();
        child.pvpad().schedule(base + 1 + i, 0, value);
    }
    let reason = child
        .run_until(Duration::from_secs(60), |o: &Observed| {
            o.frame_counter_writes.len() >= CHILD_FRAMES
        })
        .expect("child run");
    assert_eq!(
        reason,
        StopReason::Predicate,
        "child must advance {CHILD_FRAMES} frames; serial:\n{}",
        child.serial_text()
    );
    let drops = child
        .channel
        .as_ref()
        .expect("channel attached")
        .drop_counters()
        .expect("drop counters readable");
    digest_from_trace(&child.sink.ops, &child.observed.events, &drops)
}

/// The gate's iteration skeleton: per seed, two independently-restored
/// children with identical seed-derived inputs and fault plans must fold
/// bit-identical S1–S4 digests. Round 2 raises `DETGUEST_REPLAY_ITERS` to
/// 1000 and fills the MS5-STUB call sites below (record leg with live
/// inject decisions → replay leg from the recorded log → VerifyReplay
/// cross-check).
#[test]
#[ignore = "KVM tier: Intel runner only (DETGUEST_VM_TESTS=1)"]
fn determinism_replay_seeded_iterations_are_bit_identical() {
    if !gated() {
        return;
    }
    let iters = env_u32("DETGUEST_REPLAY_ITERS", 2);
    let seed_base = env_u32("DETGUEST_REPLAY_SEED_BASE", 0);
    let resume_at = env_u32("DETGUEST_REPLAY_RESUME_AT", 0);

    let cfg = m5_config();
    let mut root = boot_to_warmup();
    let snap = root.snapshot().expect("snapshot");
    drop(root);

    for i in resume_at..iters {
        let seed = seed_base.wrapping_add(i);
        let a = vm_leg_digest(&cfg, &snap, seed);
        let b = vm_leg_digest(&cfg, &snap, seed);
        if let Err(m) = assert_digests_equal(&a, &b) {
            panic!("iteration {i} (seed {seed}, resume with DETGUEST_REPLAY_RESUME_AT={i}): {m}");
        }
        eprintln!("determinism_replay: iteration {i} (seed {seed}) bit-identical");

        // MS5-STUB call sites — round 2 wires these unconditionally:
        //   1. record leg with workload inject decisions,
        //   2. replay leg seeded from the recorded log,
        //   3. VerifyReplay end-state cross-check.
        match std::env::var("DETGUEST_REPLAY_EXERCISE_STUBS").as_deref() {
            Ok("inject") => stub_workload_inject_call_sites(),
            Ok("ingest") => stub_ingest_recorded_decisions(),
            Ok("crosscheck") => stub_cross_check_verify_replay(),
            _ => {}
        }
    }
}
