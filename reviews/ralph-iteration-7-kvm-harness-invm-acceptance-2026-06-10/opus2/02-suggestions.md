# Suggestions (non-blocking)

### S-1 — `install_vcpu_kick_handler` is a process-wide SIGALRM install from a constructor; document the contract

- **Where:** `tests/vm/src/harness/mod.rs:222` (called from `VmHarness::new`) and
  `mod.rs:401-409`.
- **Analysis (signal globals — verified, not a bug):**
  - The handler is **idempotent**: each call re-installs the identical no-op,
    `sa_flags = 0` (no `SA_RESTART`) handler. Repeated installs across the 10
    boots in `ready_icount_across_ten_boots` are harmless.
  - It **permanently overrides** any SIGALRM disposition the embedding test
    process had. No other code in this repo uses SIGALRM / `alarm()` /
    `setitimer` (grepped clean), and the Rust libtest harness implements
    per-test timeouts with a background polling thread, not SIGALRM — so there is
    no conflict today. The risk is purely future: if another test in the
    `detguest-vmtest` binary ever relies on `alarm()`/SIGALRM, this silently
    eats it.
  - The **watchdog race is genuinely safe**, as the inline comment claims, and I
    confirm the precise argument: `me = pthread_self()` captures the
    `run_until` *caller* thread (the vCPU runs on the same thread). The caller
    thread does not exit — it stores `done=true`, then `join()`s, then returns up
    the stack. The watchdog can fire `pthread_kill(me, SIGALRM)` in the window
    between observing `done==false` and the kill even after `done` was stored, but
    `me` is unconditionally alive throughout the watchdog's lifetime, and `join()`
    blocks until the watchdog exits its loop — so no kill ever targets a
    dead/joined `pthread_t`. No `ESRCH`, no UB. The only cost is that `join()` can
    block up to ~50 ms (one sleep interval) at the end of each `run_until`.
- **Suggestion:** Either move the install to a `std::sync::Once` (make the
  process-wide side effect explicit and single-shot), or add a one-line note to
  the harness module docs that the harness owns SIGALRM process-wide for the
  lifetime of the test binary, so future test authors don't fight it. No
  correctness change required.

### S-2 — `INIT_GO` IN before any commit returns `u32::MAX`, which is outside the §5 status enum

- **Where:** `tests/vm/src/harness/pio.rs:60` (`init_status: u32::MAX`) read at
  `pio.rs:82`.
- API.md §5 defines INIT_GO IN as "status: 0/1/2/3"; reading before a commit is
  undefined in the spec, and the agent's `channel_init` (`crates/detguest-agent/src/pio.rs:64-69`)
  always commits before reading, so the sentinel is never observed in practice.
  `u32::MAX` is a fine "never committed" marker. Consider a one-line comment that
  this value is intentionally out-of-enum (so a reader doesn't think the harness
  can return a bogus status to the guest), or assert in a unit test that the
  pre-commit readback is never consumed.

### S-3 — print-lines test does not assert `LogLine` level mapping

- **Where:** `tests/vm/tests/m2_acceptance.rs:340-366`.
- The test asserts per-stream message content and order (stdout=stream 1,
  stderr=stream 2) but ignores the `level` field. The suite comment (lines 22-23)
  promises "correct stream/level framing". The drain ordering is sound — `LogLine`
  records are FIFO on ring A and the `WorkloadExited` doorbell drains everything
  before it in the same `drain()` (host crate returns records in (ring, seq)
  order), so earlier `LogLine`s are guaranteed present. Since the framing is
  already there, cheaply assert the level mapping too (e.g. stdout→level 2 /
  stderr→level 0, per whatever the agent emits) so a future level-mapping
  regression is caught. `log_mask = 0x1F` already admits both streams.

### S-4 — `ready_icount_across_ten_boots` spread is only visible with `--nocapture`

- **Where:** `tests/vm/tests/m2_acceptance.rs:236-256`.
- The per-boot icount and the min/max/delta spread are `eprintln!`'d, so they are
  swallowed unless the in-VM tier runs with `--nocapture`. The CI invocation
  (`ci.yaml`) does not pass `--nocapture`, so the headline measurement of this
  gate (the spread under a real-time PIT) is invisible in CI logs. Consider adding
  `--nocapture` to the in-VM tier command, or write the spread to a step summary,
  so the "record always" intent (the non-strict default) actually produces a
  durable record.

### S-5 — `Hello.vnanos` boot criterion measures from `timekeeping_init`, not power-on

- **Where:** `tests/vm/tests/m2_acceptance.rs:197-203`; source
  `crates/detguest-agent/src/supervise.rs:87-95` (`CLOCK_MONOTONIC_RAW`).
- `CLOCK_MONOTONIC_RAW` is zeroed at kernel `timekeeping_init`, which runs very
  early but *after* firmware/decompression/early arch setup. So `vnanos` at Hello
  is "kernel-timekeeping-init → agent Hello", a slight under-count of true
  power-on-to-agent time. This is the right deterministic quantity for the gate
  (it's what the platform's virtual clock measures) and the `< 1 s` budget has
  ample margin, so it is correct as a *proxy*. The test comment calls it "guest
  time from boot" — accurate to within the pre-timekeeping window. Suggestion:
  soften the comment to "guest time from kernel timekeeping init (≈boot)" so the
  small offset is not mistaken for true wall-from-power-on in a tighter future
  budget.
