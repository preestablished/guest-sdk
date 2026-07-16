# Critical & Important findings

## CRITICAL

### C1 — `boot::parse` consumes guest entropy via the `toml` crate's random-seeded hasher (§7 rule 2, P0)

**File:** `crates/detguest-agent/src/boot.rs:82` (`text.parse::<toml::Value>()`), root
cause in the dependency `crates/detguest-agent/Cargo.toml:15` (`toml = "0.8"`).

ARCHITECTURE.md §7 rule 2 is explicit and classifies violations as P0:

> **No entropy consumption.** The SDK and agent never read `/dev/(u)random`,
> `getrandom(2)`, or `RDRAND`/`RDSEED` … **No `HashMap` default hasher (it seeds from
> randomness)** — use `FxHashMap`/`BTreeMap` everywhere in-guest.

`toml = "0.8"` (resolved `toml 0.8.23`) parses through `toml_edit 0.22.27`, whose
document model is `pub(crate) type KeyValuePairs = IndexMap<Key, Item>`
(`toml_edit-0.22.27/src/table.rs:522`) — an `indexmap::IndexMap` with the **default
`S = RandomState`** hasher, backed by `hashbrown` (the dep chain `toml → toml_edit →
indexmap → hashbrown` is in `Cargo.lock`). Constructing/inserting into that map seeds a
random hasher, which calls `getrandom(2)` in-guest.

I verified this empirically. Stracing a minimal `toml::Value::parse(...)` binary against
an empty-Rust-binary baseline shows **one extra `getrandom` syscall**:

```
# toml-parsing binary
getrandom(..., 8,  GRND_NONBLOCK) = 8     # std RandomState (also in baseline)
getrandom(..., 16, GRND_NONBLOCK) = 16    # std (also in baseline)
getrandom(..., 16, GRND_INSECURE) = 16    # <-- ONLY in the toml binary (hashbrown/ahash seed)
# empty baseline binary: only the first two
```

Why this matters even though the agent only does key *lookups* (so the parsed
`BootManifest` is still deterministic): the violation is the **act of calling
`getrandom` in-guest**, which the spec forbits as P0 regardless of whether the result
leaks into the wire. Two concrete in-VM hazards the host tests cannot see:

1. **Entropy consumption is itself the violation.** It will trip any audit/assertion the
   determinism harness adds for §7 rule 2, and it muddies the icount-reproducibility
   story (the syscall and its replay handling are part of the input log).
2. **Early-boot fragility:** the agent runs as PID 1 very early, before the kernel CSPRNG
   may be fully initialized. `getrandom(GRND_NONBLOCK)` returns `EAGAIN` when the pool is
   uninitialized; the hasher then falls back to a weaker seed, but this is a boot-timing-
   dependent code path that only manifests inside the VM and is exactly the kind of
   "in-VM failure mode host tests can't catch" this milestone is supposed to be solid on.

`toml::map::Map` (the public `Value::Table`) is `BTreeMap`-backed by default and is fine;
the entropy comes entirely from `toml_edit`'s internal parse document, so swapping the
public type does **not** help — the parser itself is the source.

**Fix options (pick one):**

- **Preferred:** drop the `toml` dependency for boot.toml and hand-write a tiny parser
  for the §7 subset (the format is repo-owned and small: `boot_toml_version`, `[[unit]]`,
  `[unit.control]`, `[autostart]`, `[[expected_region]]` — all flat tables of integers
  and strings). This removes the only general-purpose parser in-guest and the §7
  liability with it.
- If `toml` is kept, confirm the determinism harness's §7 enforcement explicitly
  whitelists this one boot-time `getrandom` *and* document that the boot.toml parse
  happens before the READY snapshot point (it does — `boot.rs` parse is between Hello and
  Ready in `runtime::run`), so it is outside the deterministic-replay window. That is a
  weaker position and still leaves the early-boot `EAGAIN` fallback path; I would not
  rely on it.

---

### C2 — `start_unit` overwrites a running workload: leaks the old process, its pipe fds, and its epoll registrations, and emits a lying `WorkloadStarted`

**File:** `crates/detguest-agent/src/supervise.rs:251-280` (`start_unit`), reachable via
`crates/detguest-agent/src/commands.rs:17-24` (`StartWorkload`).

`start_unit` unconditionally does `self.workload = Some(w)` (line 270). `Workload` has
**no `Drop` impl**, so if a workload is already running when a second
`StartWorkload{unit}` arrives on ring C:

