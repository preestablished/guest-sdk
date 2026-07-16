# Action Items

### Critical
- [ ] None.

### Important
- [ ] [image/kernel.config:46 + image/build.sh:44] Add `CONFIG_NET=y` and `CONFIG_UNIX=y` to the fragment (and to `REQUIRED_SET`) — ARCHITECTURE.md §4.2 specifies the agent↔SDK control plane is `socketpair(AF_UNIX, SOCK_SEQPACKET)`; the current image has `# CONFIG_NET is not set` and cannot create it (M3 will fail at runtime). Loopback-only, stays in the §7 determinism envelope. Forces one rebuild (config is in the build key). (I-1)
- [ ] [image/build.sh:157] Pin `umask 022` at the top of `cmd_initramfs` (or `chmod` the staged dirs/files to canonical modes) — initramfs is NOT byte-reproducible across umask today (verified: 022 vs 077 → different cpio sha256), because newc records the mode bits of the `mkdir`-created mountpoint dirs. (I-2)
- [ ] [image/build.sh:71-75] Make the docker temp container unique-named and clean it up unconditionally (or `trap … RETURN`) — a failed `apt-get` aborts under `set -e` before `docker rm`, wedging every later run on "name already in use" on the box's primary no-sudo path. (I-3)

### Suggestions
- [ ] [image/kernel.config:54] Remove dead `CONFIG_PIPEFS=y` line (no Kconfig symbol; silently dropped, unchecked by the assert). (S-1)
- [ ] [image/build.sh:133] Capture `merge_config.sh` stderr and fail/warn on "value requested … not in final .config" — generic guard against silently-ineffective fragment lines. (S-2)
- [ ] [image/build.sh:91-94] Harden re-extract against partial/swapped same-version tarballs (extract-to-temp + atomic mv, or a digest-keyed `.extracted-ok` stamp). (S-3)
- [ ] [image/kernel.config timer section] Pin `CONFIG_HZ_PERIODIC=y` / `CONFIG_HZ_250=y` explicitly + add to `REQUIRED_SET` rather than inheriting them from olddefconfig defaults (READY-point icount depends on the tick model). (S-4)
- [ ] [image/KERNEL.md] Add a one-line note that the kernel runs with no paravirt clock (`HYPERVISOR_GUEST` off, bare TSC) and that this is the intended determinism choice (hypervisor controls TSC/timer exits). (S-5)
- [ ] [image/KERNEL.md:32-33] Clarify the pinned SHA256 is over `linux-6.12.93.tar.xz` specifically (what build.sh checks), so future bumpers copy the right digest. (S-6)
