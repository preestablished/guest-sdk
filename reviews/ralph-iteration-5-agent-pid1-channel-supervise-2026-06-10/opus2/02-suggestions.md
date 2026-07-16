# Suggestions (non-blocking)

### S1 — Ring-I relay `seq` is non-monotonic across a pad boundary (flag for the M3 SDK consumer)

**File:** `crates/detguest-agent/src/channel.rs:240` (`let seq = prod / total as u32;`).

`seq` is derived from the pre-pad producer index divided by the fixed record size. Once a
pad has intervened (when a record does not fit the contiguous tail), `prod` is no longer a
clean multiple of `total`, so `prod / total` skips/repeats values and is not a monotonic
per-record counter. The code comment already concedes seq is "advisory on ring I" and that
"the SDK consumer does not enforce continuity across the two producers". This is **not a
bug today** because the ring-I consumer (the SDK) is M3 work and nothing reads this seq
yet. But it violates §7 rule 3 ("per-ring record `seq` is a monotonically increasing
`u32` owned by that ring's producer"). When the M3 SDK lands, either give the relay its
own monotonic ring-I seq counter (separate from the host's), or make the SDK consumer
explicitly tolerate the discontinuity. Recommend filing a bead so this is not forgotten.

### S2 — `relay_workload_ctrl`'s "temporal exclusivity" two-producer argument deserves a runtime guard

**File:** `crates/detguest-agent/src/channel.rs:216-268`.

The agent appends to ring I continuing the host's producer index, justified by
"the host pushes only while the vCPU is paused and this relay runs only in response to a
ring-C command — temporally exclusive". The reasoning is sound for v1, but it is a subtle
invariant with no assertion. A `debug_assert` that the agent only relays from inside the
ring-C dispatch (e.g., a re-entrancy flag), or a comment cross-referencing the exact
host-side guarantee in determinism-hypervisor docs, would make this less fragile to future
refactors that might call the relay from another context.

### S3 — `mount_all` is not idempotent for `/proc`, `/sys`, `hugetlbfs` (only `/dev` tolerates EBUSY)

**File:** `crates/detguest-agent/src/runtime.rs:34-47`.

Only the devtmpfs mount tolerates `EBUSY`. If `mount_all` ever runs against an environment
where `/proc` or hugetlbfs is already mounted (it never does in the v1 initramfs, and PID
1 never restarts), it returns an error and the agent powers off. This is correct for v1,
but the asymmetry (devtmpfs tolerant, others not) is worth a one-line comment explaining
why devtmpfs is special (kernel auto-mounts it) so a future reader does not "fix" the
others into tolerance and mask a real double-mount.

### S4 — Add `pagesize=2M` to the hugetlbfs mount as defense-in-depth

**File:** `crates/detguest-agent/src/runtime.rs:44`.

`mount("hugetlbfs", "/dev/hugepages", "hugetlbfs")` with null data uses the kernel's
*default* hugepage size. On x86-64 that is 2 MiB, matching `CHANNEL_SIZE = 0x20_0000` and
the spec's "single 2 MiB hugetlb page". But if a future image (or a mis-set
`default_hugepagesz=1G` cmdline) changed the default to 1 GiB, `ftruncate(2 MiB)` on that
mount would fail or over-reserve. Passing `data = "pagesize=2M"` pins the assumption the
channel code depends on. Low severity (the image owns its own kernel config) but cheap.

### S5 — `boot_toml_version` is exact-equality; spec says "unknown MAJOR" (forward-compat note)

**File:** `crates/detguest-agent/src/boot.rs:95-99`.

The check is `version != BOOT_TOML_MAJOR` (exact). API.md §7.2 frames it as "unknown
*major* ⇒ boot fault" with a major/minor convention. With the field being a single
integer today, exact-equality is equivalent to major-only matching, so this is correct now.
But the variable and error message say "major" while the comparison is "whole value" — if a
minor convention is ever encoded (e.g., `1.1` → some integer scheme), this would reject a
forward-compatible minor bump. A short comment ("v1: the version is a bare major; revisit
when a minor convention is introduced") would prevent a future surprise. Already partially
acknowledged in the prompt's own notes.

### S6 — `control.proto_version` equality vs the agent's spoken version is deferred — make the TODO explicit

**File:** `crates/detguest-agent/src/boot.rs:159-165` and
`crates/detguest-agent/src/runtime.rs:138-140`.

`boot.rs` parses `proto_version` but does not compare it to the version the agent speaks;
`autostart_and_ready` punts the whole control leg to M4. That is the right call for M2, but
the parse-time acceptance of any `proto_version` means a misconfigured manifest passes
validation and only fails (or silently mis-drives) at M4. A `// M4: validate proto_version
== agent's control version` marker at the parse site keeps the deferral visible.

### S7 — Doc-comment overstates what paces the loop ("every loop pass + on SIGCHLD")

**File:** `crates/detguest-agent/src/supervise.rs:3-5` and `:397-399`.

The comments attribute the deterministic cadence to "every loop pass + on SIGCHLD", but
what actually bounds the pass rate (and thus the ring-C poll latency when the workload is
silent) is the **10 ms timerfd**; the 100 ms `epoll_wait` timeout is only a backstop that
the timerfd almost always pre-empts. Both are virtual-time and therefore deterministic, so
the §7 claim is *defensible* — no overclaim — but the comment under-describes the timerfd's
role. Worth one sentence: "the 10 ms timerfd sets the ring-C polling cadence; the 100 ms
epoll timeout is a backstop; both are virtual time, hence deterministic." Also worth noting
the timerfd is a latency optimization, not a correctness requirement (the loop would still
be correct, just laggier, with only the 100 ms timeout).
