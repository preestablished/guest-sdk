# Intel-box self-hosted GitHub Actions runner

The in-VM test tier (KVM harness, retired-instruction icount gates, kernel boot
tests) runs only on a self-hosted runner on the VT-x Intel box. This documents
how the `intel-box` runner for `preestablished/guest-sdk` was provisioned and
how to re-provision it.

## Runner identity

- Name: `intel-box`, labels: `self-hosted`, `intel`, `kvm` (plus implicit
  `Linux`, `X64`). Workflows gate the in-VM tier with
  `runs-on: [self-hosted, intel, kvm]`.
- Host: `infra-control`, user `infra-admin`, install dir
  `~/actions-runner-guest-sdk/`.
- Note: this box hosts runner instances for other repos in sibling directories
  (`~/actions-runner`, `~/actions-runner-verin`, `/home/github-runner/…`).
  Each instance gets its own directory — never reconfigure an existing one.

## Host requirements (verified at provisioning)

| Requirement | Why | Check |
|---|---|---|
| VT-x CPU | KVM guests | `grep -m1 vmx /proc/cpuinfo` |
| `/dev/kvm` access | harness opens KVM | user in `kvm` group (`id`) |
| `perf_event_paranoid` ≤ 1 | retired-instruction counter | `cat /proc/sys/kernel/perf_event_paranoid` |
| Rust stable + musl target | agent cross-build | `rustup target list --installed \| grep x86_64-unknown-linux-musl` |

## Provisioning steps

```bash
mkdir -p ~/actions-runner-guest-sdk && cd ~/actions-runner-guest-sdk
curl -sL https://github.com/actions/runner/releases/download/v2.335.1/actions-runner-linux-x64-2.335.1.tar.gz | tar xz

# Registration token requires repo admin; expires after ~1 h.
TOKEN=$(gh api -X POST repos/preestablished/guest-sdk/actions/runners/registration-token --jq .token)
./config.sh --url https://github.com/preestablished/guest-sdk --token "$TOKEN" \
  --name intel-box --labels self-hosted,intel,kvm --work _work --unattended
```

The runner runs as a **user-level systemd service** (root was not available for
`svc.sh install`; user services need none and survive reboots via lingering):

```bash
# unit: ~/.config/systemd/user/actions-runner-guest-sdk.service
#   ExecStart=/home/infra-admin/actions-runner-guest-sdk/run.sh
#   Restart=always, WantedBy=default.target
systemctl --user daemon-reload
systemctl --user enable --now actions-runner-guest-sdk.service
loginctl enable-linger infra-admin     # start at boot without a login session
```

## Verification

```bash
systemctl --user status actions-runner-guest-sdk.service
gh api repos/preestablished/guest-sdk/actions/runners \
  --jq '.runners[] | {name, status, labels: [.labels[].name]}'
# expect: {"name":"intel-box","status":"online","labels":[...,"intel","kvm"]}
```

## What the in-VM lane runs

The `in_vm` job (push-only, `runs-on: [self-hosted, intel, kvm]`) runs
`./scripts/intel-preflight.sh` and then the whole ignored VM tier in one sweep:

```bash
DETGUEST_VM_TESTS=1 cargo test -p detguest-vmtest -- --ignored --test-threads=1
```

That sweep includes, alongside the M2 acceptance:

- **M4 snapshot/restore validation** (`tests/vm/tests/m4_snapshot.rs`): the
  KVM snapshot/restore/fork harness fidelity tests that de-risk the big loop.
- **M4 acceptance gate** (`tests/vm/tests/m4_acceptance.rs`): the Ms4
  platform-readability acceptance — one root VM boots the m4-regions fixture,
  is snapshotted after warm-up, and N children (default **100**;
  `DETGUEST_M4_CHILDREN=<n>` overrides for local iteration — the evidence file
  records the actual count) each prove restore fidelity, run 60 frames under a
  child-specific pad schedule, verify host-side reads against recomputation,
  and exercise `ReverifyRegions` (echoes only, zero P0 alarms), plus a
  fork-of-fork leg.

Evidence artifact convention: each M4 acceptance run writes a durable evidence
root under `target/m4-acceptance-<UTC>Z/` containing `evidence.json`
(environment, child count, SHA-256 table) and `root-regions/` (the root
snapshot's region dumps) — same discipline as the hypervisor's M9 acceptance
artifacts.

Preflight note: the host 2 MiB hugepage-pool check is **opt-in** via
`./scripts/intel-preflight.sh --require-host-hugepages`. Nothing in `tests/vm`
needs host hugepages (guest RAM is a plain anonymous mmap; the agent's
hugetlbfs channel page comes from the guest-internal `hugepages=4` cmdline
pool) — the flag exists for the determinism-hypervisor repo's harness, which
does map host hugepages. The default lane invocation passes no flag.

## Removal

```bash
systemctl --user disable --now actions-runner-guest-sdk.service
cd ~/actions-runner-guest-sdk
./config.sh remove --token "$(gh api -X POST repos/preestablished/guest-sdk/actions/runners/remove-token --jq .token)"
```

## Security posture (review findings, 2026-06-10)

The repo is **public**, so the in-VM job is gated to `push` events only
(`if: github.event_name == 'push'` in ci.yaml) — fork-PR code never runs on
this box; hosted lanes carry the full pre-merge signal. GitHub's
first-time-contributor approval is NOT sufficient (returning contributors
bypass it).

Residual risk and recommended least-privilege follow-ups (this runner user
currently has `docker` — root-equivalent — and `sudo` membership, and the box
hosts other personal services):

- run the runner as a dedicated user with no `docker`/`sudo` membership
  (kvm group only);
- consider ephemeral runners (`--ephemeral`) so each job gets a fresh
  registration;
- SHA-pin third-party actions if the workflow surface grows.
