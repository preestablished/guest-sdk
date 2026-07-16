# Critical & Important Findings

## CRITICAL

### C1 — `pio.rs:30,61` — `options(nomem)` on the doorbell `OUT` permits the compiler to reorder it before the Release store that publishes the ring record

**File:** `crates/detguest-agent/src/pio.rs:26-32` (`out32`) and `:61-63` (`doorbell`),
in concert with `channel.rs:144-173` (`emit` / `emit_with_doorbell`).

The doorbell is `out32(PORT_DOORBELL, mask)`:

```rust
core::arch::asm!("out dx, eax", in("dx") port, in("eax") value, options(nomem, nostack));
```

`options(nomem)` is a promise to the compiler that the asm block reads and writes **no**
memory observable to the abstract machine. With that promise, the compiler is free to
reorder the `out` instruction across surrounding memory operations — **including atomic
stores** — because it sees no memory dependency and no fence. The doorbell is always
preceded by a `Release` store of the ring-A producer index (inside `Producer::try_push`,
`ring.rs`), and in `emit_with_doorbell` that store publishes a *successfully written*
record (Hello / Ready / WorkloadExited / QuiesceReady):

```rust
// channel.rs
pub fn emit_with_doorbell(&mut self, ...) -> bool {
    let landed = self.emit(...);            // ends in a Release store of prod_a
    (self.doorbell)(ports::DOORBELL_RING_A); // <-- nomem OUT, no fence between
    landed
}
```

There is no `compiler_fence` between the Release store and the `nomem` OUT, so the
compiler may legally emit the `out dx, eax` (the VM exit that tells the host to drain
ring A *now*) **before** the producer-index store is visible. The host's PIO handler
would then drain on a stale index and miss the record — exactly the failure mode the
spec's discipline forbids: ARCHITECTURE.md §2 ("the record is guaranteed visible because
it precedes the write, same discipline as `InjectQuery` before `OUT 0xD384`") and API.md
§5 ("write the record and release-store the producer index **before** `OUT`"). This is a
**compile-time** reordering bug; the x86 `out` instruction's hardware serialization does
not help because the instructions never reach the CPU in the wrong order at runtime —
they are emitted in the wrong order by the compiler. It will not reproduce reliably in
tests (the test doorbell is a plain Rust `fn`, and `-O0` rarely reorders), which makes it
exactly the kind of latent miscompile that surfaces only under release optimization on
real hardware.

The same applies to the `emit` doorbell-retry path: after a *failed* push the doorbell
asks the host to drain so space frees up; if the OUT is reordered before the
consumer-index `Acquire` re-load on the next loop iteration the retry logic still
re-loads `cons` with Acquire, so the retry case is less dangerous — but the
`emit_with_doorbell` "publish then ring" case is a genuine correctness hole.

**Fix** — insert a compiler fence before the port write so the publishing store cannot be
sunk past it, or drop `nomem` on the doorbell OUT (which makes the compiler treat it as a
full memory clobber and is the simplest correct option). A `compiler_fence(SeqCst)` is
the minimal, intent-revealing fix:

```rust
/// Ring the doorbell for the rings in `mask`.
pub fn doorbell(mask: u32) {
    // The record + Release-store of the producer index MUST be visible before
    // this OUT (ARCHITECTURE.md §2 / API.md §5). `out32` is marked `nomem`, so
    // without this fence the compiler may sink the publishing store past the OUT.
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
    out32(ports::PORT_DOORBELL, mask);
}
```

Alternatively, give `out32` itself memory-ordering semantics by removing `nomem`
(keep `nostack`, drop `nomem`, optionally add `preserves_flags` since `out` does not
touch flags). Removing `nomem` is defensible for all detcall OUTs whose purpose is to
make a prior memory write observable to the host (DOORBELL, and the INJECT/QUIESCE_ACK
OUTs the SDK will add later). Note the same `nomem` reasoning will bite the future SDK
`InjectQuery`-before-`OUT 0xD384` path if this helper is reused — fixing it here is the
right layer.

---

## IMPORTANT

### I1 — `channel.rs:216-268` — ring-I relay `seq = prod / total` diverges from the host producer's seq counter, breaks on a tail pad, and reuses one seq for pad + record

**File:** `crates/detguest-agent/src/channel.rs:240` and surrounding `relay_workload_ctrl`.

