# Positive notes

These are correct things I specifically traced (because they are the easy-to-get-wrong
PID-1 / signal / ordering details) and confirmed hold.

### P1 — The PID-1 orphan-reaping loop is correct (no zombie accumulation)

`crates/detguest-agent/src/supervise.rs:329-340`. As init, the agent receives reparented
orphans; when they exit they SIGCHLD PID 1. The `reap()` loop calls
`waitpid(-1, WNOHANG|WUNTRACED)` and, on a pid that is *not* the supervised workload
(line 338), does `continue` — **not** `return`. The crucial detail: the `waitpid(-1)` call
that returned the orphan's pid has *already reaped its zombie*; the `continue` only skips
workload-specific event emission and loops to drain the next reapable child. Because the
loop continues on `pid > 0` and only exits on `pid <= 0`, all reapable children (workload
and orphans alike) are drained in one pass. This is the load-bearing init correctness
property and it is implemented correctly.

### P2 — SIGCONT / WCONTINUED interaction is benign (no stopped-flag desync)

`crates/detguest-agent/src/supervise.rs:329-350, 503-512`. `reap()` uses `WUNTRACED` (not
`WCONTINUED`). A SIGCONT'd child does generate a SIGCHLD (SA_NOCLDSTOP is not set on the
signalfd), but its "continued" status is not requested, so `waitpid` returns 0 and `reap`
simply no-ops — no zombie, no spurious event. The `stopped` flag is driven by the command
path (`forced_quiesce` → WIFSTOPPED sets it; `forced_resume` clears it after SIGCONT), not
by a WIFCONTINUED reap, so there is no desync between the flag and reality.

### P3 — Ring-A event ordering: WorkloadStarted precedes Ready

`crates/detguest-agent/src/runtime.rs:141-159` + `supervise.rs:271-278`.
`autostart_and_ready` calls `start_unit(unit)` (which emits `WorkloadStarted`) **before**
`emit_ready`, so on ring A the `WorkloadStarted` seq is strictly less than the `Ready` seq
— matching ARCHITECTURE §4 step 7 ("start unit … then emit Ready"). `WorkloadStarted` is
emitted without a doorbell (it is critical, so on a full ring it still doorbell-retries),
and the subsequent `Ready` doorbell flushes both — the orchestrator can rely on the
ordering.

### P4 — `emit_with_doorbell` and the ring-I relay get the release ordering right

`channel.rs:169-173` writes the record (via the safe ring API, which release-stores the
producer index internally) before ringing the doorbell. The manual ring-I relay
(`channel.rs:255-265`) copies the record bytes *then* does an explicit
`AtomicU32::store(..., Ordering::Release)` on the producer index — the record is published
before the index advance, exactly the SPSC discipline the SDK consumer's Acquire-load
pairs with. The pad-then-record byte accounting (`needed == tail + total` when a pad is
required) matches the bytes consumed.

### P5 — boot.toml validation is thorough and faults loudly per §7.2/§7.3

`crates/detguest-agent/src/boot.rs`. Dense-from-0 unique unit ids, absolute exec paths,
region-name length cap (56-byte manifest cap), duplicate-region detection, autostart
references an existing unit, `refwork-ctl` requires `game_dev`, integer range checks on
every numeric field — each maps to a §7.2 rule and produces a descriptive `BootFault`
string that `boot_fault` ships as the agent LogLine before power-off. The test matrix
(`faults_per_7_2`, `spec_example_shape_parses`) exercises each path. Good spec fidelity.

### P6 — `power_off()` refuses to reboot unless PID 1

`crates/detguest-agent/src/runtime.rs:51-60`. Guarding `reboot(RB_POWER_OFF)` behind
`std::process::id() == 1` means running the agent on a dev host (e.g., a stray invocation)
cannot reboot the developer's machine — it `sync`s and `exit(1)`s instead. Exactly the
right defensive instinct for code whose normal action is to power off the box.

### P7 — `translate.rs` decodes pagemap correctly and orders the failure checks well

`crates/detguest-agent/src/translate.rs:53-66`. Swapped is checked before present (a
swapped page also clears the present bit, and the more-specific error is the useful one);
a present entry with PFN 0 is correctly reported as `PfnHidden` (the no-CAP_SYS_ADMIN
case) rather than silently producing GPA 0. The `decode_entry` is pure and unit-tested,
keeping the unsafe-free file genuinely unsafe-free.

### P8 — `LineBuf` caps runaway lines at the wire limit

`crates/detguest-agent/src/supervise.rs:29-61`. A workload printing megabytes without a
newline cannot grow the agent's buffer unboundedly — `LineBuf` flushes at `MAX_LINE`
(== `MAX_LOG_MSG`), so nothing is silently clipped downstream and memory stays bounded.
Well tested (`linebuf_caps_runaway_lines`).

### P9 — Module-scoped `unsafe` discipline is real, not decorative

`lib.rs:15` is `#![deny(unsafe_code)]`; each permitted-unsafe module re-enables it with a
documented rationale (`pio`, `channel`, `supervise`, `runtime`), and `translate` genuinely
needs none. This is the IMPLEMENTATION-PLAN M6 policy enforced by the compiler rather than
by convention.
