# 02 — Agent IPC server, per-pid pagemap, agent-side manifest writer

Closes beads `guest-sdk-m4-agent-ipc-server` and
`guest-sdk-m4-agent-pagemap-pid-extents`. The agent becomes the **sole**
manifest writer, satisfying the seqlock discipline stated in
`detguest-wire/src/manifest.rs:10`.

## New module: `crates/detguest-agent/src/region_ipc.rs`

### State

```rust
pub(crate) struct RegionIpc {
    listener: OwnedFd,              // bound + listening, O_NONBLOCK
    conns: Vec<Conn>,               // accepted, O_NONBLOCK, each with peer pid
    records: Vec<RegionRecord>,     // registration ledger, indexed by region_id
}
// NO agent-side name_id counter: name_id arrives in the request (the SDK's
// InternTable is the single allocator; two counters would collide in the
// host's unified ring-A+ring-W intern map — see channel.rs intern_redefinitions).
pub(crate) struct RegionRecord {
    pub region_id: u32,
    pub name: Vec<u8>,
    pub name_id: u32,
    pub layout_version: u32,
    pub flags: u32,
    pub pid: i32,
    pub gva: u64,
    pub len: u64,
    pub extents: Vec<Extent>,
    pub dead: bool,
}
```

### Bind

`RegionIpc::bind() -> io::Result<RegionIpc>`: `create_dir_all("/run/detguest")`,
unlink stale socket, `socket(AF_UNIX, SOCK_SEQPACKET|SOCK_NONBLOCK|SOCK_CLOEXEC)`,
`bind(AGENT_SOCK_PATH)`, `listen(4)`. Called in `runtime.rs::run()` after
`Supervisor::new` and **before** `autostart_and_ready` (so the socket exists
before any workload runs). Bind failure = boot fault (§7.3) — a guest without
the region path must not reach Ready.

Store the `RegionIpc` on the `Supervisor` (new field), since command dispatch
(`commands.rs`) and the epoll loop need it, and registration must write through
`sup.channel`.

### Service entry point (the deadlock-avoidance primitive)

```rust
/// Accept pending connections and process every readable request datagram.
/// Non-blocking; returns after draining. Safe to call from any wait loop.
pub(crate) fn service(sup: &mut Supervisor) -> io::Result<()>
```

(Free function or method — pick whichever borrows cleanly; it needs
`sup.channel`, `sup.workload` pid, and the `RegionIpc` state simultaneously,
so a free function taking disjoint borrows may be required. Resolve the borrow
split however the code reads best, e.g. `RegionIpc::service(&mut self, channel:
&mut AgentChannel, workload_pid: Option<i32>)`.)

Per accepted conn: `getsockopt(SO_PEERCRED)` once at accept; store pid.
Per request datagram: decode → handle → reply on the same conn. `EAGAIN` ends
the drain. Peer EOF / send failure → drop the conn (workload death is handled
by reap; records are NOT dropped on disconnect — regions outlive the socket).

### Call sites (all three are required — see 00 §deadlock)

1. **Supervise epoll loop** (`supervise.rs::run`): register the listener fd
   (`TOK_REGION_LISTENER = 5`) and each accepted conn fd (`TOK_REGION_CONN_BASE
   + i`) with `epoll_ctl`; on those events call `service`. Also acceptable and
   simpler: register only the listener + conns and call the same `service`
   drain for any of their tokens. Keep the existing 100 ms timeout pass
   calling `poll_command` unchanged.
2. **`runtime.rs::wait_for_expected_regions`** poll loop: call `service`
   each iteration before `expected_regions_ready`, replacing nothing else.
   (This is where registrations actually complete for the autostart path.)
3. **`control.rs::drive_refwork_start`** recv waits: the workload registers
   regions between `GameLoaded` and control-`Ready`
   (`m9_refwork_contract.rs:58-64`), so the blocking `recv` at `control.rs:109`
   deadlocks. Change `ControlSocket::recv` to take a "while idle" callback:
   loop `recv(MSG_DONTWAIT)`; on `EAGAIN` call the callback (which calls
   `service`) then `sched_yield`; bound the loop with the same
   `READY_REGION_POLL_LIMIT`-style iteration cap pattern used in `runtime.rs`
   (test cfg small / prod cfg large) so a dead workload still faults instead
   of spinning forever. Apply to all three recv sites in
   `drive_refwork_start` (HelloAck, GameLoaded, Ready) — registration before
   GameLoaded is legal for non-refwork workloads.

   **Stated semantic change:** today's recv blocks indefinitely; the bounded
   loop adds a new boot-fault mode on a contract-frozen path. Size the cap
   against the slowest legitimate leg (LoadGame doing real device I/O on the
   reference workload), not against region publication — the prod cap should
   represent well over a minute of sched_yield spins; document the number
   and rationale in a comment.

Threading: everything stays single-threaded; ordering is deterministic
(drain order = accept order, datagram order per conn is FIFO by SEQPACKET).

## Per-pid pagemap: extend `crates/detguest-agent/src/translate.rs`

- `pub fn open_pagemap_for(pid: i32) -> io::Result<File>` —
  `/proc/<pid>/pagemap`. Keep `open_pagemap()` (self) delegating to it.
- Move/copy `build_extents` (page-walk + adjacent-GPA coalescing + extent cap)
  from `detguest-sdk/src/regions.rs:177-210` into the agent (e.g.
  `region_ipc.rs` or `translate.rs`), **with its unit tests** (they inject a
  translate closure — port them as-is). The SDK's copy is deleted in `03-…`.
