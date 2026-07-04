# Package 03 — Verification Matrix, Guard Reversion, Handback

## Test matrix (all must pass before handback)

| Tier | Command / gate | Expectation |
|---|---|---|
| Agent unit | `cargo test -p detguest-agent` | Green, fast (test-mode budgets); includes the new delayed-peer and death-during-gate tests |
| Host unit | `cargo test --workspace` (host-runnable crates) | Green |
| VM preemptive | `DETGUEST_VM_TESTS=1`: `m2_acceptance`, `m4_acceptance`, `m4_snapshot`, `game_materialization` | Green — the timer-ful path is unchanged (`timer_interrupts` defaults true) |
| VM no-timer tier 1 | `no_timer_boot` (`DETGUEST_VM_TESTS=1`) | **Red before Fix A** (if it wedged pre-fix — see pkg 01 §3), **green after** |
| VM no-timer tier 2 | refwork twin (`REFWORK_READY_INITRAMFS=…`) | **Red before, green after** — this is the request's §3 criterion: reaches and *holds* Ready. ⚠ The initramfs embeds the agent: the green run REQUIRES the rebuilt artifact (local uncommitted `guest-sdk.lock` bump — pkg 01 §4); against the stale pre-fix artifact it stays red no matter what |

## Guard-reversion proof (ecosystem convention)

With the fix in place, temporarily revert the wait mechanism (put the
`sched_yield` back in `control.rs::recv` and `wait_for_expected_regions`,
skip `wait_boot_io`) and confirm the no-timer reproducer(s) go red again;
restore. Record the check in the reproducer test's module docs, same style as
`refwork_ready_hold.rs:25-28` and `game_materialization.rs:24-27`.

## If Fix A does NOT green the reproducer

Stop. Do not begin Fix B (deterministic pv-timer tick) in this repo — it is
cross-repo (a guest kernel driver for `dh-devices/src/clock.rs` + worker-side
arming) and the bridge drives the determinism-hypervisor half. Instead the
resolution file must carry: the reproducer's post-Fix-A trail (serial +
drained events), where it now wedges (agent blocked? workload blocked in what
syscall — add a one-shot `/proc/<pid>/stack`-style diagnostic to the agent
only if cheap), and the explicit judgment that Fix B (or A+B) is necessary.
That evidence is exactly what the request's verification loop asks for
("if it stays red with Fix A, that's the signal Fix B is needed").

## Handback: `.agents/requests/phase3-boot-scheduling-deadlock/03-resolution.md`

Write it with:

1. **Which fix**: A (or the A-insufficient evidence per above), with a
   two-paragraph summary of the mechanism actually proven by the reproducer
   (this request was explicit that the diagnosis was suspected-only — state
   what the reproducer demonstrated, e.g. "blocking the agent sufficed; no
   workload tick-dependency surfaced" or the opposite).
2. **Reproducer**: built (yes), mechanism used for interrupt suppression
   (GSI routing vs fallback), tier-1 pre-fix behavior (wedged or not),
   tier-2 red→green evidence (paste the key trail lines).
3. **Commits**: the guest-sdk sha(s).
4. **Lock bump line**: the exact
   `rev = "<full guest-sdk sha>"` for
   `reference-workload/image/guest-sdk.lock` (their build refuses on
   mismatch) — the bridge applies it.
5. **Bridge action items**: (a) re-run `dh-m9-ready-handoff` (their final
   gate → their `04-verification.md`) — including the **committed**
   `guest-sdk.lock` bump (item 4; the local bump used for tier-2 green stays
   uncommitted on our side); (b) expect a **shifted READY icount /
   state hash** (spin→block syscall-stream change — give the new value from
   tier 2 if observable); (c) **confirm the worker has a wall-clock budget**:
   with the agent parked in `epoll_wait` and no tick, a genuinely dead
   workload leaves the vCPU in HLT burning *no* instructions, so the icount
   HARD_CAP will never trip — the host-side wall deadline is now the only
   backstop for that failure mode. This is the deliberate residual risk of
   the bound redesign (plan 02 §5); the bridge must own it on the worker
   side; (d) two new-but-correct pre-Ready event-stream shapes are possible:
   workload stdout/stderr LogLines before Ready, and `WorkloadExited` before
   Ready when a workload dies mid-boot. `detguest-host`'s drain is a pure
   decoder (no Ready-first state machine), but confirm no worker-side
   consumer asserts Ready-first ordering.

## Session close

Standard protocol: quality gates, `bd close` the package beads,
`git pull --rebase`, `bd dolt push`, `git push`, verify up-to-date.
