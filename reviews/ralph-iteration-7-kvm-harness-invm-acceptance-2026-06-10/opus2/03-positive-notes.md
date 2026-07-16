# Positive notes

### P-1 — The watchdog signal dance is correct AND honestly commented

`tests/vm/src/harness/mod.rs:283-297`. The hard problem (a halted in-kernel-irqchip
guest blocks `KVM_RUN` with no exits, and a process-wide interval timer lands on an
arbitrary thread) is solved the right way: a per-`run_loop` watchdog thread that
`pthread_kill`s the *specific* vCPU-owning thread with a no-op SIGALRM to force
`EINTR`. The SAFETY comment's claim that the target is "joined before run_loop
returns" holds up under precise analysis — the killed thread is the caller, which
outlives the watchdog's join. This is the kind of code that is usually subtly wrong;
here it is right and the reasoning is written down.

### P-2 — detcall handler delegates to the real `detguest-host` crate, not a reimplementation

`tests/vm/src/harness/pio.rs` (`attach_channel`, the INJECT path, `drain`). The
harness exercises the actual production host crate (`Channel::attach`,
`drain_events`, `InjectResponder::answer`) rather than a parallel mock, so the M2
gate tests the real wire/drain/inject code. The §5 port→behavior mapping is a faithful
1:1 with the API.md table (all seven ports verified, including RAZ/WI fallthrough and
the "drain ring W before answering INJECT" sequencing rule at `pio.rs:113-114`).

### P-3 — The PID1 stdio bootstrap ordering is exactly right and the "why" is documented

`crates/detguest-agent/src/runtime.rs:44-78`. The insight that PID 1 starts with no
valid fds 0–2 (no `/dev/console` node in the initramfs), so *any* print before stdio
setup self-panics with exit 101 and masks the real error, is precisely the trap that
makes early-boot agents miserable to debug. Mounting devtmpfs first, binding fds 0–2
to `/dev/console` (with `/dev/null` fallback) before anything that can print, and only
then mounting proc/sys, is the correct sequence — and the failure path
(`setup_stdio()` best-effort even when devtmpfs mount fails) is handled too.

### P-4 — `emergency_serial` / `console_log` are genuinely no-panic, fd-free diagnostics

`crates/detguest-agent/src/pio.rs:76-98` and `runtime.rs:21-30`. Last-resort boot
diagnostics that need no filesystem and no fds — raw `OUT` to the 8250 THR, guarded by
an idempotent IOPL raise, with unreportable errors deliberately swallowed. Paired with
the `write(2)`-to-fd-2 path in `console_log`, the agent now has a diagnostics channel
that survives the exact failure modes (no console node, closed fds) that previously
produced a bare exit 101. The replacement of the four `eprintln!` death-path calls
(`runtime.rs:205-228`) with `console_log` is the right call.

### P-5 — `CONFIG_X86_IOPL_IOPERM` fix carries its scar story in the config comment

`image/kernel.config:48-52`. "tinyconfig disables the syscall entirely — without this
the agent's first OUT is a GPF (found the hard way on first in-VM boot)". This is
exactly how config knobs should be documented: the symptom, the cause, and the cost of
omission, so the next person bumping tinyconfig doesn't silently drop it. (This same
discipline is what motivates finding I-1/I-2 — capture the *next* such requirement
before it bites.)

### P-6 — icount measurement is set up with the right perf semantics for cross-boot comparability

`tests/vm/src/harness/icount.rs`. `perf_event_open` with `pid=0` (this thread),
`cpu=-1` (any CPU), `exclude_host` set → counts only guest (VMX non-root) retired
instructions on the vCPU thread, excluding the harness's own host-side work. Each
`VmHarness` opens its own fd and enables it before the run, so the 10 cross-boot reads
are apples-to-apples absolute counts from a fresh per-boot counter. The honest gating
of the strict bit-identical assert behind `DETGUEST_STRICT_ICOUNT=1` — because this
minimal harness has a real-time PIT and lacks the deterministic timer-interrupt
delivery the strict gate needs — is the correct call and is documented at
`m2_acceptance.rs:13-19,249-255`.
