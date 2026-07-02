# 05 — KVM snapshot/restore/fork in the VM test harness

The 100× acceptance needs snapshot/fork and the harness
(`tests/vm/src/harness/`) has none — every existing test boots fresh. This
package adds it. It is independent of 01–04 and can be built in parallel.

Stack facts (review-verified): `kvm-ioctls = 0.23`, `kvm-bindings = 0.13`,
single vCPU, one memslot (slot 0, 128 MiB anonymous private mapping,
`mod.rs:136-152`), **in-kernel irqchip + PIT2** (`mod.rs:130-134`,
`KVM_PIT_SPEAKER_DUMMY`), `set_tss_address(0xfffb_d000)` before
`create_irq_chip` (`mod.rs:124` — required on Intel, must be repeated in
`from_snapshot`). `PioState` fields: `init_lo/init_hi/init_status/
inject_answer/pvpad` (`pio.rs:43-53`) — no serial buffer there (serial bytes
accumulate in `Observed.serial`); the harness has **no pv-blk device** (m9's
PvBlkClient targets the hypervisor's device map, not this harness). Guest
kernel has `CONFIG_HYPERVISOR_GUEST` unset — no kvmclock; time is PIT-polled
TSC calibration done at boot, so restoring TSC via the MSR list keeps the
guest's clocks coherent. All harness device state is plain Rust structs.

## Design

New `tests/vm/src/harness/snapshot.rs`:

```rust
pub struct VmSnapshot {
    memory: Vec<u8>,            // full 128 MiB guest RAM copy
    vcpu: VcpuState,            // see list below
    clock: kvm_clock_data,
    pit: kvm_pit_state2,
    pic_master: kvm_irqchip, pic_slave: kvm_irqchip, ioapic: kvm_irqchip,
    pio: PioState,              // detcall latches incl. channel GPA (init_lo/init_hi) + pv-pad
    host_channel: Option<HostChannelState>, // see below — REQUIRED for push_command in children
}
pub struct HostChannelState {
    gpa: u64,                   // from PioState init latches / CHANNEL_INIT
    producer_seqs: ProducerSeqs, // detguest_host::Channel::producer_seqs()
    interns: …,                 // the host intern map accumulated by drains
    pending_injects: …,         // whatever Channel exposes/needs re-seeding
}
```

**Host-side `Channel` state is load-bearing (review MAJOR).** A child built
by `from_snapshot` never re-executes the guest's CHANNEL_INIT PIO, so
`VmHarness.channel` would stay `None` and `push_command` panics
(`mod.rs:374-376`). Worse, `detguest-host` documents that ring C/I producer
seqs are NOT reconstructible and MUST round-trip via
`Channel::producer_seqs()` / `restore_producer_seqs()`
(`crates/detguest-host/src/channel.rs:95-133`) — a fresh attach would push
seq 0 at a guest consumer expecting a continuation. `from_snapshot` must:
`Channel::attach(child_memslot, gpa)` → `restore_producer_seqs(saved)` →
carry over the intern map (RegionUpdate/LogLine name resolution in children
depends on interns drained before the snapshot) and any pending-inject state
(check what `Channel` exposes; extend `detguest-host` with a getter only if
one is missing). Child `Observed`/drain baselines start fresh — acceptance
predicates must count **deltas** (e.g. FrameMark events drained since child
start), never absolute totals from the root's history.

`VcpuState`, captured in this order (KVM migration conventions):
`kvm_regs`, `kvm_sregs`, `kvm_xsave` (covers FPU — skip `kvm_fpu` if xsave is
taken), `kvm_xcrs`, MSRs, `kvm_lapic_state`, `kvm_vcpu_events`,
`kvm_debugregs` (kvm-ioctls names: `get_debug_regs`/`set_debug_regs`),
`kvm_mp_state`, TSC via the MSR list (`MSR_IA32_TSC`).

MSRs: `KVM_GET_MSR_INDEX_LIST` once (system fd). **Do not do one bulk
`get_msrs` over the whole list**: kvm-ioctls `get_msrs` returns the count read
*before the first failing index* and silently drops the tail — a single
unreadable MSR loses everything after it and the restore diverges undetected.
Capture per-MSR (or in bisecting chunks), asserting each read succeeds or the
index is on the checked-in skip-list; apply the same skip-list on `set_msrs`
and assert the set count matches. Populate the skip-list empirically on this
machine with a comment per entry (Firecracker's serializable-MSR list is the
precedent). Determinism: the skip-list is a checked-in constant, not runtime
discovery.

xsave: `get_xsave` is safe; `set_xsave` is `unsafe` with a
beyond-4096-bytes FAM hazard when dynamic XSTATE features are enabled. Guard:
`check_extension_int(Cap::Xsave2)` and assert the reported size ≤
`size_of::<kvm_xsave>()` before using plain get/set (tinyconfig guests won't
enable AMX, but assert it rather than assume).

### API

```rust
impl VmHarness {
    /// Capture full VM state. Only valid between run_until calls (vCPU stopped).
    pub fn snapshot(&mut self) -> io::Result<VmSnapshot>;
    /// Boot-free child construction: fresh VM/vCPU carrying `snap`'s state.
    pub fn from_snapshot(cfg: &VmConfig, snap: &VmSnapshot) -> io::Result<VmHarness>;
}
```

`from_snapshot`: `create_vm` → `set_tss_address` → `create_irqchip`/
`create_pit2` (same config as `new`) → create + register the memslot →
`memcpy` snapshot memory into the mapping → create vCPU → **set order**:
cpuid (same as `new`), sregs, regs, xsave, xcrs, msrs, lapic, vcpu_events,
debug_regs, mp_state → set_clock, set_pit2, set_irqchip ×3 → clone `PioState`
from the snapshot → re-attach the host `Channel` + `restore_producer_seqs` +
intern map (see above) → fresh `GuestIcount` (per-child counter starts at 0;
children compare frame counts, not absolute icounts).
`install_vcpu_kick_handler` is process-wide and idempotent — share it via the
common path rather than assuming `new()` ran first. Reuse/refactor the guts
of `VmHarness::new` (`mod.rs:118-237`) so setup code isn't duplicated —
extract the common "create vm + tss + irqchip + memslot + vcpu-plumbing"
path, with boot-loading vs state-restore as the variable part.

**pv-pad input scheduling (review MAJOR — capability gap).** `PvPad` today is
a static latch (`set_pad(port, value)`, read live by `sdk::poll_input`); the
acceptance needs per-frame input schedules. Add a schedule queue to `PvPad`
in this package: `schedule(frame: u32, port: u8, value: u32)`; harness
`apply_pvpad_write` already observes every guest FRAME_COUNTER write — on
frame N's write, latch the values scheduled for frame N+1. This makes "which
poll observes which value" exact and deterministic (poll_input during frame
K sees the value scheduled for frame K), which the acceptance's
input-history-hash recomputation and 2k/2k+1 determinism pairs depend on.
Unit-test the queue against a scripted FRAME_COUNTER sequence.

