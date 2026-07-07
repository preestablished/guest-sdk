//! detguest-vmtest — the repo's own minimal KVM test harness (IMPLEMENTATION-PLAN M2).
//!
//! This crate is a normal workspace member so every hosted CI lane builds,
//! formats, and lints it. The tests that actually need `/dev/kvm` follow two
//! gates at once:
//!
//! 1. `#[ignore]` — `cargo test --workspace` on hosted runners never runs them;
//! 2. an env gate — they immediately (and loudly) skip unless
//!    `DETGUEST_VM_TESTS=1`, so even an accidental `-- --ignored` on a laptop
//!    fails soft.
//!
//! The Intel runner's in-VM tier runs them with:
//! `DETGUEST_VM_TESTS=1 cargo test -p detguest-vmtest -- --ignored --test-threads=1`
//!
//! The M2 harness work items (KVM runner, PIO handler, GuestMem over the
//! memslot, pv-pad latch stub, retired-instruction counter, guest-time
//! measurement) land here behind these gates.

pub mod harness;
pub mod replay;

/// True when the environment opts into running KVM-requiring tests.
pub fn vm_tests_enabled() -> bool {
    std::env::var_os("DETGUEST_VM_TESTS").is_some_and(|v| v == "1")
}

/// True when `/dev/kvm` exists and is accessible to this process.
pub fn kvm_available() -> bool {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kvm")
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Placeholder for the in-VM tier; replaced by the real harness tests.
    /// Exercises both gates so the gating pattern itself stays tested.
    #[test]
    #[ignore = "KVM tier: Intel runner only (DETGUEST_VM_TESTS=1)"]
    fn vm_tier_gate_works() {
        if !vm_tests_enabled() {
            eprintln!("skipping: DETGUEST_VM_TESTS != 1");
            return;
        }
        assert!(
            kvm_available(),
            "DETGUEST_VM_TESTS=1 but /dev/kvm not accessible"
        );
    }

    /// Runs everywhere: the wire crate is linkable from the harness.
    #[test]
    fn wire_crate_links() {
        assert_eq!(detguest_wire::PROTO_VERSION, 1);
    }
}
