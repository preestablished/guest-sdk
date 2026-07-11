//! KVM snapshot / restore / fork for the harness (Ms4 plan package 05).
//!
//! A [`VmSnapshot`] is a full, immutable copy of everything a child VM needs:
//! guest RAM, vCPU state (KVM migration conventions), in-kernel device state
//! (clock, PIT2, both PICs, IOAPIC), the harness's own PIO/pv-pad latches,
//! and the **host-side** channel state (`detguest-host` documents that the
//! ring C/I producer seqs are NOT reconstructible — they must round-trip via
//! `Channel::producer_seqs` / `restore_producer_seqs`).
//!
//! "Fork" is restore-N-times: the root snapshot is never mutated; every
//! [`VmHarness::from_snapshot`] child copies from it, so children are
//! independent. Children start with a fresh `Observed` / drain baseline —
//! test predicates must count deltas since child start, never absolute
//! totals from the root's history.

use std::io;

use kvm_bindings::{
    kvm_clock_data, kvm_debugregs, kvm_irqchip, kvm_lapic_state, kvm_mp_state, kvm_msr_entry,
    kvm_pit_state2, kvm_regs, kvm_sregs, kvm_vcpu_events, kvm_xcrs, kvm_xsave, Msrs,
    KVM_IRQCHIP_IOAPIC, KVM_IRQCHIP_PIC_MASTER, KVM_IRQCHIP_PIC_SLAVE,
    KVM_VCPUEVENT_VALID_NMI_PENDING, KVM_VCPUEVENT_VALID_SHADOW, KVM_VCPUEVENT_VALID_SIPI_VECTOR,
    KVM_VCPUEVENT_VALID_SMM,
};
use kvm_ioctls::{Cap, Kvm, VcpuFd, VmFd};
use vm_memory::{Bytes, GuestAddress};

use detguest_host::{
    Channel, InjectResponder, InternSnapshotEntry, ProducerSeqs, RecordingSink, TableFaultPlan,
};

use super::icount::GuestIcount;
use super::pio::PioState;
use super::{
    create_vcpu_with_cpuid, create_vm_core, install_vcpu_kick_handler, Observed, VmConfig, VmCore,
    VmHarness,
};

/// MSR indices we deliberately do NOT capture/restore, even though
/// `KVM_GET_MSR_INDEX_LIST` advertises them on this machine. This is a
/// checked-in constant (determinism: no runtime discovery); every entry
/// carries the empirical reason observed on `infra-control-kvm-intel`.
/// Precedent: Firecracker's serializable-MSR deny list.
const MSR_SKIP_LIST: &[(u32, &str)] = &[
    // 0x4b564d00.. MSR_KVM_* paravirt MSRs would be candidates, but the
    // guest is built with CONFIG_HYPERVISOR_GUEST unset and never touches
    // them; they read back 0 and restore cleanly, so they stay captured.
    //
    // (empirically empty on this host so far — get/set both succeed for
    // every advertised index; extend with `(index, "reason")` entries if a
    // kernel/microcode update ever changes that, and keep the comment.)
];

fn msr_skipped(index: u32) -> bool {
    MSR_SKIP_LIST.iter().any(|(i, _)| *i == index)
}

/// Captured vCPU state, in KVM migration-convention capture order.
/// `kvm_fpu` is deliberately absent: `kvm_xsave` covers the FPU.
struct VcpuState {
    regs: kvm_regs,
    sregs: kvm_sregs,
    xsave: kvm_xsave,
    xcrs: kvm_xcrs,
    /// Every readable MSR from `KVM_GET_MSR_INDEX_LIST` minus the skip-list
    /// (includes `MSR_IA32_TSC`, which is how the TSC round-trips).
    msrs: Vec<kvm_msr_entry>,
    lapic: kvm_lapic_state,
    events: kvm_vcpu_events,
    debug_regs: kvm_debugregs,
    mp_state: kvm_mp_state,
}

/// One host-interned name captured from the root channel's intern map
/// (`Channel::interns()`), kept in the snapshot so `from_snapshot` can
/// re-seed the child's name resolution via `Channel::restore_interns`.
#[derive(Clone)]
pub struct InternRecord {
    /// The guest-assigned name_id.
    pub name_id: u32,
    /// Raw name bytes. The host caches names as lossy UTF-8 `String`s, so
    /// these are the lossy form's bytes; converting back with
    /// `String::from_utf8_lossy` at restore is exact (already valid UTF-8).
    pub name: Vec<u8>,
    /// REACHABLE_DECL flag.
    pub reachable_decl: bool,
}

