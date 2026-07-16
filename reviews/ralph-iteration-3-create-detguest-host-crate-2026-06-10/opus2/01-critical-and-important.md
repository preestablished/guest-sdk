# Critical & Important Findings

## CRITICAL

### C1 — Host ring C/I seq counters are lost across snapshot restore; re-attach re-emits duplicate seqs

**Files:** `crates/detguest-host/src/channel.rs:91-114, 175-176`,
`crates/detguest-host/src/commands.rs:33, 41, 54, 62`

The doc comment on `Channel` (channel.rs:91-96) states the host-side state "must be
checkpointed alongside the hypervisor's per-branch state; it is **reconstructible from the
event stream** but caching it avoids re-scans." This claim is **false for `next_seq_c` and
`next_seq_i`**.

`next_seq_c`/`next_seq_i` are the host's *producer* sequence counters for rings **C** and
**I**. The host is the producer on C/I and the consumer only on A/W — `drain_events` reads A
and W. So these two counters are **never** observed by draining; they are pure host-side
producer state. `attach` initializes both to `0` (channel.rs:175-176), there is no
derive-from-ring path, and no public getter/setter exists (both fields are `pub(crate)`).

Consequence of the documented "re-attach after restore" flow: after a snapshot restore the
hypervisor re-runs `Channel::attach`, resetting both counters to 0, while ring C/I *memory*
(and the prod/cons indices, which are guest RAM and survive the restore) already reflect
prior pushes with seq 0, 1, 2, …. The next `push_command` then stamps the new record with
seq **0 again**, colliding with the seq-0 record already present in the ring. The whole point
of the per-ring seq is bit-deterministic identity of host-produced records; restoring to a
duplicate seq breaks that determinism silently — no error, no metric.

Two clean fixes (pick one):

1. **Derive at attach** (matches the "reconstructible" doc claim). Scan ring C and ring I
   from `cons` to `prod`, counting records *and pads* (each consumes one seq, exactly as
   `push_record` allocates), and seed the counters:

   ```rust
   // in attach(), after header validation, before returning Channel:
   let next_seq_c = Self::count_producer_seqs(&gm, base_gpa, RingId::C)?;
   let next_seq_i = Self::count_producer_seqs(&gm, base_gpa, RingId::I)?;
   ```

   where `count_producer_seqs` walks `cons..prod` masked, adding 1 per record/pad.
   (Note: this only works while the records remain in-ring; if the guest has consumed and
   the producer wrapped past them, the count is lost — so prefer fix 2 for robustness.)

2. **Expose the counters for checkpointing** (matches "must be checkpointed alongside"):

   ```rust
   /// Host producer seqs for rings C and I — opaque checkpoint state that the
   /// hypervisor must save/restore with its per-branch state (NOT derivable by
   /// draining, which only consumes A/W).
   pub fn producer_seqs(&self) -> (u32, u32) { (self.next_seq_c, self.next_seq_i) }
   /// Restore the producer seqs captured by [`Channel::producer_seqs`].
   pub fn restore_producer_seqs(&mut self, c: u32, i: u32) {
       self.next_seq_c = c;
       self.next_seq_i = i;
   }
   ```

   and amend the doc comment to stop claiming C/I seqs are reconstructible from the event
   stream — they are not.

At minimum, the doc comment must be corrected; shipping it as-is invites the hypervisor
author to assume re-draining rebuilds these counters, which it cannot.

---

## IMPORTANT

### I1 — `read_region` extent walk has unchecked u64 arithmetic (debug panic / release wrap on guest-corrupt manifest)

**File:** `crates/detguest-host/src/manifest.rs:131, 147`

The manifest lives in guest RAM and is guest-written; `Extent::read_from`
(detguest-wire/src/manifest.rs:257) performs **no** validation of `gpa`/`len`. Two
unchecked `u64` operations on that untrusted data:

- Line 147: `self.gm.read(x.gpa + to_skip, chunk)` — `x.gpa + to_skip` can overflow. With a
  guest-supplied `gpa` near `u64::MAX`, this panics in debug builds and wraps in release,
  potentially aliasing an unrelated mapped GPA instead of returning an error.
- Line 131: `let covered: u64 = region.extents.iter().map(|x| x.len).sum();` — an unchecked
  sum. A crafted extent table summing to `> u64::MAX` wraps to a small value and can bypass
  the `covered < region.len` coverage guard.

This contradicts the wire crate's explicit posture ("arbitrary bytes never cause a panic",
detguest-wire/src/ring.rs:36-37) and the host crate's role of treating channel memory as
untrusted-but-bounded (every other access maps failures to `MemError`/`OutOfBounds`).

```rust
// line 131
let covered: u64 = region
    .extents
    .iter()
    .try_fold(0u64, |acc, x| acc.checked_add(x.len))
    .ok_or(RegionReadError::OutOfBounds)?;

// line 147
let src = x.gpa.checked_add(to_skip).ok_or(RegionReadError::OutOfBounds)?;
self.gm.read(src, chunk)?;
```

(Severity is Important, not Critical: the blast radius is the guest's own VM — a guest can
DoS/confuse only its own host worker — but a panic in the hypervisor's drain/read path is
still a robustness defect on guest-controlled input.)

### I2 — `drop_counters` signature deviates from the normative API.md §2 declaration

**File:** `crates/detguest-host/src/channel.rs:229` vs `prompts/docs/guest-sdk/API.md:340`

API.md §2 (line 340) declares:

```rust
pub fn drop_counters(&self) -> DropCounters;
```

The implementation returns `Result<DropCounters, MemError>`. The fallible form is arguably
*better* (the header read can fail on an unmapped page), but it is a normative-signature
deviation and the spec is the stated source of truth for this iteration. Either:

- update API.md §2 to `-> Result<DropCounters, MemError>` and note the rationale, or
- make the impl infallible per spec (the header region is validated-mapped at attach, so an
  `.expect()`/`unwrap_or_default()` is defensible).

Pick one so the spec and code agree; right now a downstream caller written to the spec will
not compile against the crate.

### I3 — `GuestEvent.payload` type deviates from API.md §2 (`EventPayload` vs `OwnedPayload`)

**File:** `crates/detguest-host/src/drain.rs:17-28, 34-36` vs `prompts/docs/guest-sdk/API.md:356-362`

API.md §2 declares `GuestEvent { …, pub payload: EventPayload }` (the borrowed wire enum,
§3.2). The impl introduces a new owned mirror `OwnedPayload` and uses it instead. This is the
*right* engineering call — `drain_events` returns a `Vec` outliving the scratch decode buffer,
so a borrowed payload cannot work — but it is an undocumented divergence from the normative
type. Add `OwnedPayload` to API.md §2 (or annotate `GuestEvent.payload` as "owned mirror of
EventPayload") so the spec reflects reality. Without it, the spec is now wrong about the
public return type of the crate's headline method.
