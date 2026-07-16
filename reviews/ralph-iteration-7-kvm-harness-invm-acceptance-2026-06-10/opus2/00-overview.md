# Review — KVM harness + M2 in-VM acceptance (2nd reviewer)

- **Branch:** `ralph/iteration-7-kvm-harness-invm-acceptance`
- **Date:** 2026-06-10
- **Reviewer:** Claude Opus (2nd reviewer) — systems / signals / measurement-validity focus
- **Base:** `main`

## Summary

This branch lands a self-contained KVM test harness (`tests/vm/src/harness/*`) that
direct-boots the repo's pinned 6.12.93 kernel + initramfs, services the detcall PIO
ports against the real `detguest-host` crate, stubs the pv-pad MMIO latch, and counts
guest-only retired instructions via `perf_event_open`. It is driven by a 4-test M2
acceptance suite (`tests/vm/tests/m2_acceptance.rs`) covering boot-to-Hello within one
guest second, Ready-point icount across 10 boots, graceful shutdown power-off, and the
print-lines stdout/stderr/exit-code workload. Supporting fixes: agent PID1 stdio
wiring + an `emergency_serial`/`console_log` no-panic diagnostics path, `CONFIG_X86_IOPL_IOPERM=y`
so the agent's first detcall OUT is not a GPF, a `hugepages=4` harness cmdline so the
agent's hugetlbfs channel alloc has a 2 MiB pool, and a CI `$GITHUB_PATH` fix. The
work is careful, the SAFETY/why comments are unusually good, the detcall handler maps
the API.md §5 register table faithfully, and the watchdog signal dance is in fact
race-free as the inline comment claims. The two findings worth raising are not bugs in
the landed code but **undocumented requirements this repo now exports to the
hypervisor/M3 teams**: (1) the canonical deterministic cmdline MUST carry
`hugepages>=1` or the agent cannot boot, and (2) the built kernel has
`CONFIG_DEVMEM is not set`, which the M3 SDK's `/dev/mem` pv-pad mapping (API.md §1)
will trip over.

## Verdict

**APPROVE** — the landed harness and fixes are correct and the M2 gate is green
(4/4, reconfirmed on this box). The two Important items are documentation/export-surface
gaps, not defects in this diff; they are flagged so they land before M3 rather than
being rediscovered "the hard way" again.

## Validation performed

- `cargo test --workspace` (hosted lanes): **green** — 22 + 19 + 38 + 9 + 8 + 1 + 1 passing
  across crates, 0 failures.
- `DETGUEST_VM_TESTS=1 cargo test -p detguest-vmtest --test m2_acceptance -- --ignored --test-threads=1`:
  **4 passed; 0 failed** in 16.59 s (single permitted rerun).
- Inspected built `image/build/linux-6.12.93/.config`: `CONFIG_X86_IOPL_IOPERM=y` ✓,
  `CONFIG_HUGETLBFS=y`/`CONFIG_HUGETLB_PAGE=y` ✓, **`# CONFIG_DEVMEM is not set`**,
  `# CONFIG_IO_URING is not set`.
- Cross-checked the detcall handler against API.md §5 register table and the
  `detguest-wire::ports` constants — exact match on all seven ports.

## Stats

- Files changed: 14 (+1441 / −12)
- New harness modules: `mod.rs`, `icount.rs`, `memslot.rs`, `pio.rs`, `x86.rs` (+ `lib.rs` re-export)
- New test file: `tests/vm/tests/m2_acceptance.rs` (4 in-VM tests)
- Findings: **0 Critical**, **2 Important**, **5 Suggestions**, **6 positive notes**
