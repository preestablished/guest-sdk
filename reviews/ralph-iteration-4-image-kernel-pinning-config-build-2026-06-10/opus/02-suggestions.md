# Suggestions (non-blocking)

### S-1 — Dead fragment line: `CONFIG_PIPEFS=y` is silently dropped (no Kconfig symbol)

- **File:** `image/kernel.config:54`

`PIPEFS` is an internal always-on filesystem with no user-selectable Kconfig entry. I
verified it is absent from the final `.config` even though the fragment requests it —
`merge_config.sh -m` accepted the line and `olddefconfig` dropped it with no effect. It's
harmless (pipes work regardless), but a dead line that *looks* load-bearing can mask a typo
in a neighbouring symbol and isn't covered by `assert_required_set`. Either drop it or, better,
have `merge_config.sh` run *without* `-m` for a one-off audit and grep its "Value requested
for X not in final" warnings (see S-2). Recommend simply deleting the line.

### S-2 — `merge_config.sh` "value requested … not in final .config" warnings are discarded

- **File:** `image/build.sh:133`

`merge_config.sh -m` prints a warning for every fragment symbol that doesn't survive into the
merged config (the PIPEFS case in S-1, and it would catch a misspelled symbol or one with
unmet deps). The script swallows that signal. Consider capturing stderr from line 133 and
`die`-ing (or at least `log`-ing loudly) on any `not in final .config` line for symbols you
*expect* to take. This is a cheaper, broader safety net than enumerating `REQUIRED_SET` by
hand — it catches the "I added a knob and it silently did nothing" class generically.

### S-3 — Stale/partial extracted tree is trusted across same-version re-runs

- **File:** `image/build.sh:91-94` (`fetch_kernel`)

The digest is verified before *first* extraction (correct ordering). But the re-extract guard
is `[[ ! -d "$SRC" ]]`: if a previous `tar` was interrupted (Ctrl-C / OOM) it leaves a
partial `$SRC` that is then trusted forever, and a same-version tarball swap (matching
filename) is never re-extracted. Low likelihood given the SHA pin, but cheap to harden:
extract to a temp dir and `mv` into place atomically, or drop a `.extracted-ok` stamp keyed on
the digest and re-extract when it's missing:
```bash
if [[ ! -f "${SRC}/.extracted-${KERNEL_SHA256:0:12}" ]]; then
  rm -rf "$SRC"; tar -C "$BUILD" -xf "$tarball"; touch "${SRC}/.extracted-${KERNEL_SHA256:0:12}"
fi
```

### S-4 — `HZ_PERIODIC` + `HZ=250` is implicit (defaulted by olddefconfig), not pinned

- **File:** `image/kernel.config` (timer section), surfaced in final config as `CONFIG_HZ_250=y`, `CONFIG_HZ_PERIODIC=y`, `# CONFIG_HIGH_RES_TIMERS is not set`

The determinism story leans on the timer regime, but the fragment is silent on it, so the
values come from tinyconfig + olddefconfig defaults (I observed `HZ_PERIODIC`, `HZ=250`, no
hrtimers, `NO_HZ` off). These are *reasonable* deterministic choices (a fixed periodic tick
under virtualized time), but leaving them implicit means a future tinyconfig/version bump
could shift HZ or flip `NO_HZ_IDLE` without tripping any assertion. Recommend pinning
`CONFIG_HZ_PERIODIC=y` / `CONFIG_HZ_250=y` (or whichever is canonical) explicitly with a
one-line rationale, and add them to `REQUIRED_SET`. The READY-point icount depends on the
tick model; making it explicit is cheap insurance. (Not blocking — the §4.1 contract is
"same image, N boots", and the image is self-consistent today.)

### S-5 — `HYPERVISOR_GUEST`/kvmclock is off; confirm that's intended for the determinism model

- **File:** `image/kernel.config` (no paravirt section), final config `# CONFIG_HYPERVISOR_GUEST is not set`

With `HYPERVISOR_GUEST` off there is no kvmclock/paravirt-clock; the guest uses bare TSC
(`CONFIG_X86_TSC=y`). The KERNEL.md determinism rationale doesn't mention clock source at all.
Given §7 says time is hypervisor-virtualized, a bare-TSC-only guest is plausibly *more*
controllable than a paravirt clock — but this is a load-bearing determinism decision that's
currently made by omission. A one-line note in KERNEL.md ("no paravirt clock; the hypervisor
controls TSC/timer exits — see issue #1 / §7") would make the intent reviewable rather than
accidental.

### S-6 — KERNEL.md cites a sha256sums URL that isn't the conventional kernel.org filename

- **File:** `image/KERNEL.md:32-33`

> The digest above is from `cdn.kernel.org/pub/linux/kernel/v6.x/sha256sums.asc` (2026-06-10).

kernel.org publishes per-release detached signatures (`linux-6.12.93.tar.sign`, over the
*uncompressed* tar) and a `sha256sums.asc` aggregate; the latter does exist but the digest
that matters for `build.sh` is over the `.tar.xz`, which the aggregate file covers only if it
lists the compressed artifact. Worth a one-line note clarifying that the pinned SHA256 is over
`linux-6.12.93.tar.xz` specifically (which is what `build.sh` checks), so a future bumper
knows exactly which digest to copy. Provenance claim is otherwise good and dated.
