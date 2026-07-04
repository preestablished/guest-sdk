//! The repo's own minimal KVM test harness (IMPLEMENTATION-PLAN M2):
//! boots the `image/`-built kernel + initramfs on raw KVM, handles the
//! detcall PIO ports against `detguest-host`, stubs the pv-pad MMIO latch,
//! and counts guest retired instructions via perf.
//!
//! It mirrors `determinism-hypervisor`'s KVM setup path but depends on
//! nothing from that repo. The kernel cmdline used here is harness-local and
//! explicitly NON-canonical (the canonical cmdline is hypervisor-owned —
//! GitHub issue #1).

pub mod icount;
pub mod memslot;
pub mod pio;
pub mod pvblk;
pub mod snapshot;
pub mod x86;

use std::fs::File;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use kvm_ioctls::{Kvm, VcpuExit, VcpuFd, VmFd};
use linux_loader::configurator::{linux::LinuxBootConfigurator, BootConfigurator, BootParams};
use linux_loader::loader::bootparam::boot_params;
use linux_loader::loader::{bzimage::BzImage, KernelLoader};
use vm_memory::{Bytes, GuestAddress, GuestMemory, GuestMemoryMmap};

use detguest_host::{Channel, GuestEvent, InjectResponder, RecordingSink, TableFaultPlan};

use self::icount::GuestIcount;
use self::memslot::MemSlot;
use self::pio::{PioState, PvPad};
use self::pvblk::PvBlkModel;

/// Guest physical layout constants.
const BOOT_PARAMS_ADDR: u64 = 0x7000;
const CMDLINE_ADDR: u64 = 0x20000;
const CMDLINE_MAX: usize = 0x800;
const HIMEM_START: u64 = 0x10_0000;

/// The deterministic worker's forced timer-disabling flags
/// (`dh-vmm/src/config.rs:92` `BZIMAGE_FORCED_CMDLINE`): no TSC, no APIC
/// timer, jiffies as the clocksource. `lpj=4096` is load-bearing, not
/// cosmetic: when the quick TSC calibration against the PIT counter fails
/// (it is flaky — see the SPEAKER_DUMMY note in `create_vm_core`),
/// calibrate_delay falls back to converging on jiffies, and with delivery
/// suppressed jiffies never advance — a nondeterministic pre-userspace
/// hang. The preset skips calibration entirely, exactly like the worker.
/// Appended to the harness default cmdline by the no-timer reproducer
/// tests (request phase3-boot-scheduling-deadlock).
pub const TIMERLESS_CMDLINE_FLAGS: &str =
    "notsc tsc=unstable clocksource=jiffies noapictimer lpj=4096";

/// Harness configuration.
pub struct VmConfig {
    /// Path to the `image/build.sh`-built bzImage.
    pub bzimage: PathBuf,
    /// Path to the assembled initramfs cpio.
    pub initramfs: PathBuf,
    /// Harness-local cmdline (NON-canonical; see module docs).
    pub cmdline: String,
    /// Guest RAM in bytes (MAP.md canonical demo guest: 128 MiB).
    pub mem_size: usize,
    /// Deliver timer interrupts (irqchip PIT → IRQ0). Default true. False
    /// reproduces the deterministic worker's environment: interrupts exist
    /// as machinery but nothing is armed, so the guest's jiffies never
    /// advance (request phase3-boot-scheduling-deadlock).
    pub timer_interrupts: bool,
}

impl VmConfig {
    /// Defaults matching the M2 acceptance environment.
    pub fn new(bzimage: PathBuf, initramfs: PathBuf) -> VmConfig {
        VmConfig {
            bzimage,
            initramfs,
            // 8250 console on, panic reboots (-> triple fault visible to us),
            // quiet-ish boot. Deliberately NOT the canonical deterministic
            // cmdline (issue #1) — this is the harness's own boot.
            // hugepages=N pre-fills the 2 MiB pool the agent's channel alloc
            // needs (tinyconfig has no runtime sysctl path configured).
            cmdline: "console=ttyS0,115200 panic=-1 reboot=t 8250.nr_uarts=1 hugepages=4"
                .to_string(),
            mem_size: 128 << 20,
            timer_interrupts: true,
        }
    }

