# Package 02 — Fix A: epoll-Blocking Boot Waits (agent side)

Prerequisite: package 01's reproducer is built and red. Everything here is in
`crates/detguest-agent/`.

## The shape

Both boot waits — the control-reply wait
(`control.rs::ControlSocket::recv`, `:214`) and the expected-regions gate
(`runtime.rs::wait_for_expected_regions`, `:366`) — currently spin:
`MSG_DONTWAIT → service_region_ipc → sched_yield`. They become: *block in the
supervisor's existing epoll until something relevant is readable, service it,
re-poll*. The agent deschedules; the workload's `send(2)` on fd 3 or connect/
send on agent.sock performs the wakeup directly (no tick required). That is
the whole fix.

## Step 1 — `ControlSocket::raw_fd()`

`control.rs`: add `pub(crate) fn raw_fd(&self) -> RawFd` (trivial
`self.fd.as_raw_fd()`). Nothing else in the socket changes; keep
`MSG_DONTWAIT` on the recv itself — the fd stays blocking-agnostic and the
epoll wait is edge-agnostic (level-triggered, default).

## Step 2 — `Supervisor::wait_boot_io` (the mini supervise pass)

`supervise.rs`. New token `const TOK_CONTROL: u64 = 7;` and:

```rust
/// Boot-leg blocking wait: park in the supervise epoll until any wake
/// source fires, then service it. Used by the pre-Ready waits so the boot
/// handshake makes progress in a guest with NO timer interrupts — the
/// workload's own send/connect syscalls are the wakeup (request
/// phase3-boot-scheduling-deadlock). The timeout only ever fires in a
/// tickful environment (host unit tests, the PIT-ful probe): in the
/// no-tick guest, timer expiry is itself interrupt-driven, so the
/// authoritative hang bound is the HOST's wall-clock deadline, not this.
pub(crate) fn wait_boot_io(&mut self, timeout_ms: i32) -> io::Result<()>
```

Body: one `epoll_wait(self.epfd, &mut events, 8, timeout_ms)` (retry on
`EINTR`), then for each event:

- `TOK_SIG` → drain the signalfd **and reap the workload**. Do not merely
  drain: consuming the SIGCHLD datum without reaping would leave a workload
  that dies mid-gate as an unreported zombie after Ready (the supervise loop
  would never see TOK_SIG for it). Reaping here is correct and strictly
  better than today: a workload death now *wakes* the boot wait, the reap
  clears `workload_control`/emits `WorkloadExited`, and the caller's next
  control recv hits EOF ("control socket closed") → immediate named boot
  fault instead of a poll-cap timeout. **But do NOT call the existing
  `reap()` here** — its `waitpid(-1, WNOHANG)` loop (`supervise.rs:443`)
  reaps *any* child of the process, and `wait_boot_io` (unlike
  `Supervisor::run`) is exercised by the multi-threaded cargo test harness,
  where a `-1` wait can steal another test's child (e.g.
  `spawn_exports_sdk_channel_fd…`'s `wait_for(pid)` at `supervise.rs:773`)
  and flake CI. Add a targeted variant — `reap_workload()`, waiting on
  `self.workload`'s pid specifically with `WNOHANG | WUNTRACED` and then
  running the same exited/stopped handling — and call that from
  `wait_boot_io`. The supervise loop's `reap()` (single-child PID 1 context)
  stays as-is.
- `TOK_OUT | TOK_ERR` → `self.drain_pipe(ev.u64)`, plus the same
  HUP/ERR-deregister discipline as the run loop (`supervise.rs:535-557`).
  **This arm is load-bearing, not housekeeping**: the workload's
  stdout/stderr pipes are already in the epoll set during boot (registered
  at `start_unit_inner`, `supervise.rs:369-377`), epoll is level-triggered,
  and workloads print during the boot leg (the tier-1 reproducer's
  `game-load-check` `println!`s immediately before sending GameLoaded). An
  undrained readable pipe would make every `wait_boot_io` return instantly —
  burning the wakeup cap into a false "wedged" boot fault on a healthy boot,
  and in the no-tick guest the agent would never deschedule, reinstating the
  original deadlock class. `drain_pipe` is pre-Ready-safe. Side effect:
  workload LogLines can now be emitted before Ready — note it in Step 7 and
  the package-03 bridge items.
- `TOK_TIMER` → drain the timerfd counter (level-triggered; an undrained
  tick would make every subsequent wait return immediately and quietly turn
  the block back into a spin in tickful environments).
- `TOK_REGION_LISTENER | TOK_REGION_CONN | TOK_CONTROL | _` → nothing here;
  fall through.

Then unconditionally `self.service_region_ipc()?` (same
covers-races posture as the supervise loop, `supervise.rs:565-567`) and
return. The caller re-polls its own condition; readiness of the control fd
is discovered by the caller's next `MSG_DONTWAIT` recv, so `TOK_CONTROL`
needs no payload handling.

