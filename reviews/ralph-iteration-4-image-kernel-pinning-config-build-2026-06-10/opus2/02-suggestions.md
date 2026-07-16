# Suggestions (non-blocking)

### S1 — cache key omits build.sh and the toolchain image → stale-bzImage trap

- **File:** `image/build.sh:96-98` (`build_key`) + `image/KERNEL.md:39-44` (cache-key description).
- **What/why:** `build_key()` hashes `KERNEL_VERSION + kernel.config` only. It does NOT cover
  `build.sh` itself (so editing `REQUIRED_SET`, the merge/olddefconfig pipeline, or any
  config-affecting logic silently reuses a stale `bzImage`) nor the toolchain image. A contributor
  who tightens the determinism set and re-runs `image/build.sh kernel` gets the cached old kernel
  with no warning. KERNEL.md §"Build artifact caching" describes the key as
  `sha256(version + config fragment)` — so doc and code agree, but both share the same blind spot.
- **Snippet:**
  ```bash
  build_key() {
    { echo "$KERNEL_VERSION"; cat "${SCRIPT_DIR}/kernel.config" "${BASH_SOURCE[0]}"; } \
      | sha256sum | cut -d' ' -f1
  }
  ```
  (Hashing the script is cheap insurance; the toolchain image is harder — see S2.)

### S2 — toolchain inputs are unpinned and committed-once

- **File:** `image/build.sh:50` (`DOCKER_IMAGE=ubuntu:24.04`) and `:69-75` (apt install, no versions).
- **What/why:** `ubuntu:24.04` is a moving tag, and `apt-get install build-essential flex bison ...`
  pulls whatever versions the mirror serves the day the image is first built. The result is
  `docker commit`'d into `detguest-kernel-build:24.04` and cached forever, so toolchain drift is
  invisible after first build — and a *different* contributor's first build can get a *different*
  gcc/binutils, yielding a different `bzImage`. For a determinism-focused image this is the
  weakest reproducibility link. At minimum pin the base by digest
  (`ubuntu:24.04@sha256:...`) and note the toolchain is not version-pinned; ideally pin apt
  package versions or fold the toolchain build/version into the cache key.

### S3 — print-lines exit code 7 is not guaranteed under a closed stdout

- **File:** `tests/vm/workloads/src/bin/print_lines.rs:21-29`.
- **What/why:** Rust sets `SIGPIPE` to `SIG_IGN`, so a `println!` to a closed pipe returns `EPIPE`,
  and `println!` *panics* (process exits 101), not 7. In the agent's pipe model the read end stays
  open for the workload's lifetime, so in practice EXIT_CODE=7 holds — but the workload's whole
  point is asserting `WorkloadExited { exit_code: 7 }`, so the latent dependency on "the reader
  never closes early" is worth a one-line comment, or use explicit `writeln!(...).ok()` /
  `BufWriter` + `flush().ok()` to make the exit code robust.

### S4 — autostart sleep loop relies on virtual-time correctness; pause() is stronger

- **File:** `tests/vm/workloads/src/bin/autostart_trivial.rs:18-22`.
- **What/why:** `loop { thread::sleep(3600s) }` wakes once an hour even under perfect virtual time,
  each wake being a tiny icount perturbation. The doc comment explicitly wants "never exits" with
  no perturbation. A raw `pause()`-style block (e.g. `loop { thread::park(); }` after no unpark, or
  blocking on an fd that never becomes ready) parks with zero periodic wakeups and is a cleaner
  "park forever" primitive for the READY-icount measurement. Minor — sleep is fine if virtual time
  is exact, but park is closer to the stated intent.

### S5 — add `#![forbid(unsafe_code)]` to the workloads

- **File:** `tests/vm/workloads/src/bin/*.rs` (both bins).
- **What/why:** These are trivial, all-safe binaries baked into a determinism-critical image. A
  `#![forbid(unsafe_code)]` at the top of each makes the "no unsafe, fully deterministic" property
  machine-checked and future-proof against drift. Currently neither file has it (verified).

### S6 — SC2001 style + portability note

- **File:** `image/build.sh:108` (`sed 's/^# \(CONFIG_...\) is not set$/\1/'`).
- **What/why:** shellcheck flags SC2001; a bash parameter-expansion is faster and lint-clean:
  ```bash
  local sym="${line#\# }"; sym="${sym% is not set}"
  ```
  Separately, `find -print0`, `sort -z`, and `stat -c%s` are GNU-specific. The build target is
  Linux-only so this is acceptable, but a one-line "requires GNU coreutils/findutils" note in the
  header would save a confused macOS contributor.
