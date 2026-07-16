# Critical & Important findings

## Critical

**None.** The landed harness, agent fixes, kernel-config additions, and the M2
suite are correct; both hosted and in-VM lanes pass.

---

## Important

### I-1 — `hugepages>=1` is now a REQUIREMENT this repo exports to the canonical cmdline, documented nowhere the hypervisor team will see it

- **Where:** `tests/vm/src/harness/mod.rs:62` (cmdline `"… hugepages=4"`) and the
  comment at `mod.rs:60-61`; the constraint originates in
  `crates/detguest-agent/src/channel.rs:45-92` (the agent mmaps a 2 MiB hugetlbfs
  file for the detchannel) and `crates/detguest-agent/src/runtime.rs:64-65`
  (`mount hugetlbfs`).
- **What:** The agent's channel allocation is a `MAP_SHARED` mmap of a 2 MiB
  hugetlbfs file (`CHANNEL_PATH = /dev/hugepages/detchannel`). With an empty
  hugepage pool and no runtime sysctl path (tinyconfig has no
  `/proc/sys/vm/nr_hugepages` writer wired the way the agent uses it), that mmap
  cannot be backed — the agent cannot bring up the channel and cannot boot. The
  harness works around this with `hugepages=4` on its **harness-local** cmdline,
  which is correct and inside the issue-#1 clean-room boundary. **But this is no
  longer a harness convenience — it is a hard precondition the agent imposes on
  *any* cmdline, including the canonical deterministic cmdline that
  determinism-hypervisor owns.** That requirement is not stated in
  `image/KERNEL.md` (which only says "cmdline is hypervisor-owned, see issue #1")
  nor recorded on issue #1. The hypervisor team will rediscover it "the hard way"
  exactly as `CONFIG_X86_IOPL_IOPERM` was rediscovered this iteration.
- **Why Important (not Critical):** the M2 gate is green and the harness is
  self-consistent; the risk is entirely on the M3 / hypervisor-integration handoff.
- **Fix (docs, no code change to the harness):** Add a consumer-facing note to
  `image/KERNEL.md` and a comment on issue #1. Suggested KERNEL.md addition under
  "Consumers" / a new "Exported cmdline requirements" subsection:

  ```markdown
  ## Cmdline requirements this repo exports

  The kernel cmdline is owned by determinism-hypervisor (issue #1), but the
  detguest-agent imposes one hard requirement on whatever cmdline boots it: the
  hugepage pool must be pre-populated at boot. The agent allocates its 2 MiB
  detchannel from hugetlbfs (`/dev/hugepages/detchannel`) before any runtime
  sysctl path is available, so the *canonical* cmdline MUST include
  `hugepages>=1` (the harness uses `hugepages=4`). Without it the agent's
  channel mmap is unbacked and the guest never reaches Hello.
  ```

  Optionally, harden the agent itself: on channel-alloc `ENOMEM`/`SIGBUS`, emit a
  targeted `console_log("channel alloc failed: empty hugepage pool — cmdline needs hugepages>=N")`
  so the failure is self-explaining instead of a silent power-off.

### I-2 — Built kernel has `CONFIG_DEVMEM is not set`; the M3 SDK's `/dev/mem` pv-pad mapping (API.md §1) will fail

- **Where:** `image/build/linux-6.12.93/.config` → `# CONFIG_DEVMEM is not set`;
  the M3 consumer is API.md §1 `init()`: *"maps the pv-pad MMIO window via
  /dev/mem (base GPA 0xD000_1000) … for poll_input/frame_mark"*. The kernel-config
  diff in this branch (`image/kernel.config`, `image/build.sh`) adds
  `CONFIG_X86_IOPL_IOPERM` but not `CONFIG_DEVMEM`.
- **What:** This iteration's harness stubs pv-pad in the VMM (`harness/pio.rs`
  `pvpad_read`/`pvpad_write`), so M2 does not exercise `/dev/mem` and is unaffected.
  But the real in-guest SDK opens `/dev/mem` to mmap the pv-pad window. Without
  `CONFIG_DEVMEM=y` there is no `/dev/mem` device node, so `init()` will fail at
  the pv-pad map step the first time M3 runs the SDK in this kernel — another
  "found the hard way on first in-VM boot" loop.
- **Note on STRICT_DEVMEM:** even with `CONFIG_DEVMEM=y`, `CONFIG_STRICT_DEVMEM=y`
  (Linux default) blocks `/dev/mem` access to RAM and to most non-reserved
  MMIO ranges, which can also reject the pv-pad GPA mapping depending on how the
  hypervisor marks that range in e820/memmap. M3 will likely need
  `CONFIG_DEVMEM=y` **and** either `CONFIG_STRICT_DEVMEM=n` or a `mem=`/`memmap=`
  reservation for `0xD000_1000`. Flag both now.
- **Why Important (not Critical):** out of scope for the M2 acceptance this branch
  delivers; it is a forward-looking gap for M3. But it belongs on the record from
  this iteration because the kernel-config surface is being actively edited here.
- **Fix:** Either add to `image/kernel.config` + `image/build.sh REQUIRED_SET`
  now (preferred — keeps "exactly one kernel build" valid for M3):

  ```diff
  # image/kernel.config — alongside the IOPL_IOPERM block
  +# ---- pv-pad MMIO mapping (M3 SDK: /dev/mem mmap of GPA 0xD000_1000) ----
  +# API.md §1 init() maps the pv-pad window via /dev/mem; tinyconfig disables
  +# the node. STRICT_DEVMEM (kernel default) additionally gates the range, so
  +# the M3 demo will need a mem reservation or STRICT_DEVMEM=n as well.
  +CONFIG_DEVMEM=y
  +# CONFIG_STRICT_DEVMEM is not set
  ```

  …or, if you prefer to keep the kernel surface minimal until M3 actually needs
  it, file a bead/issue-#1 note so it is not rediscovered. Either way, get it on
  the record.
