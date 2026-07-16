# Review: in-guest PID 1 agent (Milestone 2 guest side) — 2nd reviewer

- **Branch:** `ralph/iteration-5-agent-pid1-channel-supervise`
- **Head:** `9e4097d` (ralph: iteration 5 checkpoint — detguest-agent PID 1)
- **Date:** 2026-06-10
- **Reviewer:** Claude Opus (2nd reviewer)
- **Base:** `main`

## Summary

This branch lands the in-guest PID 1 agent (`crates/detguest-agent`): boot/runtime
sequencing (mounts → channel bring-up → Hello → boot.toml parse → autostart → Ready →
supervise loop), the agent's detchannel half (`channel.rs`: hugetlbfs alloc, ring-A
producer, ring-C consumer, ring-I quiesce relay), the fork/exec/epoll supervisor
(`supervise.rs`), ring-C command dispatch (`commands.rs`), detcall PIO (`pio.rs`),
pagemap GVA→GPA translation (`translate.rs`), boot.toml parse/validate (`boot.rs`), and
the M2 acceptance manifest (`image/boot.toml.m2`). The code is clean, well-commented,
maps closely to ARCHITECTURE §4/§6/§7 and API §5/§6/§7, and all 17 agent unit tests
plus the full workspace suite pass with clippy clean. The spec-mapping discipline is
excellent. However I found one **P0 determinism violation** that is structurally
invisible to host tests (the `toml` parser consumes guest entropy via a random-seeded
hasher — a direct ARCHITECTURE §7 rule 2 violation), and several **Important**
in-VM/process-edge failure modes the host harness cannot exercise: a
`StartWorkload`-while-running path that leaks the prior workload + its pipe fds + epoll
registrations, fd leaks on `spawn()` error paths, an EPOLLHUP busy-loop when a workload
closes its stdout/stderr without exiting, a `log_mask == 0` spec drift (cannot express
"silence all"), and a channel-zeroing SAFETY invariant that the code asserts but does
not enforce.

The orphan-reaping loop (the load-bearing PID-1 correctness property), the SIGCONT /
WCONTINUED interaction, the ring-A event ordering (WorkloadStarted before Ready), the
emit-with-doorbell release ordering, and the boot.toml validation are all **correct** —
I traced each and they hold.

## Verdict

**REQUEST_CHANGES** — driven by the §7 rule-2 entropy violation (P0 per the spec's own
classification) and the `StartWorkload`-while-running resource leak. Both are real, both
are largely invisible to the current host test surface, and the entropy one fires on
*every* boot the moment `boot.toml` is parsed. The remaining Important items are
defense-in-depth / robustness and could be sequenced, but the two Critical items should
land before this is treated as a trustworthy M2 guest.

## Stats

- Files reviewed: 11 source + `image/boot.toml.m2` (12 changed, +2000/-14)
- Critical: 2
- Important: 5
- Suggestions: 7
- Tests: `cargo test --workspace` → all pass (17 agent unit tests, 91 total across
  workspace); `cargo clippy --workspace` → clean.
- Host experiment run: drove `supervise::spawn()` against `/bin/echo`, `/bin/false`,
  and a missing binary — stdout pipe delivery, EOF, and exit-status decode (0 / 1 / 127)
  all behaved correctly. (Experiment removed after verification; not committed.)
- Empirical determinism check: strace of `toml::Value::parse` showed an extra
  `getrandom(GRND_INSECURE)` syscall vs an empty-binary baseline — confirms the §7
  finding.