/// Host-side `detguest_host::Channel` state (load-bearing: a child never
/// re-executes CHANNEL_INIT, so `from_snapshot` must re-attach and restore
/// the non-reconstructible producer seqs or the next `push_command` would
/// re-emit an already-used seq at a guest consumer expecting a continuation).
pub struct HostChannelState {
    /// Channel base GPA (from the PIO CHANNEL_INIT latches).
    gpa: u64,
    /// `Channel::producer_seqs()` at snapshot time (rings C/I).
    producer_seqs: ProducerSeqs,
    /// Intern map at snapshot time, captured from `Channel::interns()` (the
    /// host folds ring A+W interns into one map) and re-seeded into the
    /// child via `Channel::restore_interns` — children resolve names
    /// directly, without falling back to manifest name bytes.
    pub interns: Vec<InternRecord>,
    /// Pending-inject table (iseq → name_id of drained-but-unanswered
    /// `InjectQuery` events) at snapshot time, re-seeded via
    /// `Channel::restore_pending_injects`. Preserves in-flight inject
    /// queries across a restore; snapshots taken at a quiet boundary simply
    /// carry an empty table.
    pub pending_injects: Vec<(u32, u32)>,
}

/// A full VM snapshot. Immutable by construction: `from_snapshot` only reads
/// from it, so one root snapshot can seed any number of children.
pub struct VmSnapshot {
    /// Full guest RAM copy.
    memory: Vec<u8>,
    vcpu: VcpuState,
    clock: kvm_clock_data,
    pit: kvm_pit_state2,
    pic_master: kvm_irqchip,
    pic_slave: kvm_irqchip,
    ioapic: kvm_irqchip,
    /// Detcall latches (incl. the CHANNEL_INIT GPA words) + pv-pad state
    /// (pads, frame counter, and the input schedule queue).
    pio: PioState,
    /// Host channel state; `None` if the guest had not committed
    /// CHANNEL_INIT yet.
    host_channel: Option<HostChannelState>,
}

impl VmSnapshot {
    /// Deliberately offset the saved ring-C producer sequence.
    ///
    /// This negative-test seam changes only the host-side checkpoint, never
    /// captured guest RAM. The snapshot acceptance uses it to prove that a
    /// corrupt non-reconstructible sequence is reported as a named mismatch.
    #[doc(hidden)]
    pub fn corrupt_ring_c_producer_seq_for_test(&mut self, delta: u32) {
        let state = self
            .host_channel
            .as_mut()
            .expect("snapshot has no attached channel");
        state.producer_seqs.ring_c = state.producer_seqs.ring_c.wrapping_add(delta);
    }
}

/// Assert that plain `kvm_xsave` get/set is sound on this vCPU: with dynamic
/// XSTATE features (e.g. AMX) enabled, the kernel's xsave area outgrows the
/// fixed 4096-byte struct and `set_xsave` would write out of bounds.
/// `KVM_CAP_XSAVE2` (queried on the VM fd) reports the required size.
fn assert_xsave_fits(vm: &VmFd) {
    let needed = vm.check_extension_int(Cap::Xsave2);
    assert!(
        (needed as usize) <= std::mem::size_of::<kvm_xsave>(),
        "KVM_CAP_XSAVE2 reports {needed} bytes > kvm_xsave ({}); \
         dynamic XSTATE features are enabled — plain get/set_xsave is unsound",
        std::mem::size_of::<kvm_xsave>()
    );
}

/// Capture every readable MSR, one index at a time. NOT one bulk `get_msrs`
/// over the whole list: KVM stops at the first failing index and kvm-ioctls
/// returns the short count — a single unreadable MSR would silently drop the
/// tail and the restore would diverge undetected. Per-MSR reads make every
/// failure loud and attributable.
fn capture_msrs(kvm: &Kvm, vcpu: &VcpuFd) -> io::Result<Vec<kvm_msr_entry>> {
    let index_list = kvm.get_msr_index_list().map_err(io::Error::from)?;
    let mut out = Vec::with_capacity(index_list.as_slice().len());
    for &index in index_list.as_slice() {
        if msr_skipped(index) {
            continue;
        }
        let mut one = Msrs::from_entries(&[kvm_msr_entry {
            index,
            ..Default::default()
        }])
        .map_err(|e| io::Error::other(format!("msr fam alloc: {e:?}")))?;
        let n = vcpu.get_msrs(&mut one).map_err(io::Error::from)?;
        assert_eq!(
            n, 1,
            "MSR {index:#x} is advertised by KVM_GET_MSR_INDEX_LIST but not \
             readable — add it to MSR_SKIP_LIST with a reason"
        );
        out.push(one.as_slice()[0]);
    }
    Ok(out)
}