- the previous child process is **not** killed or reaped — it keeps running as an
  unsupervised child of PID 1 (still a child, so its eventual SIGCHLD will hit `reap()`,
  but `reap()` only matches `self.workload.pid` (line 338) which is now the *new* pid, so
  the old child is reaped as an "unrelated child" and **no `WorkloadExited` is emitted
  for it** — the host's workload model silently desynchronizes);
- the old `stdout`/`stderr` pipe read-ends (raw fds) **leak** and remain registered in
  epoll under `TOK_OUT`/`TOK_ERR`. When those old fds later HUP, `drain_pipe(TOK_OUT)`
  reads from `self.workload.stdout` — the *new* fd — so the old fds can never be drained
  or `EPOLL_CTL_DEL`'d. They will report `EPOLLHUP` on every `epoll_wait` forever (see I3,
  which this turns from transient into permanent);
- a `WorkloadStarted{new_pid}` event is emitted while the old workload is still alive —
  the host now believes one workload exists when two do.

The struct is documented "the (single, v1) supervised workload" and API.md §6 gives no
two-workload semantics, so the correct behavior is to **refuse** when one is already
running (or to kill+reap the existing one first). Silently leaking is the worst option.

**Fix (refuse):**

```rust
pub fn start_unit(&mut self, unit_id: u32) -> io::Result<()> {
    if self.workload.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "a workload is already running (v1 supervises one)",
        ));
    }
    // ... existing body ...
}
```

(The autostart caller starts from `workload == None`, so this is purely a guard on the
ring-C path. If the intended semantics are "replace", do an explicit
`immediate_shutdown()`-style kill+reap+`EPOLL_CTL_DEL`+close of the existing workload
first — but that should be deliberate, not an accidental drop.)

---

## IMPORTANT

### I1 — `spawn()` leaks pipe fds on every error path after the first `pipe2`

**File:** `crates/detguest-agent/src/supervise.rs:106-124`.

The `||` short-circuit at lines 106-109 means a failed *second* `pipe2(err_pipe)` leaks
the two already-open `out_pipe` fds. Worse, the `CString::new` NUL checks (lines 111-118)
and the `fork()` failure (lines 122-124) all `return Err(...)` **after both pipes are
open**, leaking all four fds. In a long-lived PID 1, repeated `StartWorkload` of a unit
with (e.g.) a NUL-containing arg, or spawns under fd pressure, leak fds until `EMFILE` —
a self-reinforcing failure. Host-invisible until the VM runs many spawns.

**Fix:** close the opened pipe ends before every early return after they're created, e.g.
create both pipes first, then on any subsequent error `close` all four before returning
(a small guard struct with a `Drop` that closes non-`-1` fds is the cleanest).

### I2 — channel zeroing SAFETY invariant is asserted but not enforced (`O_CREAT` without `O_TRUNC`; index cells never reset)

**File:** `crates/detguest-agent/src/channel.rs:45-78` (`alloc`) and `:86-118` (`init_at`).

`alloc` opens with `O_CREAT | O_RDWR` (no `O_TRUNC`, no `O_EXCL`) and `ftruncate`s to
`CHANNEL_SIZE`. `ftruncate` to the *same* size does **not** zero existing content. The
SAFETY comment at line 75-76 claims "CHANNEL_SIZE zeroed bytes we exclusively own", but
nothing in the code enforces zeroing — it relies entirely on the environment (hugetlbfs
freshly mounted each boot → kernel hands back a zero page). Critically, `init_at` writes
only the header `[0, OFF_RESERVED=0x30)` and the manifest region; the **ring index cells
live at 0x100–0x2C0** (`header.rs` `OFF_RING_*_PROD/CONS`) and the drop counters are
**never written** by `init_at`. So if the channel file ever pre-existed with non-zero
indices, the producer would start mid-ring with no reset.

In production this is unreachable (PID 1 never restarts; a PID-1 fault reboots the whole
VM → fresh hugetlbfs mount). But it is exactly a defense-in-depth gap that an in-VM
edge (e.g., a future warm-restart of the agent, or a hand-mounted persistent hugetlbfs)
would silently corrupt, and the SAFETY comment overstates the guarantee.

