# Action Items

### Critical
- [ ] [boot.rs:82 / Cargo.toml:15] `toml = "0.8"` parses through a random-seeded
      `IndexMap` hasher (`toml → toml_edit → indexmap → hashbrown`), calling `getrandom(2)`
      in-guest — a P0 violation of ARCHITECTURE §7 rule 2 (confirmed by strace: an extra
      `getrandom(GRND_INSECURE)` vs baseline). Replace the dependency with a small
      hand-written parser for the repo-owned §7 boot.toml subset, or formally whitelist +
      document this one boot-time entropy read (weaker). [C1]
- [ ] [supervise.rs:251-280] `start_unit` overwrites a running `Option<Workload>` with no
      `Drop`, leaking the old process (no `WorkloadExited`), its pipe fds, and its epoll
      registrations, and emitting a misleading `WorkloadStarted`. Refuse (or deliberately
      kill+reap+close) when `self.workload.is_some()`. [C2]

### Important
- [ ] [supervise.rs:106-124] `spawn()` leaks pipe fds on the second-`pipe2`-fails,
      CString-NUL, and `fork`-fails error paths. Close all opened pipe ends before every
      early return (a fd-closing guard struct is cleanest). [I1]
- [ ] [channel.rs:45-78, 86-118] Channel zeroing is asserted in the SAFETY comment but not
      enforced: `O_CREAT` without `O_TRUNC`, and `init_at` never writes the ring index
      cells (0x100–0x2C0) / drop counters. Add `O_TRUNC` or explicitly zero
      `[OFF_RESERVED, OFF_MANIFEST)`, or downgrade the comment to state the fresh-mount
      dependency. [I2]
- [ ] [supervise.rs:225-232, 284-309, 400-426] EPOLLHUP busy-loop: pipes are `EPOLLIN`-only
      but HUP is always reported; a workload that closes stdout/stderr without exiting spins
      the loop until exit (and C2 makes it permanent). Distinguish EOF (`n == 0`) from EAGAIN
      (`n < 0`) and `EPOLL_CTL_DEL` the fd on EOF. [I3]
- [ ] [commands.rs:17-24 / supervise.rs:257] `log_mask == 0` is treated as "keep unit
      default", so the host cannot request "silence all". Apply the wire `log_mask`
      unconditionally on `StartWorkload`; let the manifest default apply only on autostart.
      [I4]
- [ ] [runtime.rs:192-207] Post-Ready supervise-failure diagnostic is a *droppable* LogLine
      that can be lost when ring A is full, and no running workload is killed/reported. Use a
      critical event for the halt diagnostic and kill+reap with a final `WorkloadExited`
      before power-off. [I5]

### Suggestions
- [ ] [channel.rs:240] Ring-I relay `seq = prod / total` is non-monotonic across a pad
      boundary (§7 rule 3). No consumer today; file a bead so the M3 SDK consumer gets a
      proper per-ring seq or explicit tolerance. [S1]
- [ ] [channel.rs:216-268] Add a `debug_assert`/re-entrancy guard or a cross-ref to the
      host-side pause guarantee backing the ring-I "temporal exclusivity" two-producer
      argument. [S2]
- [ ] [runtime.rs:34-47] Comment why only devtmpfs tolerates `EBUSY` in `mount_all` (kernel
      auto-mount) so the asymmetry is not "fixed" later. [S3]
- [ ] [runtime.rs:44] Pass `pagesize=2M` to the hugetlbfs mount as defense-in-depth for the
      `CHANNEL_SIZE == 2 MiB` assumption. [S4]
- [ ] [boot.rs:95-99] `boot_toml_version` exact-equality is correct for a bare-major field;
      add a note for when a minor convention is introduced. [S5]
- [ ] [boot.rs:159-165 / runtime.rs:138-140] Mark the deferred `control.proto_version`
      validation (M4) at the parse site so a misconfigured manifest is not silently
      accepted. [S6]
- [ ] [supervise.rs:3-5, 397-399] Tighten the loop-cadence comment: the 10 ms timerfd sets
      the ring-C poll cadence; the 100 ms epoll timeout is a backstop; both are virtual time
      (deterministic). [S7]
