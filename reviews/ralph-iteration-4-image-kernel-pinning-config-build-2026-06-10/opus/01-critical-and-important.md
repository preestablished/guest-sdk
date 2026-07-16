# Critical & Important findings

No **Critical** findings.

---

## Important

### I-1 — Config omits `CONFIG_NET=y`/`CONFIG_UNIX=y`; agent IPC socket (`socketpair(AF_UNIX, SOCK_SEQPACKET)`) cannot be created

- **File:** `image/kernel.config` (the fragment as a whole; nearest line `image/kernel.config:46` after `CONFIG_MULTIUSER=y`)
- **Severity:** Important

ARCHITECTURE.md §4.2 (line 73) pins the agent↔SDK control-plane transport as:

> framing, transport (`socketpair(AF_UNIX, SOCK_SEQPACKET)`, child end inherited as fd 3) …

and IMPLEMENTATION-PLAN.md M3 line 95 restates it (`Agent IPC socket (SOCK_SEQPACKET)`).
I confirmed against the built tree that the final `.config` has `# CONFIG_NET is not set`
and no `CONFIG_UNIX` line at all, and that `config UNIX` lives inside `if NET … endif` in
`net/Kconfig` (so `CONFIG_UNIX=y` *requires* `CONFIG_NET=y`). On this image,
`socketpair(AF_UNIX, SOCK_SEQPACKET, …)` returns `EAFNOSUPPORT`.

The prompt correctly notes this is M3 functionality. But the image is built **now** and is
meant to be the one canonical pin; baking a kernel that structurally cannot host the agent's
control plane will surface as a confusing runtime failure two iterations later. AF_UNIX
socketpair is loopback-only (no NIC, no routing), so enabling it does not introduce
nondeterminism — it stays inside the §7 determinism envelope. Fix it in the fragment now and
add both symbols to `REQUIRED_SET` so a future `olddefconfig` can't silently drop them.

**Fix (`image/kernel.config`):**
```diff
 # ---- agent runtime needs (single-threaded epoll loop, Rust std) ----
+# Agent<->SDK control plane is socketpair(AF_UNIX, SOCK_SEQPACKET) (ARCHITECTURE §4.2).
+# Loopback-only: no NIC/routing, stays inside the §7 determinism envelope.
+CONFIG_NET=y
+CONFIG_UNIX=y
 CONFIG_EPOLL=y
```
and in `image/build.sh` `REQUIRED_SET`:
```diff
   "CONFIG_BLK_DEV_INITRD=y"
+  "CONFIG_NET=y"
+  "CONFIG_UNIX=y"
 )
```
This invalidates `.kernel-build-key` (config fragment is in the key — verified), forcing one
rebuild, which is correct.

---

### I-2 — "byte-reproducible initramfs" claim is umask-dependent (mountpoint dirs created by `mkdir` carry process umask into newc mode bits)

- **File:** `image/build.sh:157` (the `mkdir -p "$root"/{…}`), claim made at `image/KERNEL.md` reproducibility section and `image/build.sh:163`
- **Severity:** Important (correctness of a stated guarantee, not a build break)

`cpio -H newc --reproducible --owner=0:0` normalizes timestamps, inode numbers, and
ownership — but **not** file *mode* bits, which come from the staged tree. The empty
mountpoint dirs (`/proc /sys /dev /dev/hugepages /run /etc/detguest /sbin`) are created by
`mkdir -p` (line 157) and therefore take their permission bits from the builder's umask. I
verified empirically:

```
umask 022 -> initramfs.cpio sha256 = ae79eead…  (dir mode 0755)
umask 077 -> initramfs.cpio sha256 = d5aa31a2…  (dir mode 0700)
```

Same inputs, same size (786944 B), **different bytes**. So the reproducibility contract holds
only *given a fixed umask*, which the script does not pin. Any CI runner or developer with a
non-022 umask produces a different image and a different READY-point provenance hash.

**Fix:** pin the umask at the top of `cmd_initramfs` (or normalize modes explicitly):
```diff
 cmd_initramfs() {
   local stage="${1:?usage: build.sh initramfs <stage-dir>}"
+  umask 022   # newc records mode bits; pin them so the cpio is umask-independent
```
Alternatively `find "$root" -type d -exec chmod 0755 {} +` before the cpio, and `chmod`
the copied files to canonical modes. Then either soften the KERNEL.md/comment wording or keep
the strong claim now that it's true. (`cp -a` already preserves the *staged* files' modes, so
those are reproducible as long as the stage dir is; only the `mkdir` dirs were the leak.)

---

### I-3 — docker toolchain fallback: fixed-name temp container + no failure cleanup ⇒ second run wedges on "name already in use"

- **File:** `image/build.sh:71-75`
- **Severity:** Important (robustness on the *intended* no-sudo path)

```bash
docker run --name detguest-kbuild-tmp "$DOCKER_IMAGE" bash -c '… apt-get install …'
docker commit detguest-kbuild-tmp "$img" >/dev/null
docker rm detguest-kbuild-tmp >/dev/null
```

Under `set -e`, if the `apt-get` step fails (network blip, mirror hiccup — exactly the
flaky path), the script aborts **before** `docker rm`, leaving a stopped container named
`detguest-kbuild-tmp`. Every subsequent invocation then dies at `docker run --name …` with
`Conflict. The container name "/detguest-kbuild-tmp" is already in use`, and the image is
never created — a sticky failure that looks unrelated to the original apt error. Two
concurrent builds collide the same way. This is the box's *primary* path (no native
toolchain), so it matters.

**Fix:** clean up the temp container unconditionally and use a unique name:
```bash
local tmp="detguest-kbuild-tmp-$$"
docker rm -f "$tmp" >/dev/null 2>&1 || true
if docker run --name "$tmp" "$DOCKER_IMAGE" bash -c '…install…'; then
  docker commit "$tmp" "$img" >/dev/null
fi
docker rm -f "$tmp" >/dev/null 2>&1 || true
```
(or `trap 'docker rm -f "$tmp" >/dev/null 2>&1 || true' RETURN` inside the branch). A
multi-stage `Dockerfile` / `docker build` would also sidestep the named-container lifecycle
entirely.
