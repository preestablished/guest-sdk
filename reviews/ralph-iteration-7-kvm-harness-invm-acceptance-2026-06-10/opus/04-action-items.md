# Action Items

### Critical
- [ ] None.

### Important
- [ ] [tests/vm/src/harness/icount.rs:19,23-41] `ATTR_SIZE = 112` exceeds the declared `PerfEventAttr` length (96 bytes, ends at `clockid`@92) → 16-byte OOB read of host stack in `perf_event_open`. Benign today (no sampling flags set; works empirically) but latent UB. Fix: either set `ATTR_SIZE = 96` (VER3, matches the struct) or pad the struct to 112 (VER5), and add `const _: () = assert!(ATTR_SIZE as usize == std::mem::size_of::<PerfEventAttr>());` so the mismatch can never recur.

### Suggestions
- [ ] [tests/vm/src/harness/pio.rs:140-141] INIT_GO wrong-size maps to status 1 ("bad GPA"); a size mismatch is neither bad-GPA nor bad-magic per API.md §5. Defensible (already commented "GPA/size"); mirror a hypervisor-defined size code if one ever lands. No change required.
- [ ] [tests/vm/tests/m2_acceptance.rs:153-155] Reword "read inside the Ready doorbell window" — the count actually spans guest-entry → Ready (whole-boot total). Measurement is correct; only the prose is imprecise.
- [ ] [tests/vm/src/harness/mod.rs:217] Document that `rsp = 0x8ff0` has only one 4 KiB page of headroom above `boot_params` (0x7000), safe because the kernel head switches stacks before consuming `rsi`/boot_params.
- [ ] [tests/vm/src/harness/mod.rs:169] Guard `initrd_addr` against `u64` subtraction underflow for future sub-initramfs `mem_size` configs (`.checked_sub(...).expect("initramfs fits in guest RAM")`). Unreachable at the fixed 128 MiB.
- [ ] [tests/vm/src/harness/mod.rs:283-297] Watchdog `pthread_kill` is sound (done-flag + join before return); add a `debug_assert!` or comment to keep the "vcpu runs on the watchdog's target thread" invariant from regressing under a future refactor.
- [ ] [tests/vm/src/harness/pio.rs:127] `(SERIAL_BASE..SERIAL_END).contains(&p) && p == SERIAL_BASE` reduces to `p == SERIAL_BASE`; simplify and note that non-THR 8250 register writes are intentionally dropped (WI).
- [ ] [docs] Note that `emergency_serial` issues OUTs to port 0x3F8 (the harness's 8250 sink, *not* a detcall port). Under the real determinism-hypervisor an unclaimed-port OUT is RAZ/WI per its device map; these OUTs occur only on deterministic error paths (§7.9), so they are determinism-neutral. Worth one sentence in the agent docs to forestall a §7 false alarm.