/// Restore the captured MSRs, one at a time, asserting each set sticks (a
/// bulk set has the same silent-tail-drop failure mode as a bulk get).
fn restore_msrs(vcpu: &VcpuFd, msrs: &[kvm_msr_entry]) -> io::Result<()> {
    for e in msrs {
        debug_assert!(!msr_skipped(e.index), "skip-list applied at capture");
        let one = Msrs::from_entries(&[*e])
            .map_err(|e| io::Error::other(format!("msr fam alloc: {e:?}")))?;
        let n = vcpu.set_msrs(&one).map_err(io::Error::from)?;
        assert_eq!(
            n, 1,
            "MSR {:#x} was captured but refused the restore write — add it \
             to MSR_SKIP_LIST with a reason",
            e.index
        );
    }
    Ok(())
}

fn get_irqchip(vm: &VmFd, chip_id: u32) -> io::Result<kvm_irqchip> {
    let mut chip = kvm_irqchip {
        chip_id,
        ..Default::default()
    };
    vm.get_irqchip(&mut chip).map_err(io::Error::from)?;
    Ok(chip)
}

impl VmHarness {
    /// Capture full VM state. Only valid between `run_until` calls: the vCPU
    /// must be out of `KVM_RUN` and every exit fully serviced, which
    /// `run_until`'s return guarantees (the harness is synchronous and
    /// tracks no pending exit servicing).
    pub fn snapshot(&mut self) -> io::Result<VmSnapshot> {
        let vcpu = self
            .vcpu
            .as_ref()
            .expect("snapshot() must not be called from inside run_until");

        // Guest RAM first (the vCPU is stopped; nothing mutates it).
        let mut memory = vec![0u8; self.mem.len()];
        self.guest_mem
            .read_slice(&mut memory, GuestAddress(0))
            .map_err(|e| io::Error::other(format!("guest RAM copy: {e}")))?;

        // vCPU state, in KVM migration capture order. xsave covers the FPU;
        // the guard proves the fixed-size struct is large enough.
        assert_xsave_fits(&self.vm);
        let vcpu_state = VcpuState {
            regs: vcpu.get_regs().map_err(io::Error::from)?,
            sregs: vcpu.get_sregs().map_err(io::Error::from)?,
            xsave: vcpu.get_xsave().map_err(io::Error::from)?,
            xcrs: vcpu.get_xcrs().map_err(io::Error::from)?,
            msrs: capture_msrs(&self.kvm, vcpu)?,
            lapic: vcpu.get_lapic().map_err(io::Error::from)?,
            events: vcpu.get_vcpu_events().map_err(io::Error::from)?,
            debug_regs: vcpu.get_debug_regs().map_err(io::Error::from)?,
            mp_state: vcpu.get_mp_state().map_err(io::Error::from)?,
        };

        // In-kernel device state.
        let clock = self.vm.get_clock().map_err(io::Error::from)?;
        let pit = self.vm.get_pit2().map_err(io::Error::from)?;
        let pic_master = get_irqchip(&self.vm, KVM_IRQCHIP_PIC_MASTER)?;
        let pic_slave = get_irqchip(&self.vm, KVM_IRQCHIP_PIC_SLAVE)?;
        let ioapic = get_irqchip(&self.vm, KVM_IRQCHIP_IOAPIC)?;

        // Host-side channel state (REQUIRED for push_command in children).
        // Interns/pending injects come straight off the channel's own maps —
        // the authoritative folded state — not from re-scanning the root's
        // drained event history.
        let host_channel = self.channel.as_ref().map(|ch| HostChannelState {
            gpa: ch.base_gpa(),
            producer_seqs: ch.producer_seqs(),
            interns: ch
                .interns()
                .into_iter()
                .map(|e| InternRecord {
                    name_id: e.name_id,
                    name: e.name.into_bytes(),
                    reachable_decl: e.reachable_decl,
                })
                .collect(),
            pending_injects: ch.pending_injects(),
        });

        Ok(VmSnapshot {
            memory,
            vcpu: vcpu_state,
            clock,
            pit,
            pic_master,
            pic_slave,
            ioapic,
            pio: self.pio.clone(),
            host_channel,
        })
    }

