# Package 01 — Non-Preemptive Harness Mode + Reproducer (build FIRST)

The request is explicit: build this before touching the agent. It is the fast
inner loop (~30 s vs a real-worker handoff) and the only thing that can prove
Fix A sufficient. Expected state at the end of this package: **the reproducer
tests exist and are red** (wedge before Ready) against the current agent, and
the wedge point matches the real-worker trail (post-registration, generation
6, no Ready) — *not* a TSC-calibration hang.

## 1. Harness: a no-timer-interrupt-delivery mode

File: `tests/vm/src/harness/mod.rs`.

Add to `VmConfig`:

```rust
/// Deliver timer interrupts (irqchip PIT → IRQ0). Default true. False
/// reproduces the deterministic worker's environment: interrupts exist as
/// machinery but nothing is armed, so the guest's jiffies never advance
/// (request phase3-boot-scheduling-deadlock).
pub timer_interrupts: bool,
```

`VmConfig::new` sets `timer_interrupts: true` (all existing tests unchanged).

**Mechanism: keep `create_irq_chip` + `create_pit2(KVM_PIT_SPEAKER_DUMMY)`
exactly as-is, then suppress delivery by replacing the GSI routing table.**
In `create_vm_core` (thread the flag in — it currently takes only
`mem_size`), when `timer_interrupts == false`, call
`vm.set_gsi_routing(...)` with a table containing the default IRQCHIP routes
for GSIs 1–23 **minus GSI 0 and GSI 2** (the PIT's PIC and IOAPIC delivery
pins). Rationale:

- Port 0x61/0x42 stay in-kernel (SPEAKER_DUMMY intact) → the kernel's
  PIT-polled TSC calibration cannot spin forever — the exact trap the
  request's §3 caveat warns about (`mod.rs:137-139` comment).
- The guest kernel cannot re-enable delivery (it can unmask its PIC IMR all
  it wants; the host routing table has no route).
- The snapshot/restore path (`from_snapshot` also calls `create_vm_core`)
  gets the same flag threaded through; snapshot tests keep `true`.

Pin model (get this right or the helper suppresses the wrong thing): KVM's
post-`KVM_CREATE_IRQCHIP` default table (`virt/kvm/…/irq_comm.c`
`default_routing`) maps GSI n → PIC pin n%8 **and** IOAPIC pin n — i.e.
GSI 0 → IOAPIC *pin 0*; the familiar GSI0→IOAPIC-pin-2 remap is a QEMU
userspace convention, not the kernel default. The in-kernel PIT injects via
`kvm_set_irq(…, gsi 0, …)`, so **omitting GSI 0 alone suppresses delivery**;
omit GSI 2 as well for belt-and-braces. Build the helper by replicating
`default_routing` verbatim (`kvm_irq_routing_entry` with
`KVM_IRQ_ROUTING_IRQCHIP`) and dropping every entry whose `gsi` ∈ {0, 2},
with a comment carrying this pin-model note. API check: `VmFd::set_gsi_routing`
exists in the pinned kvm-ioctls 0.23.0 (kvm-bindings 0.13.0 provides the
`KvmIrqRouting` FAM wrapper). A missing route fails **silently** — the PIT's
`pit_do_work` ignores `kvm_set_irq`'s return and in default reinject mode
simply stops re-arming — which is exactly the behavior we want: no VM error,
no delivery.

**Fallback if `set_gsi_routing` proves awkward** (kvm-ioctls API friction,
unexpectedly breaks the boot): keep the routing table complete but mask the
IOAPIC redirection entries + set the PIC IMR bit for IRQ0 via
`KVM_GET_IRQCHIP`/`KVM_SET_IRQCHIP` after guest kernel init would be
unreliable (the guest re-programs those) — so if routing fails, fall back to
**not creating the PIT at all** and instead adding a userspace port-0x61
refresh-toggle emulation in `pio.rs` (bit 4 flips on every read) plus 0x42
reads returning a free-running down-counter derived from a poll counter. That
is more code; try routing first. Record which mechanism landed.

**Cmdline for no-timer runs:** mirror the worker's forced flags. The callers
below use the harness default cmdline **plus**
`notsc tsc=unstable clocksource=jiffies noapictimer` (the
`dh-vmm/src/config.rs:92` set — already proven bootable *with* ticks via the
`BOOT_PROBE_CMDLINE` experiment, request `01-root-cause.md` §"Timerless
cmdline"). Provide `VmConfig::timerless_cmdline()` (or a const the tests
append) so the flag set lives in one place. The real worker boots without
any interrupts at all through region registration, so the kernel demonstrably
tolerates this combination up to the wedge point.

## 2. Probe hook (cheap diagnostics)

File: `tests/vm/tests/boot_probe.rs`. Add:

