//! Green criterion 1 of reference-workload request
//! phase3-ready-not-emitted-real-worker: boot the REAL refwork-harness
//! initramfs (not the m9_refwork_contract fixture) through to a *held*
//! guest-sdk `Ready` and past the first frame boundary, with the game
//! delivered through the real pv-blk path (`game_source = "pv-blk"`).
//!
//! Gated: runs only when `REFWORK_READY_INITRAMFS` is set (path to the
//! decompressed reference-workload initramfs cpio, e.g.
//! `zstd -d dist/workload-image-0.1.0/initramfs.cpio.zst`).
//! `REFWORK_READY_BZIMAGE` overrides the kernel (default:
//! image/build/bzImage).
//!
//! Worker-parity notes (documented gaps, not full worker coverage):
//! - pv-blk is the test harness's device model, driven through the same
//!   MMIO window the real device uses; pv-pad is the harness latch stub —
//!   the refwork harness's boot leg never touches pv-pad (NoopPlatform),
//!   so the stub is behavior-identical for this test's scope.
//! - The host drains rings on every guest doorbell (the production worker
//!   buffers ring A mid-run). Ring volume through Ready is ~30 small
//!   records — far below ring capacity — so drain cadence cannot mask the
//!   failure modes this test guards (early control-socket close, boot-leg
//!   wedge); it would only matter for a full-ring critical-emit spin,
//!   which needs worker-side coverage.
//!
//! Assertion order is deliberate: Ready → held (workload alive, frames
//! advancing) → breadcrumbs. A pre-fix (`322c331`) agent reaches Ready and
//! then kills the workload by dropping its fd-3 socket, so the held
//! assertions are the ones that must fire on regression.

use std::path::PathBuf;
use std::time::Duration;

use detguest_host::OwnedPayload;
use detguest_vmtest::harness::{StopReason, VmConfig, VmHarness};

/// A minimal valid game: NOP-filled 32 KiB ROM with the reset vector at
/// 0x8000 — the all-zero blob fails the harness's BadResetVector check.
fn nop_rom() -> Vec<u8> {
    let mut rom = vec![0xeau8; 32 * 1024];
    rom[0x7ffc] = 0x00;
    rom[0x7ffd] = 0x80;
    rom
}

/// refwork meta-page layout (reference-workload `meta.rs`): the running
/// frame counter is a u64 at offset 0x08.
fn meta_frame(vm: &VmHarness) -> u64 {
    let ch = vm.channel.as_ref().expect("channel attached");
    let mut buf = [0u8; 0x10];
    ch.read_region("meta", 0, &mut buf)
        .unwrap_or_else(|e| panic!("read_region(meta): {e:?}"));
    u64::from_le_bytes(buf[0x08..0x10].try_into().unwrap())
}

/// Scan drained events for the symptom-2 death shape: a WorkloadExited or
/// the harness's `frame loop failed` stderr LogLine.
fn workload_death(events: &[detguest_host::GuestEvent]) -> Option<String> {
    events.iter().find_map(|e| match &e.payload {
        OwnedPayload::WorkloadExited {
            guest_pid,
            exit_code,
            term_signal,
        } => Some(format!(
            "WorkloadExited {{ guest_pid: {guest_pid}, exit_code: {exit_code}, \
             term_signal: {term_signal} }}"
        )),
        OwnedPayload::LogLine { msg, .. } => {
            let text = String::from_utf8_lossy(msg);
            text.contains("frame loop failed").then(|| text.into_owned())
        }
        _ => None,
    })
}

#[test]
fn real_harness_reaches_and_holds_ready() {
    let Ok(initramfs) = std::env::var("REFWORK_READY_INITRAMFS") else {
        eprintln!("skipping refwork ready-hold: REFWORK_READY_INITRAMFS unset");
        return;
    };
    let bzimage = std::env::var("REFWORK_READY_BZIMAGE").unwrap_or_else(|_| {
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../image/build/bzImage").to_string()
    });

    let cfg = VmConfig::new(PathBuf::from(bzimage), PathBuf::from(initramfs));
    let mut vm = VmHarness::new(&cfg).expect("harness boots");
    vm.attach_pv_blk(nop_rom());

    // Phase 1: boot to Ready.
    let stop = vm
        .run_until(Duration::from_secs(120), |o| {
            o.events
                .iter()
                .any(|e| matches!(e.payload, OwnedPayload::Ready { .. }))
        })
        .expect("run loop");
    vm.drain();
    assert_eq!(
        stop,
        StopReason::Predicate,
        "guest must emit Ready; serial:\n{}",
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
    // Three registrations, each a seqlock write pass: generation is
    // deterministic for this boot shape (observed 6 on the probe and the
    // real worker alike). A change here means the registration path
    // changed — re-derive, don't delete.
    assert_eq!(generation, 6);

    // Phase 2: Ready must HOLD — the workload stays alive and the frame
    // loop advances past the first frame boundary. The pre-fix agent
    // dropped the workload's fd-3 peer right after Ready, which killed the
    // harness with `control I/O error: control socket closed` (exit 1).
    let f0 = meta_frame(&vm);
    let stop = vm
        .run_until(Duration::from_secs(3), |o| {
            workload_death(&o.events).is_some()
        })
        .expect("run loop");
    vm.drain();
    if let Some(death) = workload_death(&vm.observed.events) {
        panic!("workload died after Ready ({stop:?}): {death}");
    }
    let f1 = meta_frame(&vm);
    assert!(
        f1 > f0 || f1 > 0,
        "frame counter must advance past the first boundary (before={f0}, after={f1})"
    );

    // Phase 3: the boot-leg breadcrumbs arrive in order (the step-01
    // wedge-diagnosis contract; agent LogLine stream 3, level 1).
    let breadcrumbs: Vec<String> = vm
        .observed
        .events
        .iter()
        .filter_map(|e| match &e.payload {
            OwnedPayload::LogLine { stream: 3, msg, .. } => {
                let text = String::from_utf8_lossy(msg);
                text.starts_with("boot: ").then(|| text.into_owned())
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        breadcrumbs,
        [
            "boot: helloack",
            "boot: gameloaded",
            "boot: rw-ready",
            "boot: start-sent",
            "boot: game-unlinked",
            "boot: regions-gated",
            "boot: evidence-done",
        ],
        "boot-leg breadcrumb sequence"
    );
}
