# Critical & Important Findings

## Critical

None. The build produces a correct, bootable artifact; the determinism config set is enforced in
the final `.config`; the merge_config `-m` path writes back to `.config` correctly (no
`.config.merged` trap); no path traversal or unpinned-source risk (SHA256 gate is hard-fail).

---

## Important

### I1 — initramfs is NOT byte-reproducible, but the code/comments claim it is

- **File:** `image/build.sh:154-162` (`cmd_initramfs`, the cpio assembly) and the header comment
  `image/build.sh:30-33` ("reproducible newc cpio initramfs" intent), plus `image/KERNEL.md:34`
  ("outputs (`bzImage`, `initramfs.cpio`)" framed as cached/stable).
- **Severity:** Important (correctness-of-claim; reproducibility is a stated platform value).
- **Description:** `cpio ... --reproducible --owner=0:0` only makes the archive
  *device-independent* (zeroes dev/ino) and forces ownership. It does **not** zero mtimes — GNU
  cpio has no mtime flag. File mtimes from the stage dir flow straight into the newc headers. I
  proved this: two builds from same-content stage dirs with different file mtimes produced cpios
  that `cmp` reports differ at byte 53 (the first header's mtime field). The log line literally
  says "deterministic order" and the header comment says "reproducible," which a reader will trust.
  A second non-determinism vector compounds this: `cp -a "${stage}/." "$root/"` propagates the
  stage directory's own mode onto `$root`, so a 0700 stage dir (e.g. a `mktemp -d` stage) makes the
  guest rootfs `/` become `0700` — host-umask/mktemp-dependent and reflected in the archive (I
  observed `drwx------` for `.` in the produced cpio).
- **Fix:**
  ```bash
  local root="${BUILD}/initramfs-root"
  rm -rf "$root"; mkdir -p "$root"/{proc,sys,dev,dev/hugepages,run,etc/detguest,sbin}
  cp -a "${stage}/." "$root/"
  chmod 0755 "$root"                                   # don't inherit stage-dir mode onto guest /
  ln -sf /sbin/detguest-agent "${root}/init"
  # Canonicalize every mtime so the archive is byte-reproducible.
  find "$root" -exec touch --no-dereference -d @0 {} +
  ( cd "$root" && find . -print0 | LC_ALL=C sort -z \
      | cpio --null -o -H newc --reproducible --owner=0:0 2>/dev/null
  ) > "${BUILD}/initramfs.cpio"
  ```
  (Alternatively, if true reproducibility is out of M2 scope, weaken the comment + log message to
  "deterministic entry order and ownership; mtimes not canonicalized" so the code stops over-promising.)

### I2 — docker fallback leaves a fixed-name temp container that wedges the next build

- **File:** `image/build.sh:71-75` (`run_build`, the `detguest-kernel-build:24.04` bootstrap).
- **Severity:** Important (build reliability on the no-native-toolchain path — i.e. THIS host, and
  any fresh clone without flex/bison).
- **Description:** The bootstrap runs `docker run --name detguest-kbuild-tmp ...` then `docker commit`
  then `docker rm` — but the `rm` only happens on the success path. If `apt-get update/install`
  fails (network, mirror hiccup) or the run is interrupted, a stopped container named
  `detguest-kbuild-tmp` lingers. The next invocation hits `docker run --name detguest-kbuild-tmp`,
  which fails with a name conflict, and because of `set -e` the whole build dies — with a docker
  error that doesn't point at the real cause. This exact wedge happened earlier in this session.
  There is no pre-run cleanup. (`docker ps -a` was clean at review time, so it was already manually
  cleared once.)
- **Fix:**
  ```bash
  if ! docker image inspect "$img" >/dev/null 2>&1; then
    log "creating kernel build image ${img}"
    docker rm -f detguest-kbuild-tmp >/dev/null 2>&1 || true   # clear a wedged prior attempt
    docker run --name detguest-kbuild-tmp "$DOCKER_IMAGE" bash -c \
      'apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
         build-essential flex bison bc libelf-dev libssl-dev cpio xz-utils kmod >/dev/null'
    docker commit detguest-kbuild-tmp "$img" >/dev/null
    docker rm -f detguest-kbuild-tmp >/dev/null
  fi
  ```
  Consider also `trap 'docker rm -f detguest-kbuild-tmp 2>/dev/null || true' ERR` around the block,
  or use `docker build -` with a tiny Dockerfile (no named-container lifecycle to leak).
