# Fix Options And Verification

## Step 1: Pin the exact op (before any fix)

The worker DOES drain ring A on the doorbell (`detchannel.rs:590`), so
the naive "make the worker drain" fix is a no-op. Pin the real block
first with the counter in `01`'s "How To Pin It" — one bridge-run
real-worker handoff resolves it to one of:

- **(a) `emit` doorbell-drain not freeing producer-visible space**
  (`channel.rs:203-212`): a huge doorbell count. This would be a
  producer/consumer index-visibility issue between the guest `try_push`
  and the host `drain` under the real worker's deterministic memory
  model — investigate whether the consumer bump the host writes is
  observed by the guest producer before it re-checks (a missing
  acquire/release or a cache-vs-fresh read in the guest ring producer).
  A worker-side half may exist; if so the bridge files the
  determinism-hypervisor request and drives it.
- **(b) blocking reply `send`** (`region_ipc.rs:189`, `MSG_NOSIGNAL`,
  no `MSG_DONTWAIT`): a low count. Make it non-blocking/bounded and a
  failed/would-block send a named boot-fault.

## Step 2: Structural hardening (regardless of a/b)

Regardless of A or B, replace the **unbounded** `emit` critical-full
`loop` (`channel.rs:203-212`) with a **bounded** doorbell-wait that
boot-faults with a named leg + spin count when the host doesn't drain in
N iterations. Your wedge-to-fault hardening missed this because the spin
isn't in a *poll* loop — it's in `emit`. A never-draining host must
produce a loud fault, not a 10 B silent cap. Negative-test it (a stub
channel whose consumer never advances → the named boot-fault).

## Confirm The Mechanism Cheaply

Before/with the fix, add a counter to the `emit` critical-full branch
(doorbell-ring count) surfaced in the boot-fault detail. One
bridge-run real-worker handoff will then show the spin count pinned to
ring A and name the exact event that overflowed — turning this
code-confirmed diagnosis into an observed one, consistent with the
series' evidence discipline.

## Verification Loop — Read This

**The probe cannot reproduce symptom 1.** It drains ring A continuously,
so it reaches Ready on the exact image that wedges under the real
worker. Your `refwork_ready_hold` VM test will pass on a broken fix.
**Every candidate fix must be verified by a real-worker
`dh-m9-ready-handoff` run, which is the bridge session's to run.** Hand
back a lock bump + commit (or, for Option A, the determinism-hypervisor
change) and we turn the real-worker run around fast. Green =
`dh-m9-ready-handoff` reaches `Ready` and snapshots, yielding the step-2
exit evidence (READY icount ≈ 643 M, region_count 3 / gen 6, state
hash) — which unblocks the READY-snapshot regeneration (step 3), the
`BRIDGE_REAL_SNAPSHOT_REF` cutover (step 4), and the first real frame in
the browser.

## Handback

`03-resolution.md` here with your routing decision and the fix. We
re-run the real-worker handoff and answer with `04-verification.md`.
