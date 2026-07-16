# Review Overview — Iteration 7: KVM Harness + In-VM M2 Acceptance

- **Branch:** `ralph/iteration-7-kvm-harness-invm-acceptance`
- **Base:** `main`
- **Date:** 2026-06-10
- **Reviewer:** Claude Opus
- **Scope:** `tests/vm/src/harness/{mod,memslot,pio,x86,icount}.rs`, `tests/vm/tests/m2_acceptance.rs`, agent fixes (`crates/detguest-agent/src/{pio,runtime}.rs`), `image/{build.sh,kernel.config}`, `.github/workflows/ci.yaml`.

## Summary

This iteration adds the repo's own minimal KVM test harness (raw `kvm_ioctls` bring-up of the pinned 6.12.93 kernel + initramfs into the real `detguest-agent`) and the agent/image fixes the first real boots shook out (PID1 stdio via `/dev/console` after devtmpfs; panic-proof `console_log` + IOPL-guarded emergency serial; `CONFIG_X86_IOPL_IOPERM`; `hugepages=4` cmdline; in-VM CI PATH fix). The harness exercises the *real* `detguest-host` crate for all channel work rather than reimplementing it, which is the right call for measurement fidelity. I verified the long-mode bring-up (GDT/page-table/CR/EFER bits), the detcall PIO handler against API.md §5, the e820/initrd layout math, the watchdog/halt-detection design, and the perf-counter struct layout against `linux/perf_event.h`. **I reran the suite on this Intel box: all 4 M2 acceptance tests pass in 15.89 s** (real KVM boots), confirming the empirical-green claim. The GDT encodings, `seg()` flag extraction, page-table flags, and initrd 2 MiB alignment all check out exactly. The one real correctness defect is a `perf_event_attr` size/struct-length mismatch (`ATTR_SIZE = 112` against a 96-byte struct) — a latent 16-byte out-of-bounds read of host stack that is *benign under the current flag set* (it works empirically) but should be fixed for hygiene. Everything else is sound; the doc honesty around the icount strict-gate deferral is exemplary.

## Verdict

**APPROVE**

The milestone gate is met and empirically green. The perf struct-size mismatch is the only Important finding; it does not invalidate the measurement (the counter reads are correct) but is a latent UB that warrants a one-line fix before this pattern is copied into M3.

## Stats

| Category | Count |
|---|---|
| Critical | 0 |
| Important | 1 |
| Suggestions | 6 |
| Positive notes | 8 |
| Files reviewed | 13 (1441 insertions) |
| Acceptance tests | 4/4 passing (15.89 s, reverified) |
