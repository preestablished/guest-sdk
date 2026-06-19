//! Synthetic M9 reference-workload contract binary.
//!
//! This is a deterministic stand-in for the full reference-workload harness
//! while the emulator-side control state machine is still landing. It publishes
//! the canonical M9 regions expected by the hypervisor fixture contract and
//! then parks forever so the agent can gate Ready on those registrations.

use std::thread;

use detguest_sdk::{self as sdk, RegionFlags};

static WRAM: [u8; 4096] = [0; 4096];
static FRAMEBUFFER: [u8; 4096] = [0; 4096];
static META: [u8; 256] = [0; 256];

fn main() {
    let _ = sdk::init();
    publish_regions();
    loop {
        thread::park();
    }
}

fn publish_regions() {
    // SAFETY: these static byte arrays are mapped for the process lifetime and
    // never move, satisfying the SDK region registration contract.
    unsafe {
        let _wram =
            sdk::register_region("wram", 1, WRAM.as_ptr(), WRAM.len(), RegionFlags::empty());
        let _framebuffer = sdk::register_region(
            "framebuffer",
            1,
            FRAMEBUFFER.as_ptr(),
            FRAMEBUFFER.len(),
            RegionFlags::FRAMEBUFFER,
        );
        let _meta =
            sdk::register_region("meta", 1, META.as_ptr(), META.len(), RegionFlags::empty());
    }
}