Note: region-IPC conn fds are *already* in the epoll set during boot —
`service_region_ipc` always passes `Some((self.epfd, TOK_REGION_CONN))`
(`supervise.rs:327`) and the listener is registered at `install_region_ipc`
(`supervise.rs:314`). Only the control fd is new. (Region conns
self-deregister on EOF via `OwnedFd` drop in `region_ipc.rs`, so no HUP
busy-spin from that side.)

**Test-environment caveat on the sigfd path:** signalfd only reliably
observes a process-directed SIGCHLD when *every* thread blocks it;
`Supervisor::new` blocks it on the constructing thread only
(`supervise.rs:252-254`), which is fine for single-threaded PID 1 in the
guest but means the sigfd wake is **unreliable inside the multi-threaded
cargo test process** (another thread can deliver-and-discard the signal).
Unit tests must therefore never *depend* on the sigfd→reap wake — the
deterministic death signal in tests is the control-fd EOF / pipe-HUP wake
(see Steps 5–6). In the guest both paths work.

## Step 3 — Control-fd registration lifecycle (boot leg only)

`runtime.rs::drive_and_retain_control` (`:188`):

1. Before `drive_refwork_start`: `EPOLL_CTL_ADD` the socket's `raw_fd()`
   with `TOK_CONTROL` (add a small `Supervisor::{register,deregister}_control_fd`
   pair next to `install_region_ipc` so the epfd stays private).
2. After `drive_refwork_start` returns — **on both the `Ok` and `Err`
   paths** — `EPOLL_CTL_DEL` it. Then retain the socket exactly as today
   (`sup.workload_control = Some(sock)`). Concretely: the current code
   applies `?` directly to the `drive_refwork_start` call
   (`runtime.rs:194-202`), which would skip the DEL on error — capture the
   result into a local, deregister, *then* `?` it.

Why boot-only: post-Start the agent never touches the control socket, and a
persistent registration would make the supervise loop wake on workload-side
EOF with a token it must then handle (HUP → potential busy-spin between
death and reap). Deregistering closes that whole analysis off. Document this
in the comment on `TOK_CONTROL`. (Closing the fd would auto-deregister, but
the socket outlives the boot leg by design — symptom-2 retention — so the
explicit DEL is required, mirroring the pipe-fd discipline at
`supervise.rs:475-491`.)

`reap`/`immediate_shutdown` set `workload_control = None` while the fd is
already deregistered — no change needed there.

## Step 4 — Rewire the two waits

**`control.rs::recv`:** keep the `MSG_DONTWAIT` recv + idle-callback
structure and the loop counter; delete the `sched_yield`. The `idle()`
callback (wired in `drive_and_retain_control`) now *blocks*:

```rust
control::ControlProgress::Idle => {
    sup.wait_boot_io(BOOT_WAIT_TIMEOUT_MS)
        .map_err(|e| format!("boot wait: {e}"))?;
    ...  // (service_region_ipc already happens inside wait_boot_io —
         //  drop the separate call)
}
```

**`runtime.rs::wait_for_expected_regions`:** the loop shape MUST be
**service → check → then wait**, i.e. the condition is evaluated before the
first block:

```rust
for _ in 0..READY_REGION_WAKE_LIMIT {
    sup.service_region_ipc()...?;               // drain anything pending
    match expected_regions_ready(...) {          // check BEFORE blocking
        Ok(snapshot) => return Ok(snapshot),
        Err(err) => last_err = err,
    }
    sup.wait_boot_io(BOOT_WAIT_TIMEOUT_MS)...?;  // only now block
}
```

This ordering is load-bearing, not style: the gate's condition is *manifest
state*, not fd readiness, and in the normal boot the condition is **already
true on entry** — all three regions register during the control leg that
precedes this wait (`runtime.rs:25-27` comment; the real-worker trail shows
all RegionRegisters at icount ≈643 M, mid-control-leg). A wait-then-check
loop would block first on an epoll set with nothing pending, and in the
no-tick guest that first block never times out: the *fixed* agent would
wedge at the gate and the reproducer would mis-signal "Fix B needed".
Check-then-wait has no lost-wakeup race because the epoll is
level-triggered: a registration datagram that arrives between the check and
the wait leaves its conn fd readable and the wait returns immediately.
Keep the `last_err` accumulation; the empty-`expected_regions` fast path
keeps its single non-blocking `service_region_ipc` (no wait — nothing to
wait for).

## Step 5 — Bound redesign: iteration caps → wakeup caps

Replace the two icount-proxy caps:

- `control.rs`: `CONTROL_RECV_POLL_LIMIT` → `CONTROL_RECV_WAKE_LIMIT`,
  counting *wakeups* (loop iterations, each now ≤ one `timeout_ms` block).
- `runtime.rs`: `READY_REGION_POLL_LIMIT` → `READY_REGION_WAKE_LIMIT`,
  same semantics.

Sizing (document this reasoning in the consts' doc comments, replacing the
now-obsolete icount arithmetic):

