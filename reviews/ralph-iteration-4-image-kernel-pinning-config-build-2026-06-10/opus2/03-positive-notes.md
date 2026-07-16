# Positive Notes

### P1 — Final-config re-assertion of the determinism set is the right defensive pattern
`image/build.sh:34-47` + `:101-118` (`assert_required_set`). Rather than trusting that the fragment
"sticks," the script re-greps the *final* `.config` after `olddefconfig` and hard-fails if any
determinism knob flipped. I confirmed every required line holds in
`image/build/linux-6.12.93/.config`. The handling of "disabled-by-absence" is genuinely correct and
subtle: it accepts a symbol being *absent* (deps unmet) as satisfying "not set," and only flags a
violation when the symbol is explicitly `=...`. Verified live: MIGRATION / SWAP / RANDOMIZE_BASE /
NUMA are all absent (unmet deps) and correctly pass, while the assertion would still catch an
accidental enable.

### P2 — merge_config `-m` semantics are used correctly (the classic trap is avoided)
`image/build.sh:142`. I read `scripts/kconfig/merge_config.sh` on disk
(`:104-181`): with `-m` and no `-O`, `OUTPUT` defaults to `.`, `KCONFIG_CONFIG` is `.config`, and
the merged result is written *back* to `.config` (then it exits without running make). So the
subsequent `make olddefconfig` reads the merged file, not the original — exactly what's intended.
Many hand-rolled merge_config invocations get this wrong (expecting `.config.merged`); this one is
right.

### P3 — Hard SHA256 gate refuses to build unpinned source
`image/build.sh:88-94` (`fetch_kernel`). The digest is checked both on the cached tarball and again
after download, with an explicit `die "... refusing to build unpinned source"`. The pin is
duplicated in `KERNEL.md` and `build.sh` with a clear "bump both together" instruction
(`:11-13`). The provenance note (`KERNEL.md:43-44`, digest from `sha256sums.asc`, dated 2026-06-10)
is exactly the kind of traceability this deserves.

### P4 — Deterministic cpio ordering and root:root ownership
`image/build.sh:159-161`. `find -print0 | LC_ALL=C sort -z | cpio --null -H newc --reproducible
--owner=0:0` gives a stable, locale-independent entry order and forces root ownership regardless of
the build user's uid (verified: `-u $(id -u)` containerization notwithstanding, all entries are
`root root`). The `/init -> /sbin/detguest-agent` symlink is present and correct in the produced
archive. (Reproducibility caveat re: mtimes is in 01-important — but the *ordering/ownership* half
is done well.)

### P5 — Config fragment scoping and cmdline ownership are explicit and consistent
`image/kernel.config:1-12` and `image/KERNEL.md:3-11`. The fragment restricts itself to
`CONFIG_*` build options and repeatedly, consistently disclaims the kernel cmdline as
hypervisor-owned (issue #1), matching the spec. No paravirt/kvmclock pulled in (verified absent) —
the right call for a deterministic time source. CONFIG_64BIT=y correctly overrode tinyconfig's
32-bit default (verified X86_64=y in final config).

### P6 — Workloads are honestly minimal and self-documenting
`tests/vm/workloads/src/bin/*.rs`. Both bins carry precise doc comments tying their behavior to the
exact M2 assertions (per-stream LogLine sequences, `WorkloadExited{exit_code:7}`, READY-icount
across 10 boots). The Cargo.toml documents the exact musl cross-compile invocation
(`tests/vm/workloads/Cargo.toml:11-13`), `publish = false` is set, and `Cargo.lock` was updated.
They build clean on the host target.

### P7 — LTS rationale checks out
`image/KERNEL.md:24-27`. 6.12 is a real longterm series; "projected maintenance into 2027+" is
accurate and conservative — the actual projected EOL is Dec 2028 (kernel.org). The
"tarball+SHA256 over git clone" rationale (no 5 GiB history in CI) is sound.