    /// The current cmdline plus [`TIMERLESS_CMDLINE_FLAGS`] — the no-timer
    /// reproducer cmdline (assign it back to `self.cmdline`).
    pub fn timerless_cmdline(&self) -> String {
        format!("{} {}", self.cmdline, TIMERLESS_CMDLINE_FLAGS)
    }
}

/// Everything observed from the guest so far.
#[derive(Default)]
pub struct Observed {
    /// Drained guest events, in drain order.
    pub events: Vec<GuestEvent>,
    /// Raw serial output bytes (kernel console + agent eprintln).
    pub serial: Vec<u8>,
    /// FRAME_COUNTER MMIO writes, in order (pv-pad latch stub).
    pub frame_counter_writes: Vec<u32>,
    /// QUIESCE_ACK detcall payloads, in order.
    pub quiesce_acks: Vec<u32>,
}

/// Why `run_until` returned.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum StopReason {
    /// The predicate matched.
    Predicate,
    /// The vCPU halted (the agent's power-off path lands here under
    /// `reboot=t`-less halts) or the guest triple-faulted/shut down
    /// (`reboot=t` + `panic=-1`, and `RB_POWER_OFF` without ACPI).
    GuestStopped,
    /// The wall-clock deadline expired.
    Timeout,
}

/// The harness VM.
pub struct VmHarness {
    kvm: Kvm,
    vm: VmFd,
    vcpu: Option<VcpuFd>,
    guest_mem: GuestMemoryMmap,
    mem: MemSlot,
    pio: PioState,
    /// Attached after the guest's CHANNEL_INIT (INIT_GO).
    pub channel: Option<Channel<MemSlot>>,
    /// Answers inject queries (drain-matched).
    pub responder: InjectResponder<TableFaultPlan>,
    /// The input-log trace of every host channel mutation.
    pub sink: RecordingSink,
    /// Everything observed so far.
    pub observed: Observed,
    /// Guest retired-instruction counter (perf, guest-only).
    pub icount: GuestIcount,
}

/// The common "create vm + tss + irqchip + PIT2 + memslot" setup shared by
/// [`VmHarness::new`] (boot path) and [`VmHarness::from_snapshot`]
/// (state-restore path).
struct VmCore {
    kvm: Kvm,
    vm: VmFd,
    guest_mem: GuestMemoryMmap,
    mem: MemSlot,
}

fn create_vm_core(mem_size: usize, timer_interrupts: bool) -> io::Result<VmCore> {
    let kvm = Kvm::new().map_err(io::Error::from)?;
    let vm = kvm.create_vm().map_err(io::Error::from)?;
    // Legacy Intel requirement: TSS + identity map out of the way of RAM
    // we use (above 128 MiB would collide with nothing, but the
    // conventional 0xfffbc000 area below 4 GiB works with any RAM size).
    vm.set_tss_address(0xfffb_d000).map_err(io::Error::from)?;
    vm.create_irq_chip().map_err(io::Error::from)?;
    // SPEAKER_DUMMY is load-bearing: without it, port 0x61 (PIT refresh
    // toggle) exits to userspace, our constant answer never toggles, and
    // the kernel's PIT-polled TSC calibration spins forever (a flaky
    // boot hang whenever fast TSC calibration fails).
    let pit = kvm_bindings::kvm_pit_config {
        flags: kvm_bindings::KVM_PIT_SPEAKER_DUMMY,
        ..Default::default()
    };
    vm.create_pit2(pit).map_err(io::Error::from)?;
    if !timer_interrupts {
        suppress_timer_gsi_routes(&vm)?;
    }

    let guest_mem: GuestMemoryMmap = GuestMemoryMmap::from_ranges(&[(GuestAddress(0), mem_size)])
        .map_err(|e| io::Error::other(format!("guest memory: {e}")))?;
    let host_addr = guest_mem
        .get_host_address(GuestAddress(0))
        .map_err(|e| io::Error::other(format!("host addr: {e}")))?;
    let slot = kvm_bindings::kvm_userspace_memory_region {
        slot: 0,
        flags: 0,
        guest_phys_addr: 0,
        memory_size: mem_size as u64,
        userspace_addr: host_addr as u64,
    };
    // SAFETY: the region maps a live GuestMemoryMmap allocation that
    // outlives the VM (owned by the harness struct).
    unsafe { vm.set_user_memory_region(slot).map_err(io::Error::from)? };
    let mem = MemSlot::new(host_addr, mem_size);
    Ok(VmCore {
        kvm,
        vm,
        guest_mem,
        mem,
    })
}

