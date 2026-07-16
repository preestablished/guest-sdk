# Positive notes

### P-1 — `assert_required_set` absent-symbol logic is correct and non-obvious (`build.sh:101-119`)

The single sharpest part of the script. kconfig *omits* a symbol entirely when its deps are
unmet (rather than emitting `# X is not set`), so a naive `grep -qxF "# CONFIG_X is not set"`
would spuriously fail. The author handles this precisely: for a disabled requirement it only
flags a violation when `^CONFIG_X=` is actually present. I verified against the real built
`.config` that `CONFIG_MIGRATION`, `CONFIG_SWAP`, and `CONFIG_RANDOMIZE_BASE` are **absent**
(not "is not set"), and the assertion correctly treats absence as satisfied. The `sed`
symbol-extraction (`s/^# \(CONFIG_[A-Z0-9_]*\) is not set$/\1/`) is anchored and sound. The
comment at lines 105-107 explaining *why* (SWAP without BLOCK, RANDOMIZE_BASE without
RELOCATABLE) is exactly the context a future maintainer needs.

### P-2 — Cache key actually covers what affects the bzImage (`build.sh:97-99`, `124`)

`build_key = sha256(KERNEL_VERSION + kernel.config)`. I recomputed it by hand and it matches
the stamped `build/.kernel-build-key` byte-for-byte. Both inputs that determine the bzImage
(version → which source, fragment → which config) are in the key; the cmdline (correctly) is
not part of the build. The skip path at line 124 gates on bzImage existence *and* key match,
so a deleted artifact also forces a rebuild. This is the right invalidation surface.

### P-3 — Digest verification ordering: download → verify → extract, with a refuse-to-build on mismatch (`build.sh:82-95`)

The SHA256 is checked before any extraction, and a mismatch `die`s with "refusing to build
unpinned source" rather than proceeding. The combined `[[ ! -f ]] || ! sha256sum -c` guard
also re-verifies an *existing* tarball before reuse, so a corrupted cached download is caught,
not trusted. This is the correct security posture from the shell-security research notes
("verify pinned digest before use; refuse on mismatch").

### P-4 — Clean-room cmdline boundary is fully respected — no leakage

Per the prompt's explicit ask: I grepped the whole diff and the built tree for cmdline
strings (`console=`, `root=`, `init=`, `norandmaps`, `nokaslr`, append/cmdline). The repo sets
the kernel cmdline **nowhere**. KERNEL.md and kernel.config both call this out and point at
issue #1 / determinism-hypervisor §2.3. The boundary is clean.

### P-5 — Initramfs determinism plumbing is mostly right (`build.sh:163-166`)

`find . -print0 | LC_ALL=C sort -z | cpio --null -o -H newc --reproducible --owner=0:0` is the
correct recipe for order-, locale-, owner-, and timestamp-independence. (The remaining
umask-mode gap is I-2 — but the hard parts here, NUL-safe sorted ordering under a pinned
locale and forced `0:0` ownership, are done correctly.)

### P-6 — Static-link guard distinguishes `statically linked` vs `static-pie linked` (`build.sh:147-151`)

Rust+musl can emit either form depending on toolchain version; the `grep -Eq
'static(ally|-pie) linked'` accepts both and the comment says why. The guard is wrapped in
`command -v file` so the build degrades gracefully where `file` is absent. Good attention to a
real footgun (a dynamically-linked agent would fail to exec as PID 1 in the initramfs).

### P-7 — Workloads are genuinely determinism-clean and well-documented

Both binaries match ARCHITECTURE.md §7: `print-lines` uses only fixed strings + a fixed
nonzero exit (7) and its doc correctly states the harness must assert *per-stream* sequences
(not a global interleave) — an easy thing to get subtly wrong, gotten right.
`autostart-trivial` parks in `thread::sleep` (CLOCK_MONOTONIC `clock_nanosleep`, fully under
the hypervisor's virtualized timer — no wall-clock or entropy read) and deliberately never
exits, matching the M2 "READY-point icount across 10 boots with a running unit" gate. The
`Cargo.toml` documents the exact musl cross-compile invocation and the crate builds clean on
the host (`cargo build -p detguest-workloads --release` — verified).

### P-8 — Bead/spec traceability throughout

Nearly every file and section cites the governing bead ID and spec anchor (M2 work item,
ARCHITECTURE §4/§5/§7, API.md §7). This makes the change reviewable against the normative docs
without guesswork and will make the eventual M2/M3 acceptance mapping straightforward.
