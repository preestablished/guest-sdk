# Suggestions (non-blocking)

### S1 — `commands.rs:21-23` / `supervise.rs:257` — `StartWorkload{log_mask: 0}` cannot mean "silence everything"

`start_unit` first sets `self.log_mask = unit.log_mask` (manifest default), then
`handle` applies the command's mask only `if log_mask != 0`. So a host wanting to set the
mask to literally `0` (suppress all LogLine levels) gets the manifest default instead.
API.md §6 `StartWorkload{unit, log_mask}` says "apply log_mask" unconditionally. This
follows the common "0 = unspecified, use default" convention and is harmless for M2, but
if `0` is a legal "silence all" request the gate should distinguish absent from zero
(e.g. encode `log_mask` as `Option<u32>` on the command, or document `0` as "keep
manifest default"). A one-line comment stating the chosen meaning would suffice.

### S2 — `supervise.rs:311-314` — `SetLogMask` gating is level-only; API.md §6 says "levels/streams"

`emit_lines` gates with `log_mask & (1 << level)`. API.md §6 describes `SetLogMask` as
adjusting "which LogLine levels/**streams** are produced." Stream-based gating
(stdout vs stderr vs agent) is not implemented. Fine for M2 (the mask is documented as a
level mask in `boot.rs`), but worth a comment that stream gating is deferred, so the §6
"streams" wording is not silently unmet.

### S3 — `translate.rs:60-65` — PFN 0 is treated as "hidden", which would also reject a page legitimately backed by physical frame 0

`decode_entry` maps `present && pfn == 0` to `PfnHidden`. Physical frame 0 is a valid (if
rare) backing frame; for a hugetlbfs mapping it will never occur, so this is safe in
practice. Consider noting in the comment that the agent relies on the hugepage never
landing at GPA 0 (which is true for the guest memory map), so the `pfn == 0 ⇒ hidden`
heuristic cannot false-positive on real channel pages.

### S4 — `channel.rs:241-251` — heap allocation (`vec![0u8; tail]`) on the relay pad path

The relay allocates a `Vec` for the tail pad and another `[0u8; 32]` for the record (the
latter is a stack array — fine). The pad `Vec` is a heap allocation inside the
quiesce-relay hot-ish path. Allocation is deterministic so this is not a §7 violation, but
the `Producer::try_push` path encodes pads in place without allocating; the relay could
encode the pad directly into ring memory the same way (it already computes the slice).
Minor; only matters if relay frequency grows.

### S5 — `channel.rs:132-138` — `set_agent_ready` uses `write_volatile`/`read_volatile` on `header_flags` where the rest of the channel uses atomics

Header indices are accessed via `AtomicU32` (`from_ptr`, Acquire/Release) throughout the
ring code, but `header_flags` is touched with plain `read_volatile`/`write_volatile`. The
agent is the sole writer and the host reads it only while the vCPU is paused (§7 rule 7),
so this is correct today. For consistency and to make the host-visible publish ordering
explicit, consider an `AtomicU32::from_ptr(...).fetch_or(FLAG_AGENT_READY, Release)` — it
documents that the flag publish should be visible before the subsequent Hello doorbell.

### S6 — `supervise.rs:405` — epoll timeout 100 ms vs the 10 ms timerfd interval

The loop's `epoll_wait` timeout is `100`, while the periodic timerfd fires every `10` ms.
The 100 ms is just an upper bound on how long a pass blocks when nothing is ready; the
10 ms timerfd guarantees the loop wakes at least every 10 ms for the shutdown-deadline
sweep and ring-C poll. This is fine and deterministic (wakes are driven by virtual-time
fds, not the wall clock), but the relationship is non-obvious — a comment on the `100`
("upper bound only; the 10 ms timerfd drives the real cadence") would prevent a future
reader from thinking the poll cadence is 100 ms.

### S7 — `runtime.rs:211` — `const _: u16 = ports::PORT_IDENT;` "keep the import honest" hack

Using a `const _` to silence an unused-import lint for a symbol only referenced in
doc-comment paths is a little surprising. If `ports::PORT_IDENT` is genuinely only used in
docs, prefer importing just what's used (`ports::InitStatus`, `ports::{self}` is already
imported) and dropping the dead reference, or add `#[allow(unused_imports)]` with a
comment. Cosmetic.
