## Action Items

### Critical
- [ ] (none)

### Important
- [ ] [image/build.sh:154-162 + :30-33; image/KERNEL.md:34] initramfs is NOT byte-reproducible — file mtimes leak into the newc cpio (proven: two same-content builds differ at byte 53), yet the header comment and log say "reproducible/deterministic." Either canonicalize mtimes (`find "$root" -exec touch --no-dereference -d @0 {} +`) and `chmod 0755 "$root"` (stop inheriting the stage-dir mode onto guest `/`), OR weaken the comment+log to match reality.
- [ ] [image/build.sh:71-75] docker fallback leaves a fixed-name `detguest-kbuild-tmp` container on any failed/interrupted first run, wedging every later build with a name conflict under `set -e` (happened this session). Add `docker rm -f detguest-kbuild-tmp 2>/dev/null || true` before `docker run`, and/or use `docker build` with a tiny Dockerfile to avoid the named-container lifecycle.

### Suggestions
- [ ] [image/build.sh:96-98; image/KERNEL.md:39-44] Cache key hashes version+config only — fold `${BASH_SOURCE[0]}` into `build_key()` so config-pipeline edits don't silently reuse a stale bzImage.
- [ ] [image/build.sh:50, :69-75] Pin the toolchain: `ubuntu:24.04@sha256:...` (digest) and ideally apt package versions; the committed-once `detguest-kernel-build:24.04` hides toolchain drift, the weakest reproducibility link.
- [ ] [tests/vm/workloads/src/bin/print_lines.rs:21-29] exit code 7 isn't guaranteed if stdout closes early (Rust SIG_IGNs SIGPIPE → println! panics → exit 101). Add a comment or use `writeln!(...).ok()` to make it robust.
- [ ] [tests/vm/workloads/src/bin/autostart_trivial.rs:18-22] Prefer a true park-forever (`thread::park()` loop / block on a never-ready fd) over a 1-hour `sleep` loop to remove periodic-wake icount perturbation.
- [ ] [tests/vm/workloads/src/bin/*.rs] Add `#![forbid(unsafe_code)]` to both bins to machine-check the "no unsafe, deterministic" property.
- [ ] [image/build.sh:108] SC2001: replace the `sed` with `${line#\# }` / `${line% is not set}`; add a "requires GNU coreutils/findutils" note for `find -print0`/`sort -z`/`stat -c%s`.
