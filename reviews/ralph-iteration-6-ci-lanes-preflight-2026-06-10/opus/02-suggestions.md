# Suggestions (non-blocking)

### S1 — Add explicit `permissions:` blocks (least privilege)

**File:** `.github/workflows/ci.yaml` (top-level), `.github/workflows/fuzz.yaml` (top-level)

Both checkouts are of **public** repos, so the default `GITHUB_TOKEN` perms are
sufficient and nothing is broken today. But neither workflow declares `permissions:`,
so jobs inherit whatever the repo/org default is (often read/write). Pin least
privilege explicitly:

```yaml
permissions:
  contents: read
```

`fuzz.yaml` uploads artifacts via `actions/upload-artifact@v4`, which does **not**
require any extra token scope (it uses the runtime, not the API), so `contents: read`
suffices there too. This is defense-in-depth and pairs well with the C1 hardening.

### S2 — Concurrency control to cancel superseded PR runs

**File:** `.github/workflows/ci.yaml:8`

Long lanes (miri, loom, musl, in_vm) will pile up when a PR is pushed repeatedly. Add:

```yaml
concurrency:
  group: ci-${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: true
```

This also reduces the window in which a malicious PR can keep the self-hosted runner
busy (complements C1).

### S3 — Pin `Swatinem/rust-cache` shared-key per job to avoid cross-lane cache thrash

**File:** `.github/workflows/ci.yaml` (every rust-cache step)

Several jobs share `workspaces: guest-sdk` but build different target sets (musl,
miri nightly, loom `--cfg loom`, aarch64). Without a distinct `shared-key`/`key` they
can contend on the same cache entry and evict each other. Add `with: { shared-key: <job> }`
per job (e.g. `shared-key: miri`, `shared-key: loom`) so each lane keeps a stable
cache. The loom lane especially wants isolation since `--cfg loom` produces a wholly
different `target/`.

### S4 — `in_vm` and `fuzz` should add a job-level `timeout-minutes`

**Files:** `.github/workflows/ci.yaml:119`, `.github/workflows/fuzz.yaml:16`

The default 360-minute job timeout is far too generous for a self-hosted box and for
a fuzz job whose intended budget is ~30 min + overhead. Cap them, e.g.
`timeout-minutes: 45` on both, so a wedged in-VM test or a runaway fuzz binary can't
hold the runner for 6 hours.

### S5 — Preflight: prefer `pidof`-free, explicit numeric guard on `para`

**File:** `scripts/intel-preflight.sh:44-49`

The guard `[[ -n "$para" && "$para" -le 1 ]]` is correct and short-circuits on empty
(good — avoids the `[[: -le: ...` arithmetic error on an unreadable file). One edge:
if `/proc/sys/kernel/perf_event_paranoid` ever contains `-1` (which is *more*
permissive and valid), `-le 1` correctly passes. No change strictly required; consider
a comment noting `-1` is the most-permissive accepted value so a future reader doesn't
"fix" it to `[[ "$para" -ge 0 ]]`.

### S6 — Preflight: KVM ioctl probe could leak the fd / be slightly more robust

**File:** `scripts/intel-preflight.sh:28-33`

The Python probe opens `/dev/kvm` and never closes the fd before the interpreter
exits — harmless for a one-shot, but `os.close(fd)` after the `print` is tidy. More
importantly, the probe prints the raw int and string-compares to `"12"`; if a future
kernel returns a different (still valid) API version the gate hard-fails. That's
arguably the desired strictness for a *pinned* environment, so this is informational
only. Optionally accept `>= 12`:

```python
v = fcntl.ioctl(fd, 0xAE00)
os.close(fd)
print(v)
```
with `[[ "${api:-0}" -ge 12 ]]` on the bash side, if forward-compat is wanted.

### S7 — `fuzz.yaml`: drop the redundant `+nightly` or document why it's belt-and-suspenders

**File:** `.github/workflows/fuzz.yaml:30`

`dtolnay/rust-toolchain@nightly` already sets nightly as the job default, so
`cargo +nightly fuzz run` is redundant (`cargo fuzz run` would resolve nightly). It's
harmless and arguably clearer-as-intent, but note that `cargo +nightly` requires the
nightly toolchain to be *installed under that name* — which the action does — so it
works either way. No action needed; flagging only so it isn't mistaken for a bug.
