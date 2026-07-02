# Plan: Ms4 Region Publication Acceptance (Phase 3 ⭐ milestone)

Answers `.agents/requests/phase3-ms4-region-publication-acceptance/` (filed by
rom-operator-bridge, 2026-07-02). Read that directory first; this plan does not
repeat its context.

## Goal (behavioral)

Emulator RAM and framebuffer regions published by a real workload are readable
from the host and stable across 100× snapshot/restore, on the Intel lab
machine, with durable evidence. Concretely, close the three recorded blockers:

1. Region registration goes through the real Ms4 mechanics — mlock + prefault
   in the workload, agent-IPC registration over `/run/detguest/agent.sock`,
   pagemap GVA→GPA translation and manifest write **in the agent** — so the
   manifest advertises what the kernel guarantees.
2. `detguest-agent` `ReverifyRegions` (`crates/detguest-agent/src/commands.rs:77`)
   re-validates published regions instead of no-op-succeeding.
3. The `guest-sdk-m4-platform-readability-vm` acceptance test exists and is
   green: VM workload publishes `wram` + `framebuffer` (**exactly 229,376
   bytes, layout_version 1**) + `meta`; host reads via manifest; contents exact
   and stable across 100 snapshot/restore branches; readability holds after
   restore and after fork.

Plus one contract absorption: bump the staged M9 fixture's framebuffer
(`tests/vm/workloads/src/bin/m9_refwork_contract.rs:20`, currently 4,096 B) to
the D7 length so the hypervisor's new `layout_version 1` check
(determinism-hypervisor `5698d7e`) stops rejecting it.

## The load-bearing architectural decision

**Current code diverges from the spec.** Under the agent, the SDK already does
mlock + self-translation (`/proc/self/pagemap`) and **writes the seqlock
manifest itself** (`crates/detguest-sdk/src/lib.rs` `SdkState::publish_region`,
~lines 374–448). There is no `agent.sock`, no SEQPACKET region path, anywhere
in `crates/`. Yet:

- `detguest-wire/src/manifest.rs:10` states the seqlock discipline as "the
  agent is the only writer ever" — the current SDK-writer model violates it.
- ARCHITECTURE.md §5, API.md §1.5/§6, the request text, and the beads
  (`guest-sdk-m4-agent-ipc-protocol`/`-server`) all spec the agent-IPC model.
- `ReverifyRegions` needs agent-side records ({pid, gva, len} per region) and
  `/proc/<pid>/pagemap` access — which only exist in the agent-writer model.

**Decision: implement the agent-writer model (docs-specced).** SDK does
mlock + prefault, interns the name in its own `InternTable` (the **single**
name_id authority — the host folds `NameIntern` from rings A and W into one
map, so a second allocator would collide; see `drain.rs` + `channel.rs`
`intern_redefinitions`), and sends `RegisterRegion{name, name_id,
layout_version, gva, len, flags}` over `/run/detguest/agent.sock`; the agent
binds the caller pid via `SO_PEERCRED`, walks `/proc/<pid>/pagemap`, coalesces
extents, writes the manifest under the seqlock, replies `{region_id, name_id,
manifest_generation}`, and emits `NameIntern` + `RegionRegister` on ring A
using the caller-supplied id. The SDK's intern path already emitted
`NameIntern` on ring W and, on success, re-emits `RegionRegister` on ring W
(with doorbell, as today) so the host-visible ring-W stream shape is
preserved.

**Handle semantics decision:** `RegionHandle::drop`/`unregister` sends
`UnregisterRegion` (per API.md §1.5). Consequence: workloads must hold their
handles for the process lifetime — the m9 fixture's `let _wram = …` locals in
`publish_regions()` would otherwise DEAD all three regions before Ready. The
fixture change is part of `06-…` §A.

External contracts that must NOT change: manifest byte layout, host
`read_manifest`/`read_region` behavior, `Ready`/pre-Ready evidence emission,
the `[unit.control]` fd-3 handshake, ring event codecs.

## Critical hazard: pre-Ready IPC deadlock

`m9_refwork_contract.rs:58-64` registers regions **between** `GameLoaded` and
its control-`Ready` reply, while the agent is blocked in
`drive_refwork_start`'s `recv(Ready)` (`crates/detguest-agent/src/control.rs:109`).
Under agent-IPC, the workload blocks on the register reply → mutual deadlock.
Likewise `wait_for_expected_regions` (`runtime.rs:289`) polls the manifest
before the supervise epoll loop ever runs. The agent must therefore service
region IPC in three places: the main epoll loop, the expected-regions wait
loop, and the control-`Ready` wait (which becomes a non-blocking poll loop).
See `02-agent-server.md`.