    /// Boot-free child construction: a fresh VM/vCPU carrying `snap`'s
    /// state. `cfg` must describe the same machine shape the snapshot was
    /// taken from (only `mem_size` is used — the kernel/initramfs paths are
    /// irrelevant because nothing is booted).
    pub fn from_snapshot(cfg: &VmConfig, snap: &VmSnapshot) -> io::Result<VmHarness> {
        assert_eq!(
            cfg.mem_size,
            snap.memory.len(),
            "snapshot was taken from a {}-byte guest",
            snap.memory.len()
        );
        let VmCore {
            kvm,
            vm,
            guest_mem,
            mem,
        } = create_vm_core(cfg.mem_size, cfg.timer_interrupts)?;

        guest_mem
            .write_slice(&snap.memory, GuestAddress(0))
            .map_err(|e| io::Error::other(format!("guest RAM restore: {e}")))?;

        // vCPU restore, in KVM migration set order: cpuid (same table as
        // `new`), sregs before regs, xsave/xcrs before MSRs, then lapic,
        // events, debug regs, mp_state.
        let vcpu = create_vcpu_with_cpuid(&kvm, &vm)?;
        vcpu.set_sregs(&snap.vcpu.sregs).map_err(io::Error::from)?;
        vcpu.set_regs(&snap.vcpu.regs).map_err(io::Error::from)?;
        assert_xsave_fits(&vm);
        // SAFETY: the Xsave2 guard above proves the kernel's xsave area fits
        // the fixed 4096-byte kvm_xsave (no dynamic XSTATE features), so the
        // kernel reads no bytes beyond the struct we pass.
        unsafe { vcpu.set_xsave(&snap.vcpu.xsave).map_err(io::Error::from)? };
        vcpu.set_xcrs(&snap.vcpu.xcrs).map_err(io::Error::from)?;
        restore_msrs(&vcpu, &snap.vcpu.msrs)?;
        vcpu.set_lapic(&snap.vcpu.lapic).map_err(io::Error::from)?;
        // kvm_vcpu_events flag discipline: keep only the captured
        // state-carrying valid bits (SHADOW/SMM), and additionally assert
        // the async fields (nmi.pending, sipi_vector) really are restored.
        // Never set VALID_PAYLOAD: KVM_CAP_EXCEPTION_PAYLOAD is not enabled
        // on this VM, so that flag would request stale-exception semantics
        // we did not capture.
        let mut events = snap.vcpu.events;
        events.flags &= KVM_VCPUEVENT_VALID_SHADOW | KVM_VCPUEVENT_VALID_SMM;
        events.flags |= KVM_VCPUEVENT_VALID_NMI_PENDING | KVM_VCPUEVENT_VALID_SIPI_VECTOR;
        vcpu.set_vcpu_events(&events).map_err(io::Error::from)?;
        vcpu.set_debug_regs(&snap.vcpu.debug_regs)
            .map_err(io::Error::from)?;
        vcpu.set_mp_state(snap.vcpu.mp_state)
            .map_err(io::Error::from)?;

        // In-kernel device state. kvmclock is compiled out of the guest
        // (CONFIG_HYPERVISOR_GUEST unset) but set_clock keeps the PIT time
        // base coherent; KVM_SET_CLOCK rejects flags returned by get (e.g.
        // KVM_CLOCK_TSC_STABLE), so clear them.
        let mut clock = snap.clock;
        clock.flags = 0;
        vm.set_clock(&clock).map_err(io::Error::from)?;
        vm.set_pit2(&snap.pit).map_err(io::Error::from)?;
        vm.set_irqchip(&snap.pic_master).map_err(io::Error::from)?;
        vm.set_irqchip(&snap.pic_slave).map_err(io::Error::from)?;
        vm.set_irqchip(&snap.ioapic).map_err(io::Error::from)?;

        // Host channel: re-attach over the child's memslot, restore the
        // non-reconstructible producer seqs, and re-seed the intern map and
        // pending-inject table so the child resolves names / answers injects
        // without any drain having occurred.
        let channel = match &snap.host_channel {
            Some(hc) => {
                let mut ch = Channel::attach(mem, hc.gpa)
                    .map_err(|e| io::Error::other(format!("child channel attach: {e:?}")))?;
                ch.restore_producer_seqs(hc.producer_seqs);
                ch.restore_interns(hc.interns.iter().map(|r| InternSnapshotEntry {
                    name_id: r.name_id,
                    name: String::from_utf8_lossy(&r.name).into_owned(),
                    reachable_decl: r.reachable_decl,
                }));
                ch.restore_pending_injects(hc.pending_injects.iter().copied());
                Some(ch)
            }
            None => None,
        };

        // Process-wide + idempotent, exactly like `new`: a child may be the
        // first harness this process builds.
        install_vcpu_kick_handler();

        Ok(VmHarness {
            kvm,
            vm,
            vcpu: Some(vcpu),
            guest_mem,
            mem,
            pio: snap.pio.clone(),
            channel,
            responder: InjectResponder::new(TableFaultPlan::new(Vec::new())),
            sink: RecordingSink::default(),
            // Fresh baselines: children count deltas, never root totals.
            observed: Observed::default(),
            icount: GuestIcount::open()?,
        })
    }
}
