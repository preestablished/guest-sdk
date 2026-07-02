# 07 — CI, docs, bead reconciliation, handback, session close

## CI

`.github/workflows/ci.yaml` `in_vm` lane (push-only, `[self-hosted, intel,
kvm]`) already runs `DETGUEST_VM_TESTS=1 cargo test -p detguest-vmtest --
--ignored --test-threads=1` — the new `m4_acceptance` and snapshot validation
tests are swept automatically. Verify only:

- Lane job timeout accommodates the added runtime (raise `timeout-minutes` if
  present and tight).
- `scripts/intel-preflight.sh`: review verified the host state — the ONLY
  failing check today is the 2 MiB hugepage pool (`HugePages_Total: 0`, needs
  root to change), and nothing in tests/vm needs host hugepages (guest RAM is
  a plain anonymous mmap; the agent's hugetlbfs channel page comes from the
  guest-internal `hugepages=4` cmdline pool). Kernel provenance is current;
  `/dev/kvm`, perf paranoid level, vmx, musl target all pass. Fix: move the
  host-hugepage check behind an opt-in flag (e.g. `--require-host-hugepages`)
  with a corrected comment (the current "for in-VM guests' host side"
  rationale is wrong — it exists for the hypervisor repo's harness), so the
  guest-sdk lane is legitimately green. Do not touch the checks that guard
  the pinning assumptions (COMPACTION/MIGRATION/KSM/THP/SWAP off,
  `CONFIG_PROC_PAGE_MONITOR=y`, `CONFIG_UNIX=y`) — those are exactly what
  Ms4 leans on.
- Fork-PR safety unchanged (`if: github.event_name == 'push'`).

## Docs (bead `guest-sdk-m4-docs-contracts` partially; API is in-repo prompts/docs)

- `prompts/docs/guest-sdk/API.md` §1.5: replace the informal agent.sock prose
  with the normative protocol from `01-…` (message tables, status codes,
  session model, `SO_PEERCRED` pid binding). Note ring-A registration-time
  `NameIntern`+`RegionRegister` and the retained pre-Ready evidence
  duplication.
- API.md §6 `ReverifyRegions`: confirm the implemented semantics match the
  spec line (echo / P0 alarm / DEAD on unmappable — the DEAD-on-unmappable
  behavior is new; spec it).
- `prompts/docs/guest-sdk/ARCHITECTURE.md` §5: reality now matches the spec;
  fix any residual drift (mlock not mlock2; prefault = volatile touch; agent
  ledger survives snapshot/restore — add the note from `04-…`).
- `docs/ci/intel-runner.md`: add the M4 gate to the lane description, evidence
  artifact convention, `DETGUEST_M4_CHILDREN` override.
- Cross-repo handback: write
  `.agents/requests/phase3-ms4-region-publication-acceptance/05-resolution.md`
  summarizing what landed, the evidence artifact root + hashes, the fixture
  bump, and what remains for the joint refwork M4 bring-up (mirror the
  precedent format in `../determinism-hypervisor/.agents/requests/rom-bridge-getframebuffer-region-contract/`).

## Beads

Claim at start of implementation (`bd update <id> --claim`); close only what
is genuinely done, with reasons pointing at commits/artifacts:

| Bead | Action |
|---|---|
| `guest-sdk-m4-agent-ipc-protocol` | close after 01 (reason: protocol module + tests, commit ref) |
| `guest-sdk-m4-agent-ipc-server` | close after 02 |
| `guest-sdk-m4-agent-pagemap-pid-extents` | close after 02 |
| `guest-sdk-m4-sdk-register-region` | close after 03 |
| `guest-sdk-m4-platform-readability-vm` | close after 06 green with artifact root in reason |
| `guest-sdk-m4-docs-contracts` | close after docs above if its scope is covered; else update notes |
| `guest-sdk-m4-image-boot-fixtures` | audit: boot.toml expected_region + unit.control fixtures largely exist (runtime/control tests + m4 fixture add more); close if satisfied, else note gap |
| `guest-sdk-m4-unit-control-reference-handoff` | audit: `control.rs` implements the handoff; likely closable — verify acceptance text first |
| `guest-sdk-m4-sdk-stats-region-autoreg` | NOT in this request's scope; leave open. It blocks `platform-readability-vm` in the dep graph — if the acceptance passes without it, `bd dep remove guest-sdk-m4-platform-readability-vm guest-sdk-m4-sdk-stats-region-autoreg` with a note (stats autoreg is additive, not a readability precondition) |
| `guest-sdk-m4-capture-contract-tests`, `guest-sdk-m4-host-read-region-restore-tests` | audit against what 06 covers; close or trim scope with notes |
| `guest-sdk-ext-hyp-capture-region-read` | external blocker now DONE upstream (DH-5 implemented; contract `5698d7e`) — close with pointer to the request's `01-current-state.md` |
| `guest-sdk-m3-*` deps blocking the protocol bead | do NOT silently close; if they block closing done work, `bd dep remove` with a recorded reason (M3 acceptance is separate work; flag in handoff for Matt) |
| `guest-sdk-m3m5-ci-intel-vm-lanes` | progresses but stays open (M3/M5 gates remain) — update notes |

Also `bd remember` durable gotchas discovered during implementation (KVM
restore skip-list MSRs, preflight findings, golden regeneration) —
per-project convention is bd memories, not MEMORY.md.

## Quality gates (before any close/push)

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
# musl agent (mirror ci.yaml musl lane invocation exactly)
# VM tier: DETGUEST_VM_TESTS=1 cargo test -p detguest-vmtest -- --ignored --test-threads=1
```

## Session close (CLAUDE.md mandatory workflow)

1. File beads for any follow-up work discovered.
2. Quality gates above.
3. Close/update beads per the table.
4. `git pull --rebase` → `bd dolt push` → `git push` → `git status` shows
   up-to-date. Commit granularity: 01+02+03 may need to be one commit (see 00
   risk #5 single-writer transition); 04, 05, 06, docs/CI can be separate
   commits with why-bodies.
5. Handoff note: what landed, artifact root, what the bridge team should do
   next (their `04-verification-offer.md` standing offer), remaining beads.
