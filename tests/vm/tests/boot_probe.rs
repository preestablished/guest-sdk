// @generated-by: rom-operator-bridge phase3 step-2 diagnosis (2026-07-03)
//! Boot an arbitrary initramfs under the VM harness and dump serial +
//! guest events — a diagnostic probe for silent guest deaths that the
//! production worker path cannot show (it discards serial).
//!
//! Gated: runs only when `BOOT_PROBE_INITRAMFS` is set (path to a cpio).
//! `BOOT_PROBE_BZIMAGE` overrides the kernel (default: image/build/bzImage).
//! `BOOT_PROBE_SECS` overrides the wall deadline (default 60).
//!
//! The probe never asserts guest success — it exists to make the guest's
//! last words visible. It always passes unless the harness itself fails.

use std::path::PathBuf;
use std::time::Duration;

use detguest_vmtest::harness::{VmConfig, VmHarness};

#[test]
fn boot_probe_prints_serial() {
    let Ok(initramfs) = std::env::var("BOOT_PROBE_INITRAMFS") else {
        eprintln!("skipping boot probe: BOOT_PROBE_INITRAMFS unset");
        return;
    };
    let bzimage = std::env::var("BOOT_PROBE_BZIMAGE").unwrap_or_else(|_| {
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../image/build/bzImage").to_string()
    });
    let secs: u64 = std::env::var("BOOT_PROBE_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);

    let cfg = VmConfig::new(PathBuf::from(bzimage), PathBuf::from(initramfs));
    let mut vm = VmHarness::new(&cfg).expect("harness boots");
    if let Ok(game) = std::env::var("BOOT_PROBE_GAME") {
        vm.attach_pv_blk(std::fs::read(game).expect("read BOOT_PROBE_GAME"));
    }
    let stop = vm
        .run_until(Duration::from_secs(secs), |_| false)
        .expect("run loop");
    vm.drain();

    eprintln!("=== boot probe stop: {stop:?} ===");
    eprintln!("=== serial ===\n{}", vm.serial_text());
    eprintln!("=== events: {} drained ===", vm.observed.events.len());
    for ev in vm.observed.events.iter().take(40) {
        eprintln!("{ev:?}");
    }
}