- Coalescing and error mapping semantics are unchanged:
  `NotPresent|Swapped|PfnHidden → STATUS_NOT_PINNED`, `Io → STATUS_INTERNAL`.
- Per-region extent bound: unchanged global pool discipline
  (`EXTENT_CAPACITY = 1024` across all regions; `RegionEntry.extent_n` per
  region). A 229,376-byte .bss region can produce up to 56 extents — fine.

## Registration handling (the manifest write moves here)

Port `SdkState::publish_region` (`detguest-sdk/src/lib.rs:374-448`) into the
agent as `region_ipc::handle_register`:

1. Validate: name ≤ 56 bytes; DEAD bit clear in flags; peer pid ==
   `sup.workload.pid` (else `STATUS_UNKNOWN_PID`; if no workload is running,
   also `STATUS_UNKNOWN_PID`).
2. Translate: `open_pagemap_for(pid)` + `build_extents(gva, len)`. Any
   translate failure → `STATUS_NOT_PINNED` / `STATUS_INTERNAL` per mapping.
   (The agent independently proves residency — the SDK's mlock claim is not
   trusted.)
3. Capacity: next free region slot (reuse DEAD slots? **No** — keep the
   existing SDK writer's slot policy. Check what `publish_region` does today
   and preserve it exactly: region_id assignment, extent_off = current
   `extent_count`, counts bump. If `publish_region` never reuses slots, keep
   that; unregister only sets the DEAD flag.) `REGION_CAPACITY` exhausted →
   `STATUS_MANIFEST_FULL`; extent pool exhausted → `STATUS_TOO_MANY_EXTENTS`.
4. Intern evidence: use the request's `name_id` verbatim; emit
   `EventPayload::NameIntern{name_id, name}` on **ring A** via `sup.channel`.
   (Verify in `detguest-host/src/drain.rs` that an identical id→name pair
   arriving on both rings is benign — the existing pre-Ready evidence path
   already re-emits the same pair on ring A, so this should be established
   behavior; if the `intern_redefinitions` counter increments even for
   identical pairs, fix the host to count only *conflicting* redefinitions
   and unit-test it. Reject `name_id == 0` at the codec layer per `01-…`.)
5. Manifest write under the seqlock: `writer_begin` → SeqCst fence → write
   extents into pool slots, write `RegionEntry{…, gva, len, extent_off,
   extent_n, name}` → update header counts → fence → `writer_end`. Exactly the
   discipline in `runtime.rs` test helper `write_live_region`
   (`runtime.rs:452-482`) and today's `publish_region`.
6. Record: push `RegionRecord{…}`.
7. Emit `EventPayload::RegionRegister(RegionEvent{region_id, name_id,
   layout_version, manifest_generation: u32::try_from(gen)?})` on ring A with
   doorbell.
8. Reply `STATUS_OK{region_id, name_id, manifest_generation}`.

`handle_unregister(region_id)`: record exists and live → set DEAD flag on the
manifest entry under the seqlock, bump nothing else, mark record dead, reply
OK (manifest_generation = post-write). Unknown/dead → `STATUS_UNKNOWN_REGION`.

## Interaction with existing pre-Ready evidence

`runtime.rs::emit_expected_region_evidence` re-emits `NameIntern` +
`RegionRegister` per expected region before Ready. This becomes a duplicate of
the registration-time emission. **Keep it** — it is load-bearing evidence for
the Ready gate and its exact sequence is asserted by
`runtime.rs::tests::expected_regions_ready_emit_real_manifest_snapshot`.
Duplicated RegionRegister events are harmless (hosts key on the manifest).
Note in API.md that ring A may carry both.

## Tests (host `cargo test -p detguest-agent`)

Existing agent unit tests construct `Supervisor` with `test_channel`; extend
that pattern:

- `RegionIpc` over a real socketpair-style flow: bind to a temp path (make the
  socket path injectable — `RegionIpc::bind_at(path)`, with `bind()` using
  `AGENT_SOCK_PATH`), connect a client, drive register/unregister with an
  injected translator (make the translate step injectable like
  `build_extents` already is, so tests don't need real pagemap PFNs — CI runs
  unprivileged where pagemap PFNs are hidden).
- pid binding: a request from a non-workload pid gets `STATUS_UNKNOWN_PID`
  (test can use the real client pid vs. a fabricated `sup.workload` pid).
- Malformed datagrams: server replies `STATUS_BAD_REQUEST` and survives.
- Manifest effects: after OK register, `copy_manifest_stable` shows the live
  entry with correct extents; after unregister, DEAD flag set; generation
  parity even at rest; nested-writer misuse impossible (single-threaded).
- Ring A sequence: NameIntern then RegionRegister with matching ids.
- Port the SDK's manifest-writer unit tests here: `detguest-sdk/lib.rs` tests
  that exercise `publish_region` via the `#[cfg(test)]`
  `register_region_with_extents` funnel (manifest contents, generation
  discipline, capacity errors, event sequence) lose their home when
  `publish_region` moves — they become agent-side tests of
  `handle_register` with injected extents, not deleted coverage.
- Existing tests must keep passing: stdout/stderr command polling
  (`supervise.rs` tests), control tests, runtime Ready tests (these now need
  the region-IPC service wired into their loops or a `RegionIpc` test
  double — keep `Supervisor::new` signature workable for tests, e.g.
  `RegionIpc` optional or bound to a temp dir in tests).

## Done when

- All agent host tests green; musl static agent build green
  (`ci.yaml` `musl` lane builds `-p detguest-agent --target
  x86_64-unknown-linux-musl`); no clippy warnings.
- No change to `detguest-host` behavior needed (manifest bytes identical).
