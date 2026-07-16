# Action Items

### Critical
- [ ] [pio.rs:30,61 + channel.rs:169-173] The doorbell `OUT` is `options(nomem)`, letting the compiler reorder it before the Release store that publishes the ring record — violating the "record visible before OUT" discipline (ARCHITECTURE.md §2 / API.md §5). Add `core::sync::atomic::compiler_fence(SeqCst)` before the OUT in `doorbell` (or drop `nomem` on `out32`).

### Important
- [ ] [channel.rs:240] Ring-I relay `seq = prod / total` does not match the host producer's `next_seq_i` counter, breaks after a tail pad, and reuses one seq for pad + record (§7 rule 3 / `try_push` "pads consume their own seq" convention). Replace with an explicit per-record relay seq counter and allocate a separate seq for the pad and the record; update the comment to state the chosen invariant.
- [ ] [boot.rs:159-165] `[unit.control].proto_version` equality against the agent's spoken version (API.md §7.2 "must equal the value the agent speaks") is not checked — only range-checked. Currently unreachable at boot (the autostart control leg faults pre-Ready, runtime.rs:135-140), but add the equality check when the M4 control leg lands; leave a `// TODO(M4)` marker now.

### Suggestions
- [ ] [commands.rs:21-23] `StartWorkload{log_mask: 0}` is treated as "use manifest default", so `0` cannot mean "silence all" — document the chosen meaning or model the field as `Option<u32>`.
- [ ] [supervise.rs:311-314] `SetLogMask` gating is level-only; API.md §6 mentions "levels/**streams**" — note that stream gating is deferred.
- [ ] [translate.rs:60-65] `present && pfn == 0 ⇒ PfnHidden` would also reject a page backed by physical frame 0; note the channel hugepage never lands at GPA 0 so this cannot false-positive.
- [ ] [channel.rs:241-251] Relay pad path heap-allocates a `Vec`; encode the tail pad in place like `Producer::try_push` does.
- [ ] [channel.rs:132-138] `set_agent_ready` uses plain `write_volatile` on `header_flags`; consider an atomic `fetch_or(.., Release)` for consistency with the ring index discipline and to document publish ordering before the Hello doorbell.
- [ ] [supervise.rs:405] Comment that the 100 ms `epoll_wait` timeout is an upper bound and the 10 ms timerfd drives the real poll cadence.
- [ ] [runtime.rs:211] Replace the `const _: u16 = ports::PORT_IDENT;` unused-import hack with a scoped `#[allow]` or trimmed import.