- `BOOT_WAIT_TIMEOUT_MS`: `#[cfg(not(test))] 100`, `#[cfg(test)] 5`. It
  lives in `supervise.rs` next to `wait_boot_io` (both `control.rs`-driven
  and `runtime.rs`-driven waits reach it through the `Supervisor` method,
  so the callers never need the value — pass nothing, bake it into
  `wait_boot_io`'s callers via one const).
- Non-test limits: `600` wakeups each → worst-case ≈60 s of *tickful* wall
  time per leg; in the no-tick guest, timeouts never fire, so the cap only
  counts genuine (productive or spurious) wakeups and a total dead-block
  parks the guest in HLT — **by design caught by the host deadline** (the
  harness's `run_until` wall deadline with serial dump; the worker's own
  wall budget — the resolution file must tell the bridge to confirm the
  worker has one).
- `#[cfg(test)]` limits: `CONTROL_RECV_WAKE_LIMIT = 200`,
  `READY_REGION_WAKE_LIMIT = 50` (the gate test has no wake source except
  the 10 ms timerfd, so its worst case is ~0.5 s — keep it out of
  seconds-territory; the invariants list promises a fast unit tier).
  `unit_control_faults_before_ready_when_workload_does_not_reply`
  (`runtime.rs:857`) exits fast via the **control-fd EOF wake**: `/bin/true`
  exits, its fd-3 end closes (verified: no surviving dup — `spawn()` only
  dups the *channel* fd, `runtime.rs:249` drops the parent's copy of the
  child end, and the parent's own end is CLOEXEC in the child), the
  control fd becomes readable/HUP, `wait_boot_io` wakes, and the next
  `MSG_DONTWAIT` recv returns 0 → "control socket closed". Do NOT describe
  or rely on a SIGCHLD→reap wake for this in tests (unreliable there — see
  Step 2's caveat). Keep the assertion on `"recv refwork HelloAck"` — the
  EOF and cap paths both carry that prefix.
- Keep the cap-exhausted error texts naming the leg (the request's "named
  leg" requirement): the existing `TimedOut` message in `recv` and the
  `expected-regions gate exhausted` message stay, reworded from "polls" to
  "wakeups".

## Step 6 — Tests (agent unit tier, host-runnable)

1. Existing suite: two tests assert `sup.workload.is_some()` *after* the
   boot fault — `unit_control_faults…` (`runtime.rs:876-879`) and
   `expected_regions_pending_starts_unit_but_blocks_ready` (`runtime.rs:675`).
   Under reap-inside-wait these become **timing-dependent**: if the sigfd
   wake happens to fire (it sometimes will, e.g. under `--test-threads=1`),
   `reap_workload` takes `self.workload` and the assertion fails. Relax
   both: assert the *intent* — a `WorkloadStarted` event is on ring A
   ("autostart happened before the gate/fault") — instead of the
   liveness-at-fault-time of `workload.is_some()`, and document why. The
   "expected_regions pending" / "recv refwork HelloAck" error-text
   contracts and the no-Ready assertions stay as-is.
   `control_leg_retains_workload_socket_and_names_its_legs` (breadcrumbs +
   retention) must pass unchanged.
2. New: a `fake_harness`-style test where the peer thread **delays** each
   reply (e.g. 50 ms) — proving the boot leg completes while blocked (not
   spinning through a cap) and the breadcrumb order is preserved. Assert
   completion well under the test budget.
3. New: workload-death-during-gate. Concrete recipe (this does not fall out
   of `autostart_and_ready`): build a `Supervisor` with a nonempty
   `expected_regions` manifest, `sup.start_unit(0)` with `/bin/true` (no
   control fd), wait for the child to actually exit (poll
   `kill(pid, 0)`/short sleep — do NOT `waitpid` it yourself; that's the
   agent's job), then call `wait_for_expected_regions` and assert it
   returns the gate-exhausted error **within ~1 s** (the pipes HUP-wake and
   then the test-mode timeout budget runs out) rather than hanging. Do not
   assert on `WorkloadExited`/reap here — the sigfd wake is unreliable in
   the test process (Step 2 caveat); the reap contract gets its real
   coverage in the VM tier, where the agent is single-threaded PID 1.

## Step 7 — Expected downstream shifts (do not chase)

READY-point icount and state hash change (different syscall stream). Fine:
`m2_acceptance` icount is self-consistency-only unless
`DETGUEST_STRICT_ICOUNT=1`; the deployed READY snapshot is regenerated by the
bridge (its step 3). Record the new observed READY icount in the resolution
file so the bridge sees the delta coming.

Two new *event-stream* shapes are also possible pre-Ready (both correct, both
new to the host — package 03 routes them to the bridge):
- workload stdout/stderr LogLines before Ready (Step 2's pipe-drain arm);
- `WorkloadExited` before Ready when a workload dies mid-boot
  (reap-inside-wait). Verified acceptable host-side — `detguest-host`'s
  drain is a pure decoder with no Ready-first state machine — but the
  worker has never observed this ordering.
