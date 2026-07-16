# Action Items

### Critical
- [ ] (none)

### Important
- [ ] [image/KERNEL.md / issue #1] Document that the canonical deterministic cmdline MUST include `hugepages>=1`: the agent allocates its 2 MiB detchannel from hugetlbfs (`crates/detguest-agent/src/channel.rs:45-92`) before any runtime sysctl path exists, so an empty pool means the guest never reaches Hello. The harness's `tests/vm/src/harness/mod.rs:62` `hugepages=4` is a workaround for a requirement this repo now exports. (I-1)
- [ ] [image/kernel.config / image/build.sh:42-49] The built kernel has `# CONFIG_DEVMEM is not set`; the M3 SDK's `init()` maps the pv-pad MMIO window via `/dev/mem` (API.md §1). Add `CONFIG_DEVMEM=y` (and plan for `STRICT_DEVMEM=n` or a `mem=`/`memmap=` reservation of GPA `0xD000_1000`), or file the gap on issue #1 so M3 doesn't rediscover it on first in-VM boot. (I-2)

### Suggestions
- [ ] [tests/vm/src/harness/mod.rs:222,401] Make the process-wide SIGALRM install explicit/single-shot (`std::sync::Once`) or document that the harness owns SIGALRM for the test-binary lifetime; the watchdog race itself is verified safe. (S-1)
- [ ] [tests/vm/src/harness/pio.rs:60,82] Note that the pre-commit `INIT_GO` readback sentinel `u32::MAX` is intentionally outside the §5 status enum (the agent never reads before committing). (S-2)
- [ ] [tests/vm/tests/m2_acceptance.rs:340-366] Assert `LogLine.level` mapping (stdout vs stderr), not just per-stream message order — the suite comment promises "stream/level framing". (S-3)
- [ ] [tests/vm/tests/m2_acceptance.rs:236-256 / ci.yaml] Run the in-VM tier with `--nocapture` (or write a step summary) so the 10-boot icount spread — the headline of the non-strict gate — is actually recorded in CI. (S-4)
- [ ] [tests/vm/tests/m2_acceptance.rs:197-203] Soften the `Hello.vnanos` comment to "guest time from kernel timekeeping init (≈boot)": `CLOCK_MONOTONIC_RAW` excludes the pre-`timekeeping_init` firmware/decompression window. (S-5)