**Fix:** make the zeroing explicit — either `O_TRUNC` in the open, or explicitly zero the
index/counter region in `init_at` (memset `[OFF_RESERVED, OFF_MANIFEST)` to 0) so the
"zeroed bytes" invariant is enforced by the code rather than assumed of the environment.
At minimum, downgrade the SAFETY comment to state the dependency on a fresh mount.

### I3 — EPOLLHUP busy-loop when a workload closes stdout/stderr without exiting

**File:** `crates/detguest-agent/src/supervise.rs:225-232` (pipe registration, `EPOLLIN`
only), `:284-309` (`drain_pipe` returns on `n <= 0`), `:400-426` (`run`).

The pipe read-ends are registered with `EPOLLIN` only, but Linux always reports
`EPOLLHUP`/`EPOLLERR` regardless of the requested mask. When a workload `close(1)/close(2)`
but keeps running (legitimate for a daemon), the read ends HUP. `drain_pipe` reads
`n == 0` (EOF) and returns, but the fd is **not** removed from epoll (removal only happens
in `reap()` on workload exit). With no SIGCHLD to trigger `reap`, every subsequent
`epoll_wait` returns immediately with `EPOLLHUP` on `TOK_OUT`/`TOK_ERR` → a tight
busy-loop burning virtual CPU until the workload eventually exits. Deterministic in
virtual time, but it spikes the icount unboundedly, which undermines the bit-reproducible
icount story the M2 acceptance gate keys on. (C2 makes this *permanent* for a leaked old
workload.)

**Fix:** distinguish EOF from EAGAIN in `drain_pipe` (a `read` returning `0` is EOF; `-1`
with `EAGAIN`/`EWOULDBLOCK` is "drained for now"). On EOF, `EPOLL_CTL_DEL` the fd and mark
it dead so it is not re-registered, while still keeping the `Workload` alive until its
SIGCHLD. Today the two cases (`n == 0` and `n < 0`) are conflated under `if n <= 0`.

### I4 — `log_mask == 0` cannot mean "silence all" (StartWorkload spec drift)

**File:** `crates/detguest-agent/src/commands.rs:17-24`; interacts with
`crates/detguest-agent/src/supervise.rs:257` (`self.log_mask = unit.log_mask`).

API.md §6: `StartWorkload{unit, log_mask}` → "**apply `log_mask`**" with no exception for
zero. The code treats `log_mask == 0` as "keep the unit default": `start_unit` first sets
`self.log_mask = unit.log_mask` (the manifest default, typically `0x1F`), then
`commands.rs` only overrides `if log_mask != 0`. Since the mask is a bitmask over levels,
`0` is the legitimate, meaningful value for "silence every level" — but a host that sends
`StartWorkload{unit, log_mask: 0}` gets the unit's default instead of silence. The host
cannot express "silence all" through this command.

**Fix:** apply the wire mask unconditionally on `StartWorkload` (the wire field is always
present and authoritative for that command), and let the manifest default apply only on
the *autostart* path. Concretely, in `commands.rs` set `sup.log_mask = log_mask;`
unconditionally, and in `start_unit` do **not** clobber `self.log_mask` from the unit on
the ring-C path (only on autostart). Document `0 = silence all` explicitly.

### I5 — post-Ready supervise-failure diagnostic is a droppable LogLine that can be lost when rings are full

**File:** `crates/detguest-agent/src/runtime.rs:192-207`.

When `sup.run()` returns `Err` (e.g., a ring-C `DecodeError` from a malformed host push,
propagated via `?` in `supervise.rs:443`), `runtime::run` emits the failure detail as a
`LogLine` (stream AGENT, level 0) + doorbell, then powers off. `LogLine` is **droppable**
(only Pad/Beacon/LogLine are non-critical per `record.rs:118`). §4.2 accepts droppable
for the *early* boot-fault case "because the rings are empty this early in practice" — but
this path runs **post-Ready**, when ring A may be full of buffered log/event traffic. If
it is, the diagnostic is silently dropped and the host observes only an unexplained
guest-halt. The agent also does not kill/reap a still-running workload here (no
`WorkloadExited`), so the host's model is left dangling.

**Fix:** for the post-Ready halt path, emit the diagnostic as a **critical** event (or
add a dedicated agent-fault event kind) so it cannot be dropped, and kill+reap any running
workload with a final `WorkloadExited` before `power_off()`, mirroring §4.2's
"kills the unit if still running and emits `WorkloadExited` (critical) + doorbell".