Ring I has two producers (host + agent quiesce relay — ARCHITECTURE.md §2 table). Per
§7 determinism rule 3, each ring's record `seq` is "a monotonically increasing `u32`
owned by that ring's producer." The host side honors this with an explicit counter
(`detguest-host/src/commands.rs:54` `next_seq_i`, advanced per record incl. pads —
`channel.rs:109` "pads consume one seq"). The agent instead derives:

```rust
let seq = prod / total as u32;   // total = record_len(8) = 24
```

Three problems:

1. **It does not match the host's counter.** The host's `next_seq_i` is a record count
   (0,1,2,…). `prod / 24` is a byte-offset/size quotient. They coincide only on a fresh
   ring with no pads and uniform 24-byte records; after the host has pushed anything, or
   after any wrap pad, the two producers emit conflicting seq values on the same ring.
2. **It is not even self-consistent across a tail pad.** When `needed > total`, the relay
   writes a pad of `tail` bytes (not a multiple of 24) and advances `pos`. The `seq` is
   computed once *before* the pad from the pre-pad `prod`, so after the pad `prod` is no
   longer a multiple of 24 and the next relay's `prod / 24` truncates to a meaningless
   value.
3. **Pad and record share one seq.** `try_push`'s convention (and the host path) is
   "pads consume their own seq first" (`ring.rs:139`, the
   `wrap_inserts_pad_and_seq_stays_monotonic` test asserts no seq gap). The relay encodes
   the pad with `seq` and then the record with the *same* `seq` — violating that
   convention.

The code comment defends this: "seq is advisory on ring I (the SDK consumer does not
enforce continuity across the two producers)." That is probably true for v1 — the
COOP/FORCED quiesce protocol matches `QuiesceReq`/`Resume` by **token**, not seq, and
stale tokens are ignored (ARCHITECTURE.md §6). So this is unlikely to break the demo. But
it is a real §7-rule-3 conformance gap, and "the consumer happens not to look" is a
fragile guarantee for a determinism platform whose whole premise is that ring state is
bit-reproducible and replay-verified.

**Fix** — give `AgentChannel` an explicit ring-I relay seq counter and increment it once
per pad and once per record, mirroring the host producer. Even though the two producers'
sequences still won't be globally monotone (two independent producers on one ring is the
underlying design tension), each producer's own sub-sequence should be monotone and a pad
should consume its own seq:

```rust
// in AgentChannel: add `relay_seq_i: u32`
// in relay_workload_ctrl, replace `let seq = prod / total as u32;` with per-record alloc:
let pad_seq = self.next_relay_seq();      // pad consumes a seq first
// ... encode_pad(&mut pad, tail, pad_seq) ...
let rec_seq = self.next_relay_seq();      // record gets the next
// ... encode_workload_ctrl(&mut buf, rec_seq, vnanos, rec) ...
```

If, instead, the deliberate design intent is that the agent relay must *continue the
host's* seq stream (single logical sequence per ring), then the relay needs to read and
advance a seq value the host also reads/writes through the channel — which it currently
cannot. Either way, `prod / total` is not the right derivation; please replace it and
update the comment to state the chosen invariant precisely.

### I2 — `boot.rs:159-165` / `runtime.rs:135-140` — `[unit.control].proto_version` equality against the agent's spoken version (§7.2) is not checked

**File:** `crates/detguest-agent/src/boot.rs:159-165`.

API.md §7.2 states `proto_version` "must equal the value the agent speaks." The parser
only range-checks it to `0..=u32::MAX` and stores it; the equality check is absent.
`runtime.rs:135-140` does fault when an autostart unit declares *any* control block
("unit.control protocol leg not implemented before M4"), so a mismatched version cannot
slip past the READY gate **for the autostart unit** today. But:

- A `StartWorkload` of a control unit via ring C (M4) would not be gated by that early
  return, and there is no other equality check on the path.
- A non-autostart `[[unit]]` with a stale `proto_version` parses successfully and is
  silently accepted into the manifest.

This is correctly deferred per the prompt ("control proto_version equality check is M4 —
noted?"), and the M2 fault wall makes it currently unreachable for boot. Flagging it as
Important only so it is not lost: when the M4 control leg lands, the equality check must
be added (ideally in `parse` so an over-version manifest is a boot fault per §7.2, not a
runtime surprise), and the `protocol == "refwork-ctl"` branch is the natural place to
also assert `proto_version == AGENT_REFWORK_CTL_VERSION`. A `// TODO(M4): §7.2 requires
proto_version == agent's spoken version` marker at boot.rs:159 would make the deferral
explicit.
