# Positive Notes

### P1 — Fork/exec is allocation-clean and uses `_exit` (`supervise.rs:100-163`)

Every `CString` (exec path, argv entries, the `DETGUEST_CHANNEL_FD=…` env) is built
**before** `libc::fork()` (lines 111-120), so the child between fork and exec calls only
async-signal-safe libc functions (`dup2`, `fcntl`, `setrlimit`, `execve`) and never
allocates — sidestepping the classic fork-in-a-Rust-runtime heap-deadlock hazard. The
failure exit is `libc::_exit(127)`, not `exit`, so no atexit/destructor runs in the child.
This is textbook-correct and exactly the hard part the prompt asked to scrutinize.

### P2 — Partial-line flush ordering at reap matches the M2 LogLine acceptance (`supervise.rs:329-394`)

On workload exit the reap does, in order: final non-blocking `drain_pipe(OUT/ERR)` →
`take()` the workload → `LineBuf::finish` to flush any unterminated trailing line →
`emit_lines` for those → only then `emit_with_doorbell(WorkloadExited)` (critical). So
every byte the workload wrote is framed into LogLine events *before* the terminal
WorkloadExited record, which is precisely the framing contract the M2 acceptance leg
(`print-lines` exits with fixed stdout/stderr lines) depends on.

### P3 — The boot sequence is a faithful transcription of ARCHITECTURE.md §4 (`runtime.rs`)

`run()` → `mount_all` → `bring_up_channel` (alloc → `raise_iopl` → `ident_ok` → pagemap →
`channel_init` → `set_agent_ready` → Hello) → parse boot.toml → `Supervisor::new` →
`autostart_and_ready` → `sup.run()`. The ordering of iopl-before-first-detcall,
agent_ready-before-Hello, and Hello-before-Ready all match §4 steps 5/6/7 and the §4.1
READY contract. Pre-channel failures (`mount_all`, `bring_up_channel`,
`Supervisor::new`) correctly use `eprintln` + `power_off` because no LogLine channel
exists yet, while post-channel parse/validation failures route through `boot_fault`
(LogLine + doorbell + power-off) per §7.3 — the right witness for each phase.

### P4 — `power_off` guarded on `PID == 1` (`runtime.rs:51-60`)

Refusing to call `reboot(2)` unless `std::process::id() == 1` means running the agent on a
dev host (e.g. via `--check` or a stray invocation) can never reboot the developer's
machine. `sync()` runs unconditionally first, then the PID gate, then a loud
`process::exit(1)`. Small, defensive, exactly right.

### P5 — `boot.rs` §7.2 coverage is complete and well-tested

Every §7.2 rule is enforced with a precise fault message: required `boot_toml_version` +
unknown-major rejection, dense-from-0/unique ids, absolute `exec`, default empty
`args`, default `log_mask = 0x1F`, `expected_region` name ≤ 56-byte cap, duplicate region
names (via `BTreeSet`), `refwork-ctl ⇒ game_dev` requirement, and autostart→nonexistent
unit. The `faults_per_7_2` test exercises each branch, and `spec_example_shape_parses`
round-trips the literal API.md §7.1 example. Using `BTreeSet` (not `HashMap`) for dup
detection also respects §7 rule 2 (no default-hasher randomness in-guest).

### P6 — `init_at` zero-init reasoning is sound and documented (`channel.rs:42-118`)

The SAFETY comment correctly reasons that `ftruncate` + `MAP_SHARED` of a fresh hugetlbfs
file yields zeroed bytes, so the header/manifest init can assume a zero background (drop
counters and reserved bytes are left untouched and stay zero). The header write covers
exactly `[0, OFF_RESERVED)` and the manifest is initialized via the wire crate's
`init_manifest`, with all raw-pointer writes staying in-bounds of `CHANNEL_SIZE`. The
`init_at` constructor is correctly an `unsafe fn` with a documented caller contract, and
the tests drive it over a leaked zeroed buffer — the right host-test substitute.

### P7 — WUNTRACED/SIGCONT reap reasoning is correct (`supervise.rs:329-350,504-512`)

`waitpid(-1, …, WNOHANG | WUNTRACED)` catches the FORCED-quiesce `SIGSTOP` and emits
`QuiesceReady` on ring A (matching API.md §6 FORCED row). Because `WCONTINUED` is **not**
requested, the later `SIGCONT` from `forced_resume` does not generate a spurious
waitpid/reap that would confuse the loop into a bogus `WorkloadExited` — the stop and the
true exit are the only two events the reap can observe. The `stopped` flag gates
`forced_resume` so a SIGCONT is only sent to an actually-stopped child.

### P8 — `LineBuf` runaway-line cap (`supervise.rs:29-61`)

Splitting at `MAX_LINE` (= `MAX_LOG_MSG`) means a workload that writes megabytes without a
newline cannot grow the buffer unboundedly, and nothing is silently clipped downstream
(the split boundary equals the wire cap). The three unit tests cover newline framing,
runaway capping, and trailing-partial flush.

### P9 — Doorbell-retry termination argument is sound and spec-anchored (`channel.rs:144-165`)

The critical-event `emit` doorbell-retry "infinite" loop is bounded by the §3-rule-4
guarantee that the doorbell exit *forces* the host to drain and bump the consumer index,
freeing space — and because record length is capped below ring size, a single record
always eventually fits. The comment cites this correctly; it is a deterministic
guest-initiated wait, replayed at identical icounts.
