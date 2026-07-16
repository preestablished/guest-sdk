# Critical & Important Findings

## Critical

None. The suite is empirically green (4/4 in 15.89 s on reverify) and no correctness defect rises to milestone-blocking.

---

## Important

### I-1 — `perf_event_attr` size (112) exceeds the declared struct length (96): 16-byte OOB read of host stack

**File:** `tests/vm/src/harness/icount.rs:19,23-41,53-71`

The `PerfEventAttr` struct as declared ends at `clockid` (offset 92, size 4) — i.e. the
allocation is **96 bytes**. But `ATTR_SIZE = 112` (`PERF_ATTR_SIZE_VER5`) is written into
`attr.size` and the pointer is handed to `perf_event_open`. The kernel's `perf_copy_attr`
reads `attr.size` bytes from the user pointer, so it copies **16 bytes past the end of the
96-byte stack allocation** (the VER4/VER5 tail: `sample_regs_intr` @96, `aux_watermark`,
`sample_max_stack`, `__reserved_2`). I confirmed the offsets against the system header:

```
$ cc check.c && ./check
sizeof(perf_event_attr)=136      # VER8
offsetof clockid=92              # last field of the Rust struct, ends at 96
offsetof sample_regs_intr=96     # exactly where the Rust struct ends
PERF_ATTR_SIZE_VER3 = 96   VER4 = 104   VER5 = 112
```

**Why it's *not* Critical / why it works today:** the OOB bytes feed only
`sample_regs_intr` / `aux_watermark` / `sample_max_stack`, which the kernel ignores unless
the matching feature flags are set (`PERF_SAMPLE_REGS_INTR`, AUX, etc.) — none are. Worst
realistic case is a spurious `EINVAL` if stack garbage lands in a validated reserved field,
which the `fd < 0` check would surface loudly rather than silently corrupt the count. So the
measurement is valid; this is latent UB, not a live miscount.

**Fix (pick one):**

*Option A — make the declared size match the struct (cleanest):* `clockid` is only consulted
when `use_clockid` is set (it isn't), so a VER3-sized struct is the honest declaration.

```rust
/// PERF_ATTR_SIZE_VER3 — exactly the fields this struct declares.
const ATTR_SIZE: u32 = 96;
```

*Option B — pad the struct out to 112 so the buffer is as long as `size` claims:*

```rust
#[repr(C)]
#[derive(Default)]
struct PerfEventAttr {
    // ... through clockid: i32,  (offset 92)
    sample_regs_intr: u64,   // 96
    aux_watermark: u32,      // 104
    sample_max_stack: u16,   // 108
    __reserved_2: u16,       // 110  -> total 112
}
```

Option B is the more future-proof choice (keeps the door open to `use_clockid` for replay
determinism in M3). Either way the invariant to assert is
`ATTR_SIZE as usize == std::mem::size_of::<PerfEventAttr>()` — worth a `const` assert:

```rust
const _: () = assert!(ATTR_SIZE as usize == std::mem::size_of::<PerfEventAttr>());
```

That single `const` assert would have caught this at compile time and is the real fix to
land regardless of A vs B.
