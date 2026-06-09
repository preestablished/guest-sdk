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

## Removal

```bash
systemctl --user disable --now actions-runner-guest-sdk.service
cd ~/actions-runner-guest-sdk
./config.sh remove --token "$(gh api -X POST repos/preestablished/guest-sdk/actions/runners/remove-token --jq .token)"
```