/// Suppress timer-interrupt *delivery* while keeping the irqchip + PIT
/// intact (SPEAKER_DUMMY and the in-kernel 0x61/0x42 emulation stay, so the
/// kernel's PIT-polled TSC calibration cannot hang — request
/// phase3-boot-scheduling-deadlock §3 caveat): replace the GSI routing
/// table with KVM's post-CREATE_IRQCHIP default minus GSIs 0 and 2.
///
/// Pin model (kernel `virt/kvm` `irq_comm.c` `default_routing`): GSI n maps
/// to PIC pin n%8 AND IOAPIC pin n for n in 0..16, IOAPIC-only for 16..24 —
/// i.e. GSI 0 → IOAPIC *pin 0*; the familiar GSI0→IOAPIC-pin-2 remap is a
/// QEMU userspace convention, not the kernel default. The in-kernel PIT
/// injects via `kvm_set_irq(…, gsi 0, …)`, so omitting GSI 0 alone
/// suppresses delivery; GSI 2 is dropped as well for belt-and-braces. A
/// missing route fails silently — `pit_do_work` ignores `kvm_set_irq`'s
/// return and in default reinject mode simply stops re-arming — which is
/// exactly the behavior we want: no VM error, no delivery. The guest cannot
/// re-enable it (unmasking its PIC IMR is irrelevant; the host table has no
/// route).
fn suppress_timer_gsi_routes(vm: &VmFd) -> io::Result<()> {
    use kvm_bindings::{
        kvm_irq_routing_entry, KvmIrqRouting, KVM_IRQCHIP_IOAPIC, KVM_IRQCHIP_PIC_MASTER,
        KVM_IRQCHIP_PIC_SLAVE, KVM_IRQ_ROUTING_IRQCHIP,
    };

    fn entry(gsi: u32, irqchip: u32, pin: u32) -> kvm_irq_routing_entry {
        let mut e = kvm_irq_routing_entry {
            gsi,
            type_: KVM_IRQ_ROUTING_IRQCHIP,
            ..Default::default()
        };
        e.u.irqchip.irqchip = irqchip;
        e.u.irqchip.pin = pin;
        e
    }

    let mut entries = Vec::new();
    for gsi in 0u32..24 {
        if gsi == 0 || gsi == 2 {
            continue;
        }
        entries.push(entry(gsi, KVM_IRQCHIP_IOAPIC, gsi));
        if gsi < 16 {
            let pic = if gsi < 8 {
                KVM_IRQCHIP_PIC_MASTER
            } else {
                KVM_IRQCHIP_PIC_SLAVE
            };
            entries.push(entry(gsi, pic, gsi % 8));
        }
    }
    let mut routing = KvmIrqRouting::new(entries.len())
        .map_err(|e| io::Error::other(format!("KvmIrqRouting alloc: {e}")))?;
    routing.as_mut_slice().copy_from_slice(&entries);
    vm.set_gsi_routing(&routing).map_err(io::Error::from)
}

/// Create vCPU 0 with the full supported-CPUID table (identical in the boot
/// and restore paths — CPUID determines guest-visible feature bits).
fn create_vcpu_with_cpuid(kvm: &Kvm, vm: &VmFd) -> io::Result<VcpuFd> {
    let vcpu = vm.create_vcpu(0).map_err(io::Error::from)?;
    let cpuid = kvm
        .get_supported_cpuid(kvm_bindings::KVM_MAX_CPUID_ENTRIES)
        .map_err(io::Error::from)?;
    vcpu.set_cpuid2(&cpuid).map_err(io::Error::from)?;
    Ok(vcpu)
}