```rust
if std::env::var("BOOT_PROBE_NO_TIMER").as_deref() == Ok("1") {
    cfg.timer_interrupts = false;
}
```

(Alongside the existing `BOOT_PROBE_CMDLINE` override.) This gives a
serial+events dump of the wedge with zero test scaffolding — use it to
validate the wedge signature before writing the assertions below.

## 3. Reproducer tier 1 (in-repo, `DETGUEST_VM_TESTS=1`-gated)

New file: `tests/vm/tests/no_timer_boot.rs`. Reuse the artifact recipe from
`game_materialization.rs:87-136` verbatim (agent + `game-load-check` +
`boot.toml.game-mat`, pv-blk backed by the shared 32 KiB pattern — the full
production boot shape: materialize → control leg → region gate → Ready).
Factor the staging into a shared helper only if trivially possible; a copied
`artifacts()` block with a distinct stage dir name is acceptable (the m2/m4/
game-mat tests already each carry their own).

Test `no_timer_boot_reaches_and_holds_ready`:

1. `cfg.timer_interrupts = false`; cmdline = default + timerless flags.
2. `run_until` Ready predicate (60 s deadline), `drain()`, assert
   `StopReason::Predicate` with the serial text in the failure message.
3. Hold phase: run 3 more seconds asserting no `WorkloadExited` (mirror
   `refwork_ready_hold.rs` phases 1–2; skip the meta-frame check — the
   game-load-check workload's post-Ready behavior may differ; assert
   workload-alive only via absence of `WorkloadExited`/fault LogLines).

This is the **post-fix green criterion** at this tier. Pre-fix it should be
red. **Validate the red is the right red** (one-time, via the probe hook or
by temporarily printing serial on timeout): the run must show the agent
started, pv-blk materialization done, and RegionRegister events (or at
minimum the boot breadcrumbs through `boot: gameloaded` being *absent* while
serial shows kernel + agent alive) — i.e. wedged in the handshake, not hung
in early kernel boot / TSC calibration. If it instead hangs pre-userspace,
the delivery-suppression mechanism is wrong — fix that before proceeding.

**If tier 1 does NOT wedge pre-fix** (plausible: the tick dependency may be
in refwork's harness, not `game-load-check`), keep the test as a permanent
guard, note the observation for the resolution file, and treat tier 2 as the
sole red→green arbiter.

## 4. Reproducer tier 2 (authoritative; env-gated)

New test in a new file `tests/vm/tests/no_timer_refwork_ready.rs` (or a
second test fn inside `refwork_ready_hold.rs` — implementer's choice; keep
the gating identical): the `real_harness_reaches_and_holds_ready` body
(`refwork_ready_hold.rs:76-171`) but with `timer_interrupts = false` and the
timerless cmdline. Gate on `REFWORK_READY_INITRAMFS` like the original.
Extract the shared body into a helper parameterized on
`(timer_interrupts, cmdline)` rather than copying 90 lines — **with one
deliberate relaxation in the no-timer arm**: the phase-2 hold check asserts
only workload-alive (no `WorkloadExited`, no death LogLine), NOT the
meta-frame advance (`refwork_ready_hold.rs:139-142`). Frame pacing in the
refwork harness may itself be tick-dependent, and a frozen frame counter in
a no-tick guest would fail the green criterion for a reason outside this
request's scope — the same relaxation tier 1 already makes. Frame advance
stays asserted in the timer-ful arm; if frames *do* advance no-timer,
record that as a bonus observation in the resolution.

**The initramfs artifact is agent-version-coupled — plan the two runs
explicitly or you will mis-conclude:**

- **Red run (this package):** use the bridge's existing package-04 artifact
  (built from guest-sdk `914dbde` + reference-workload `aa69558`; if not on
  disk, build it from the reference-workload checkout at `aa69558`, whose
  `image/guest-sdk.lock` already pins `914dbde`, then
  `zstd -d` the `initramfs.cpio.zst`). Must wedge post-registration with no
  Ready, matching the real-worker trail. Capture the output for the
  resolution file.
- **Green run (after package 02):** the artifact contains the agent as
  PID 1, so the *old* initramfs can never go green. Rebuild it against the
  fixed agent: in a local reference-workload checkout, bump
  `image/guest-sdk.lock` to the local fix commit's full sha (**locally and
  uncommitted** — the committed bump is the bridge's action, per the
  handback; same convention as the game-materialization plan's acceptance)
  and rebuild the image. A tier-2 red *after* Fix A is only meaningful
  against this rebuilt initramfs — verify which artifact you're holding
  before concluding "Fix B needed".

## Exit criteria for this package

- Preemptive suites still green (`timer_interrupts` default true touches
  nothing).
- Tier-2 red confirmed (and tier-1 red/green state recorded either way).
- Wedge signature validated as scheduling-shaped, not TSC-shaped.