Root snapshot stays immutable; every child copies from it → children are
independent ("fork" = restore N times from one root). Memory: 128 MiB root
copy + one live child at a time; run children **sequentially** and drop each
child before the next (bounds RSS ~256–384 MiB).

Also snapshot-relevant: `run_loop`'s SIGALRM watchdog and epoll setup must be
re-armed per child — verify nothing in `run_until` assumes the harness booted
via `new`.

## Known gotchas to code around

- Take the snapshot only while the vCPU is out of `KVM_RUN` (harness is
  synchronous — already true between `run_until` calls). Immediately-exited
  MMIO/PIO in-flight state: snapshot only at a clean exit boundary (after the
  exit has been fully serviced), which `run_until`'s return guarantees —
  assert `!immediate_exit`-style invariants if the harness tracks any pending
  exit servicing.
- `kvm_vcpu_events`: zero `flags` fields that request injection of stale
  exceptions if the capture shows none pending (read the kvm-ioctls docs for
  `KVM_CAP_EXCEPTION_PAYLOAD` interactions; set nothing you didn't capture).
- LAPIC state is raw registers — get/set verbatim, no editing.
- TSC: with `KVM_GET_MSR_INDEX_LIST` including `MSR_IA32_TSC`, get/set moves
  the TSC value; do NOT also call `KVM_SET_TSC_KHZ` unless `new` does.
  The guest calibrated its clocks at boot; identical TSC restore keeps it
  consistent. (Guest uses PIT-polled TSC calibration per the pinned config —
  calibration already happened before the snapshot point.)
- kvmclock: guest is tinyconfig; check whether CONFIG_KVM_GUEST/kvmclock is
  even enabled (`image/kernel.config`). If not, `get_clock/set_clock` is
  harmless but keep it anyway for PIT coherence.

## Validation tests (land before the 100× acceptance)

In `tests/vm/tests/` (same gating pattern as `m2_acceptance.rs`: `#[ignore]`
+ `DETGUEST_VM_TESTS=1` + `/dev/kvm` assert):

1. `snapshot_restore_guest_still_runs`: boot m9-style workload to Ready, run
   to frame N (observe via drained `FrameMark`/meta), snapshot, build a child,
   run child 10 frames — child advances (frame counter grows), serial shows no
   guest panic/oops.
2. `snapshot_restore_is_deterministic`: two children from one root, identical
   scheduled pv-pad inputs, run 10 frames each — guest RAM regions (read via
   manifest) bit-identical between the children.
3. `root_snapshot_immutability`: after running a child, a fresh child from the
   same root still starts from identical region bytes.

These three de-risk the acceptance loop; debug restore fidelity here, not
inside the 100× run.

## Done when

Validation tests green locally via `DETGUEST_VM_TESTS=1 cargo test -p
detguest-vmtest -- --ignored --test-threads=1` (subset by name while
iterating). No behavior change to existing m2 tests.
