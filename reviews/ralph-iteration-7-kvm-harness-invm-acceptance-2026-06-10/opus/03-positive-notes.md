# Positive Notes

### P-1 — The harness drives the *real* `detguest-host` crate, not a reimplementation

**File:** `tests/vm/src/harness/pio.rs:9,118-124,144-149`

The detcall handler delegates every channel mutation to `detguest_host::{Channel, InjectResponder, RecordingSink}` and maps attach failures through the host crate's own
`AttachError::init_status()`. This means the measurement gate exercises the production
host-side state machine end to end, rather than a stub that could drift from it. Exactly the
right fidelity decision for a milestone-gating harness.

### P-2 — GDT / long-mode bring-up is bit-exact and self-consistent

**File:** `tests/vm/src/harness/x86.rs:23-47,65-66`

I verified the `gdt_entry` packing against the `seg()` flag extraction independently:
`0xa09b` (code) → access `0x9b`, granularity nibble `0xa` → `type=0xb, s=1, l=1, g=1, db=0`;
`0xc093` (data) → access `0x93`, nibble `0xc` → `type=3, s=1, db=1, g=1`. The bit positions
in `seg()` (`>>14` db, `>>13` l, `>>15` g, `&0xf` type, `>>4` s) line up exactly with where
`gdt_entry` places the access byte and granularity nibble. Page-table flags (`0x83` =
P|RW|PS for the 2 MiB leaves, `0x3` = P|RW for PML4/PDPT), `CR0 = PE|ET|PG`, `CR4 = PAE`,
`EFER = LME|LMA`, `CR3 = PML4` — all correct. The 1 GiB identity map covers the 128 MiB RAM;
the pv-pad MMIO at 3.25 GiB is intentionally *not* covered (the agent maps it via `/dev/mem`
under the kernel's own page tables at M3 — the harness page tables only matter until the
kernel builds its own), which the harness reasoning gets right.

### P-3 — `KVM_PIT_SPEAKER_DUMMY` rationale is correct and load-bearing

**File:** `tests/vm/src/harness/mod.rs:128-134`

Without `SPEAKER_DUMMY`, port `0x61` (the PIT refresh toggle the kernel's slow-path TSC
calibration polls) exits to userspace; a constant userspace answer never toggles the refresh
bit and the calibration loop hangs. Enabling the dummy keeps the toggle in-kernel. This is a
subtle, genuinely flaky-boot-preventing detail and the comment explains it precisely.

### P-4 — VM setup ordering is correct (TSS/irqchip/PIT before vCPU)

**File:** `tests/vm/src/harness/mod.rs:124-134,208`

`set_tss_address` → `create_irq_chip` → `create_pit2` are all issued before `create_vcpu(0)`,
which is the KVM-required order (the in-kernel irqchip and PIT must exist before the vCPU is
created so the vCPU picks up the LAPIC). `set_tss_address(0xfffb_d000)` sits in the
conventional sub-4 GiB hole, clear of the 128 MiB RAM region.

### P-5 — Detcall discipline matches ARCHITECTURE.md §2 exactly: drain-before-INJECT-answer, pause-bounded host writes

**File:** `tests/vm/src/harness/pio.rs:111-124,170-180`; `tests/vm/src/harness/memslot.rs:1-8`

`INJECT` drains ring W *first* (so the matching `InjectQuery` released before the `OUT` is
visible) before `responder.answer()` — the §5 sequencing rule. The FRAME_COUNTER MMIO write
drains inside the same exit (the `FrameMark` record preceding the write is guaranteed
visible). Every host channel touch happens through `MemSlot`, whose module doc nails the
load-bearing invariant: the host only reads/writes channel memory while the vCPU is paused in
an exit — which is precisely when these methods are reachable.

### P-6 — Halt/power-off detection is principled, not a heuristic guess

**File:** `tests/vm/src/harness/mod.rs:313-329`

Distinguishing a *dead* halt (powered off) from an *idle* halt is done by `MP_STATE_HALTED`
**and** `RFLAGS.IF == 0`: an idle `hlt` in the kernel keeps IF set and wakes on the next
timer, whereas the power-off path halts with interrupts masked. This avoids the false-positive
a bare `HALTED` check would produce. The watchdog SIGALRM (non-`SA_RESTART`, no-op handler)
to force `EINTR` out of a blocking `KVM_RUN` is the standard correct technique, and the
`done`-flag + `join()` ordering closes the signal-after-exit window.

### P-7 — Honest, well-documented deferral of the strict bit-identical icount gate

**File:** `tests/vm/tests/m2_acceptance.rs:13-19,249-256`

The doc comment and the `DETGUEST_STRICT_ICOUNT` env gate are exemplary engineering honesty:
the harness has a *real-time* KVM PIT, so bit-identical icounts would require deterministic
timer-interrupt delivery (determinism-hypervisor M2/M3 machinery this minimal harness does
not have). Rather than assert something it cannot guarantee, the default run **records and
reports the spread** and hard-asserts equality only behind an explicit opt-in flag. This is
the right way to ship a measurement harness ahead of the machinery that makes the strict
property hold.

### P-8 — The agent stdio/console fixes correctly sequence around the "no /dev/console yet" trap

**File:** `crates/detguest-agent/src/runtime.rs:44-79`; `crates/detguest-agent/src/pio.rs:84-98`

Mounting devtmpfs *first*, immediately binding fds 0–2 to `/dev/console` (with `/dev/null`
fallback) **before** any code that can `println!`, directly fixes the exit-101 masking
(println to a closed fd panics, hiding the real error). `console_log` uses raw `libc::write`
with the result ignored (panic-proof by construction), and `emergency_serial` guards every
OUT behind `raise_iopl()` so an un-privileged OUT can't itself GPF — a strictly better
failure mode than losing the message. `CONFIG_X86_IOPL_IOPERM=y` in both `kernel.config` and
the `build.sh` `REQUIRED_SET` assert list closes the loop so a future config regression fails
the image build loudly rather than at first in-VM boot.
