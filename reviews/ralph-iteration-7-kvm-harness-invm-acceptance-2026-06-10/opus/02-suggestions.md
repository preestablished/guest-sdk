# Suggestions (non-blocking)

### S-1 — INIT_GO wrong-size maps to status 1 ("bad GPA"), but a size mismatch is not a GPA problem

**File:** `tests/vm/src/harness/pio.rs:140-141`

```rust
if size_pages != CHANNEL_SIZE_PAGES {
    return 1; // bad GPA/size commit
}
```

API.md §5 defines status `1 = bad GPA`, `2 = bad magic/version`. A wrong *size* commit is
neither, strictly. The host crate's own `AttachError` has no "bad size" variant (size never
reaches `Channel::attach` — the harness validates it up-front), so there is no canonical
code to return. Status 1 is defensible (a wrong-size commit means the host can't trust the
channel-region GPA span), and the inline comment already hedges "bad GPA/size". This is fine
as-is; flagging only so the deviation is a conscious one. If the hypervisor ever defines a
distinct size-error code, mirror it here. No change required.

### S-2 — `boot_to_ready` comment says count is read "inside the Ready doorbell window"; it is actually the whole-boot total

**File:** `tests/vm/tests/m2_acceptance.rs:153-155,162` and `icount.rs:48-52`

`icount.enable()` is called right after `VmHarness::new()` (kernel loaded, vCPU not yet run)
and `read()` after the `Ready` predicate stops the loop, so the counter spans **first guest
instruction → Ready**, not a narrow doorbell window. The measurement is well-defined and
correct for the across-boots spread; only the prose is imprecise. Suggest: "the retired-
instruction count from guest entry through the Ready doorbell." Each boot allocates a fresh
`GuestIcount`, so there's correctly no cross-boot accumulation.

### S-3 — Stack at `rsp = 0x8ff0` sits one page above `boot_params` (0x7000) with no guard

**File:** `tests/vm/src/harness/mod.rs:217` (rsp) vs `:34` (BOOT_PARAMS_ADDR 0x7000)

The early stack grows down from `0x8ff0`; `boot_params` occupies `0x7000-0x8000` (one page).
There is exactly one 4 KiB page (`0x8000-0x8ff0`) of headroom before the stack would clobber
`boot_params`. The 64-bit kernel head switches to its own `init` stack within the first
handful of instructions (before `rsi`/boot_params is consumed), so this is safe in practice
and matches the standard rust-vmm recipe — but it is undocumented tightness. A one-line
comment ("kernel head switches stacks before this can reach boot_params") would prevent a
future reader from "tidying" the layout into a collision.

### S-4 — `initrd_addr` underflow is unreachable only because `mem_size` is hard-coded

**File:** `tests/vm/src/harness/mod.rs:169`

```rust
let initrd_addr = (cfg.mem_size as u64 - initramfs_bytes.len() as u64) & !((2 << 20) - 1);
```

The `2 << 20` mask (= `0x200000`, a correct 2 MiB align-down — verified) is fine, and at the
fixed 128 MiB the initrd lands at 126 MiB, clear of the 1 MiB kernel. But if `mem_size` ever
shrinks below the initramfs size (a future small-guest config), the `u64` subtraction
underflows to a huge address. Cheap guard:

```rust
let initrd_addr = (cfg.mem_size as u64)
    .checked_sub(initramfs_bytes.len() as u64)
    .map(|top| top & !((2 << 20) - 1))
    .filter(|a| *a >= HIMEM_START + /* a kernel-size guard */ 0x80_0000)
    .ok_or_else(|| io::Error::other("initramfs too large for guest RAM"))?;
```

A plain `.expect("initramfs fits in guest RAM")` is enough — the point is to fail loudly
rather than write to a wrapped GPA.

### S-5 — Watchdog `pthread_kill` after the target thread could in principle exit (currently impossible, worth a belt-and-braces note)

**File:** `tests/vm/src/harness/mod.rs:283-297`

The design is sound: `done` is set and the watchdog `join()`-ed *before* `run_loop` returns,
and `me = pthread_self()` is the vcpu thread, which is alive for the whole of `run_loop`. So
`pthread_kill(me, SIGALRM)` can never target a dead TID here. The only residual is the
classic `pthread_kill`-after-`pthread_exit` UB *in general* — not reachable in this control
flow, but a future refactor that moved the vcpu run onto a *different* thread than the one
that spawned the watchdog would reintroduce it. The `// SAFETY: ... joined before run_loop
returns` comment already captures the invariant; consider adding `debug_assert!` that the
join completes, or leave as-is. No functional change needed.

### S-6 — Serial OUT decode in `handle_out` is correct but slightly redundant

**File:** `tests/vm/src/harness/pio.rs:127`

```rust
p if (SERIAL_BASE..SERIAL_END).contains(&p) && p == SERIAL_BASE => {
```

`(SERIAL_BASE..SERIAL_END).contains(&p) && p == SERIAL_BASE` reduces to `p == SERIAL_BASE`
(the THR). The range check is dead. Harmless, but `p if p == SERIAL_BASE =>` (or a plain
match arm `SERIAL_BASE =>`) reads cleaner and makes it obvious that writes to the other
8250 registers (IER/FCR/LCR/MCR) are intentionally dropped (WI), which is the correct 8250
sink behavior. Worth a one-word comment that non-THR serial writes are deliberately ignored.