## Work packages (files in this plan)

| File | Package | Depends on |
|---|---|---|
| `01-ipc-protocol.md` | Wire protocol for agent.sock (spec + codec + tests) | — |
| `02-agent-server.md` | Agent listener, pid-bound dispatch, per-pid pagemap, agent-side manifest writer, deadlock-free servicing | 01 |
| `03-sdk-client.md` | SDK `register_region` → mlock + prefault + IPC client; handle unregister | 01, 02 |
| `04-reverify-regions.md` | Real `ReverifyRegions` + corruption-detection tests | 02 |
| `05-harness-snapshot-fork.md` | KVM snapshot/restore/fork in `tests/vm/src/harness/` | — (parallel) |
| `06-acceptance-test-and-fixture.md` | D7 fixture bump, new M4 workload, the 100× acceptance test, evidence artifacts | 02–05 |
| `07-ci-docs-beads-evidence.md` | CI lane check, docs updates, bead reconciliation, session close | all |

Suggested order: 01 → 02 → 03 → 04 (host `cargo test` green after each), with
05 developed in parallel; then 06; then 07. Packages 01–04 land as one logical
transition (the SDK must not lose its manifest-writer before the agent gains
one on the same commit that flips the SDK to IPC).

## Non-goals (from the request)

- The operator-game lab run / first-room padlog (reference-workload
  package-05).
- Ms5 `determinism_replay` CI gate.
- snapshot-store M7 GC.
- Bridge-side verification (items 4–5 of the request's acceptance) — offer the
  handback per `04-verification-offer.md` after refwork M4 regenerates a READY
  snapshot; not gated here.

## Acceptance (verified by us, durable artifacts)

1. Tests that would fail if mlock, translation, or agent registration silently
   regressed to a no-op (unit + VM tiers).
2. `ReverifyRegions` proven non-no-op: a deliberately corrupted/unmapped
   region is detected (unit tier with injected translator; VM tier echo path).
3. The 100× snapshot/restore readability acceptance green on
   `infra-control-kvm-intel`, artifact pointers + hashes recorded under a
   recorded artifact root (same discipline as hypervisor M9:
   `../determinism-hypervisor/target/m9-final-acceptance-20260621T004402Z/`).
4. `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace
   --locked` green; musl agent build still green.

## Top risks and mitigations

1. **KVM restore fidelity** (biggest): restoring a running Linux guest needs
   full vCPU/irqchip/PIT/clock state. Mitigate: incremental validation test
   (snapshot→restore→guest still advances frames) before wiring the 100× loop;
   see `05-…` for the exact state set and known gotchas.
2. **Pre-Ready deadlock** — addressed by design (above + `02-…`).
3. **Host provisioning on this box** (no sudo): verified — the only failing
   `intel-preflight.sh` check on this host is the 2 MiB hugepage pool
   (`HugePages_Total: 0`), and **nothing in tests/vm needs host hugepages**
   (guest RAM is a plain anonymous `GuestMemoryMmap`; the agent's hugetlbfs
   channel page comes from the guest-internal `hugepages=4` cmdline pool).
   The check's comment claiming it is "for in-VM guests' host side" is wrong.
   Resolution (see `07-…`): scope the hugepage check to the
   hypervisor-oriented use with a corrected rationale so the lane can be
   legitimately green. Kernel provenance is current (the bead note is stale).
4. **Golden-hash drift**: resolved by review — `testload.rs` / other workloads
   never register regions and the golden's filter drops RegionRegister, so
   `0x3b0d3ebc93e4ba51` must NOT shift. If it does, that's a bug to fix, not
   a golden to regenerate.
5. **Single-writer transition**: SDK and agent both writing the manifest even
   transiently would corrupt the seqlock. Land 01–03 atomically.
6. **Concurrent-session coordination**: the deployed `dh-workerd` and the
   hypervisor checkout have in-flight state (request `02-…` item 3). This plan
   touches only guest-sdk; do not rebuild/restart anything under
   `~/git/preestablished/.dh-clean-ff1e88c` or the deployed worker.
