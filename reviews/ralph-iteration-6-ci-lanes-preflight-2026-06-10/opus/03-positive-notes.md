# Positive Notes

### P1 — Consistent dual-checkout for the `../control-plane` path dep

**File:** `.github/workflows/ci.yaml` — every job; `fuzz.yaml:19-22`

Every single job checks out both `preestablished/guest-sdk` (into `guest-sdk`) and
`preestablished/control-plane` (into `control-plane`), and `defaults.run.working-directory:
guest-sdk` (`ci.yaml:16-18`) makes the relative `../control-plane/crates/determinism-proto`
path dep (`Cargo.toml:30`) resolve correctly. This is exactly right and easy to get
wrong — the sibling layout is reproduced faithfully in CI. Verified the path dep is
real and consumed only by `m0-proto-client`.

### P2 — Loom RUSTFLAGS matches the test's own cfg gate

**File:** `.github/workflows/ci.yaml:68-69, 78`

The loom job sets `RUSTFLAGS: --cfg loom` and runs `--test loom_ring --release`, and
`crates/detguest-wire/tests/loom_ring.rs` opens with `#![cfg(loom)]`. Without the flag
the test compiles to nothing and would silently pass; the workflow and the test agree.
`--release` is also the correct choice for loom (keeps interleaving exploration
tractable). Well done.

### P3 — Musl lane verifies static linkage, not just that it built

**File:** `.github/workflows/ci.yaml:96-99`

The job doesn't just cross-build for musl — it runs the produced binary with `--check`
(which `crates/detguest-agent/src/main.rs:6` implements as a clean version print + exit
0) *and* asserts `file ... | grep -E 'static(ally|-pie) linked'`. That grep catches the
real failure mode (a musl target that accidentally dynamic-links), and the alternation
correctly handles both classic `statically linked` and modern `static-pie linked`
`file(1)` wordings. Running a musl-static binary directly on the glibc hosted runner is
valid (no loader dependency), so the `--check` smoke run is sound.

### P4 — Correct workspace tiering: vmtest in-workspace, fuzz excluded

**File:** `Cargo.toml` (members/exclude) ↔ workflow lanes

`tests/vm` (detguest-vmtest) is a normal member so hosted lanes fmt/clippy/build it,
while its KVM tests are `#[ignore]` + `DETGUEST_VM_TESTS=1` double-gated and only the
`in_vm` job opens both gates (`ci.yaml:127-129`). `fuzz/` is `exclude`d and is its own
workspace root, so it never pollutes `cargo test --workspace`, and the fuzz workflow
drives it standalone. The CI structure mirrors the workspace structure precisely.

### P5 — Preflight FAIL-accumulation pattern is correct under `set -uo pipefail`

**File:** `scripts/intel-preflight.sh:6-10, 81-85`

Deliberately omitting `-e` is the right call: each check sets `FAIL=1` via `fail()`
rather than aborting, so the script reports *all* failing gates in one run instead of
dying at the first. The final `if [[ $FAIL -ne 0 ]]` exits 1. `set -u` is safe because
every variable that could be unset is read with a default (`${api:-}`, `${para:-...}`).
I traced every command path — none can exit the script early or spuriously. Verified
live: exits 0 with all gates green on this machine.

### P6 — Empty-var guards before numeric comparisons

**File:** `scripts/intel-preflight.sh:34, 45`

`[[ "${api:-}" == "12" ]]` and `[[ -n "$para" && "$para" -le 1 ]]` both guard against
empty/unreadable values *before* doing the comparison, so a missing
`/proc/sys/kernel/perf_event_paranoid` or a failed ioctl produces a clean `FAIL`
message rather than a bash arithmetic error. This is the kind of detail that usually
bites preflight scripts; it's handled.

### P7 — KVM probe avoids a compiled dependency

**File:** `scripts/intel-preflight.sh:25-41`

Using a tiny inline `python3` `fcntl.ioctl(fd, 0xAE00)` for `KVM_GET_API_VERSION`
(value `0xAE00` is correct) sidesteps shipping/compiling a C probe on the runner, with
a `command -v python3` guard and stderr suppression. Pragmatic and portable for a gate
script.

### P8 — Toolchain/component/target syntax is all correct

**Files:** `ci.yaml:29-30, 45/59-60, 89-90`; `fuzz.yaml:23`

`dtolnay/rust-toolchain@stable with: { components: "rustfmt, clippy" }`,
`@nightly with: { components: miri }` (component name `miri` is correct; the action
sets nightly as the job default so `cargo miri`/`cargo +nightly` resolve), and
`@stable with: { targets: x86_64-unknown-linux-musl }` all use the right input keys.
`Swatinem/rust-cache@v2 with: { workspaces: guest-sdk/fuzz }` is valid — `fuzz/Cargo.lock`
exists at that path. `miri test --lib ring` is valid since `pub mod ring` is exported.