impl VmHarness {
    /// Build and fully configure the VM (memory, kernel, initramfs, boot
    /// params, long-mode vCPU state, irqchip+PIT, perf counter).
    pub fn new(cfg: &VmConfig) -> io::Result<VmHarness> {
        let VmCore {
            kvm,
            vm,
            guest_mem,
            mem,
        } = create_vm_core(cfg.mem_size, cfg.timer_interrupts)?;

        // ---- load kernel (bzImage), initramfs, cmdline, boot params ----
        let mut kernel = File::open(&cfg.bzimage)?;
        let loaded = BzImage::load(
            &guest_mem,
            None,
            &mut kernel,
            Some(GuestAddress(HIMEM_START)),
        )
        .map_err(|e| io::Error::other(format!("bzImage load: {e}")))?;
        let mut setup = loaded
            .setup_header
            .ok_or_else(|| io::Error::other("bzImage without setup header"))?;

        let initramfs_bytes = std::fs::read(&cfg.initramfs)?;
        // Load the initramfs high and 2 MiB-aligned, below the top of RAM.
        let initrd_addr = (cfg.mem_size as u64 - initramfs_bytes.len() as u64) & !((2 << 20) - 1);
        guest_mem
            .write_slice(&initramfs_bytes, GuestAddress(initrd_addr))
            .map_err(|e| io::Error::other(format!("initramfs write: {e}")))?;

        let mut cmdline = linux_loader::cmdline::Cmdline::new(CMDLINE_MAX)
            .map_err(|e| io::Error::other(format!("cmdline: {e}")))?;
        cmdline
            .insert_str(&cfg.cmdline)
            .map_err(|e| io::Error::other(format!("cmdline: {e}")))?;
        linux_loader::loader::load_cmdline(&guest_mem, GuestAddress(CMDLINE_ADDR), &cmdline)
            .map_err(|e| io::Error::other(format!("cmdline load: {e}")))?;

        setup.type_of_loader = 0xFF;
        setup.cmd_line_ptr = CMDLINE_ADDR as u32;
        setup.cmdline_size = cfg.cmdline.len() as u32 + 1;
        setup.ramdisk_image = initrd_addr as u32;
        setup.ramdisk_size = initramfs_bytes.len() as u32;

        let mut params = boot_params {
            hdr: setup,
            ..Default::default()
        };
        // e820: conventional low memory + everything above 1 MiB.
        params.e820_table[0].addr = 0;
        params.e820_table[0].size = 0x0009_FC00;
        params.e820_table[0].type_ = 1; // E820_RAM
        params.e820_table[1].addr = HIMEM_START;
        params.e820_table[1].size = cfg.mem_size as u64 - HIMEM_START;
        params.e820_table[1].type_ = 1;
        params.e820_entries = 2;

        LinuxBootConfigurator::write_bootparams(
            &BootParams::new(&params, GuestAddress(BOOT_PARAMS_ADDR)),
            &guest_mem,
        )
        .map_err(|e| io::Error::other(format!("boot params: {e}")))?;

        // ---- vCPU: CPUID + 64-bit long mode entry ----
        let vcpu = create_vcpu_with_cpuid(&kvm, &vm)?;
        x86::setup_long_mode(&vcpu, &guest_mem)?;
        let mut regs = vcpu.get_regs().map_err(io::Error::from)?;
        regs.rip = loaded.kernel_load.0 + 0x200; // 64-bit entry point
        regs.rsi = BOOT_PARAMS_ADDR;
        regs.rsp = 0x8ff0;
        regs.rflags = 2;
        vcpu.set_regs(&regs).map_err(io::Error::from)?;

        let icount = GuestIcount::open()?;
        install_vcpu_kick_handler();

        Ok(VmHarness {
            kvm,
            vm,
            vcpu: Some(vcpu),
            guest_mem,
            mem,
            pio: PioState::new(),
            channel: None,
            responder: InjectResponder::new(TableFaultPlan::new(Vec::new())),
            sink: RecordingSink::default(),
            observed: Observed::default(),
            icount,
        })
    }

    /// Borrow the vm-memory view (test assertions on raw guest RAM).
    pub fn guest_memory(&self) -> &GuestMemoryMmap {
        &self.guest_mem
    }

    /// Drain channel events into `observed` (pause-boundary or doorbell).
    pub fn drain(&mut self) {
        if let Some(ch) = self.channel.as_mut() {
            match ch.drain_events(&mut self.sink) {
                Ok(evs) => self.observed.events.extend(evs),
                Err(e) => panic!("drain failed (host-side corruption?): {e:?}"),
            }
        }
    }

    /// Run the vCPU until `stop` matches (checked after every VM exit and
    /// every drain), the guest halts/shuts down, or `deadline` passes.
    pub fn run_until(
        &mut self,
        deadline: Duration,
        stop: impl FnMut(&Observed) -> bool,
    ) -> io::Result<StopReason> {
        // The VcpuExit borrows the VcpuFd's kvm_run mapping; moving the fd
        // into a local lets the handlers take &mut self without aliasing.
        let mut vcpu = self.vcpu.take().expect("vcpu present");
        let r = self.run_loop(&mut vcpu, deadline, stop);
        self.vcpu = Some(vcpu);
        r
    }

    fn run_loop(
        &mut self,
        vcpu: &mut VcpuFd,
        deadline: Duration,
        mut stop: impl FnMut(&Observed) -> bool,
    ) -> io::Result<StopReason> {
        let t0 = Instant::now();
        // A halted guest (HLT, in-kernel irqchip) blocks KVM_RUN with no
        // exits. A watchdog thread pthread_kills THIS thread with SIGALRM
        // every 50 ms (process-wide timers land on arbitrary threads — only
        // a targeted signal reliably interrupts the vcpu ioctl), so KVM_RUN
        // returns EINTR and the deadline + halt detection below run.
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let me = unsafe { libc::pthread_self() };
        let done = Arc::new(AtomicBool::new(false));
        let done2 = Arc::clone(&done);
        let watchdog = std::thread::spawn(move || {
            while !done2.load(Ordering::Relaxed) {
                // SAFETY: signaling a live thread (joined before run_loop
                // returns) with a handled, no-op signal.
                unsafe { libc::pthread_kill(me, libc::SIGALRM) };
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        let r = self.run_loop_inner(vcpu, t0, deadline, &mut stop);
        done.store(true, Ordering::Relaxed);
        let _ = watchdog.join();
        r
    }

    fn run_loop_inner(
        &mut self,
        vcpu: &mut VcpuFd,
        t0: Instant,
        deadline: Duration,
        stop: &mut impl FnMut(&Observed) -> bool,
    ) -> io::Result<StopReason> {
        loop {
            if t0.elapsed() > deadline {
                return Ok(StopReason::Timeout);
            }
            let exit = match vcpu.run() {
                Ok(exit) => exit,
                Err(e) if e.errno() == libc::EINTR => {
                    // Dead halt = powered off: HALTED with interrupts
                    // disabled (idle HLT keeps IF set and wakes on timer).
                    let halted = vcpu
                        .get_mp_state()
                        .map(|s| s.mp_state == kvm_bindings::KVM_MP_STATE_HALTED)
                        .unwrap_or(false);
                    if halted {
                        let if_clear = vcpu
                            .get_regs()
                            .map(|r| r.rflags & 0x200 == 0)
                            .unwrap_or(false);
                        if if_clear {
                            self.drain();
                            return Ok(StopReason::GuestStopped);
                        }
                    }
                    continue;
                }
                Err(e) => return Err(io::Error::from(e)),
            };
            match exit {
                VcpuExit::IoIn(port, data) => {
                    let v = pio::handle_in(self, port);
                    data.copy_from_slice(&v.to_le_bytes()[..data.len()]);
                }
                VcpuExit::IoOut(port, data) => {
                    let mut bytes = [0u8; 4];
                    bytes[..data.len().min(4)].copy_from_slice(&data[..data.len().min(4)]);
                    let v = u32::from_le_bytes(bytes);
                    pio::handle_out(self, port, v);
                }
                VcpuExit::MmioRead(addr, data) => {
                    // pv-blk gets the RAW slice (the agent's SECTOR/BUF_GPA
                    // accesses are single 8-byte ops — truncating them to
                    // the low 4 bytes would silently zero the high halves),
                    // and only when a test attached a device; otherwise the
                    // window keeps today's pvpad-path behavior (reads 0).
                    if pvblk::in_window(addr) && self.pio.pvblk.is_some() {
                        pvblk::pvblk_read(self, addr, data);
                    } else {
                        let v = pio::pvpad_read(self, addr);
                        let n = data.len().min(4);
                        data[..n].copy_from_slice(&v.to_le_bytes()[..n]);
                    }
                }
                VcpuExit::MmioWrite(addr, data) => {
                    if pvblk::in_window(addr) && self.pio.pvblk.is_some() {
                        pvblk::pvblk_write(self, addr, data);
                    } else {
                        let mut bytes = [0u8; 4];
                        let n = data.len().min(4);
                        bytes[..n].copy_from_slice(&data[..n]);
                        pio::pvpad_write(self, addr, u32::from_le_bytes(bytes));
                    }
                }
                VcpuExit::Hlt | VcpuExit::Shutdown => {
                    // Power-off path: without ACPI, RB_POWER_OFF halts; with
                    // reboot=t a panic triple-faults into Shutdown.
                    self.drain();
                    return Ok(StopReason::GuestStopped);
                }
                other => {
                    return Err(io::Error::other(format!("unhandled VM exit: {other:?}")));
                }
            }
            if stop(&self.observed) {
                return Ok(StopReason::Predicate);
            }
        }
    }

    /// Push a host command onto ring C (panics on a full ring — the tests
    /// never fill 16 KiB of commands).
    pub fn push_command(&mut self, cmd: &detguest_wire::Command) {
        let ch = self.channel.as_mut().expect("channel not attached yet");
        ch.push_command(cmd, &mut self.sink).expect("ring C full");
    }

    /// The serial output as lossy UTF-8 (diagnostics).
    pub fn serial_text(&self) -> String {
        String::from_utf8_lossy(&self.observed.serial).into_owned()
    }

    pub(crate) fn pio_state(&mut self) -> &mut PioState {
        &mut self.pio
    }

    pub(crate) fn mem(&self) -> MemSlot {
        self.mem
    }

    /// A `GuestMem` view of guest RAM for host-side reads that must NOT run
    /// the vCPU or touch the rings (e.g. `detguest-host` `read_manifest` /
    /// `read_region` from an integration test — the M4 "platform
    /// readability" discipline). Copies are only safe while the vCPU is
    /// stopped, i.e. between `run_until` calls (ARCHITECTURE.md §2).
    pub fn guest_mem_view(&self) -> MemSlot {
        self.mem
    }

    /// The pv-pad latch stub (schedule pad values for `poll_input` tests).
    pub fn pvpad(&mut self) -> &mut PvPad {
        &mut self.pio.pvpad
    }

    /// Attach a read-only pv-blk device model over `backing` at the
    /// hypervisor's game-device window (`pvblk::PVBLK_BASE`). Without this
    /// call the window behaves exactly as before (reads 0 / writes
    /// dropped), so existing tests stay bit-identical.
    pub fn attach_pv_blk(&mut self, backing: Vec<u8>) {
        self.pio.pvblk = Some(PvBlkModel::new(backing));
    }

    /// The attached pv-blk model, if any (test assertions on latched
    /// registers/STATUS), mirroring [`VmHarness::pvpad`].
    pub fn pv_blk(&mut self) -> Option<&mut PvBlkModel> {
        self.pio.pvblk.as_mut()
    }
}

/// SIGALRM handler that exists only to interrupt KVM_RUN (EINTR).
extern "C" fn vcpu_kick(_sig: libc::c_int) {}

/// NOTE: process-wide side effect — the harness owns SIGALRM for the whole
/// test process (no other SIGALRM users exist here; libtest does not use it).
fn install_vcpu_kick_handler() {
    // SAFETY: installing a no-op, non-SA_RESTART handler for SIGALRM.
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = vcpu_kick as *const () as usize;
        sa.sa_flags = 0; // crucially NOT SA_RESTART
        libc::sigaction(libc::SIGALRM, &sa, std::ptr::null_mut());
    }
}
