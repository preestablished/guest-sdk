# Package 05 — Recurring Intel Lane And As-Built Documentation

Covers `guest-sdk-m3m5-ci-intel-vm-lanes`,
`guest-sdk-m3-docs-as-built`, and `guest-sdk-m5-docs-replay` after their
acceptance dependencies close.

## Workflow shape

Keep the security boundary already present in `.github/workflows/ci.yaml`:
the KVM job remains push-only and runs only on `[self-hosted, intel, kvm]`;
fork PR code never reaches the shared runner.

Replace the opaque whole-ignored-tier-only signal with named steps (or named
test invocations) for:

- preflight;
- Ms3 real-workload acceptance;
- Ms3 PAD_SET acceptance;
- existing Ms4 gates;
- Ms5 inject/reattach acceptance; and
- Ms5 recurring replay sample.

Give each a useful command timeout (for example GNU `timeout`, since Actions
has no step-level timeout) and artifact upload on success or failure. Preserve
the broad sweep only if it adds coverage without running the same expensive
test twice.

## Choose and state the recurring replay budget

Use the package-04 10-run measurement and the runner's practical timeout to
select a small exact recurring iteration count. The value must fit with margin
alongside Ms3/Ms4 and still cover multiple seeds, all fault-decision classes,
and input bursts. Encode it explicitly in CI, document why it was selected,
and distinguish it from the one-time 1000-run lab acceptance. Do not increase
the job timeout to many hours merely to rerun 1000 on every push.

If a long manual workflow is added, it must remain trusted-operator-only,
accept a manifest/run ID and explicit range, use the same pinned environment,
and upload durable evidence. It complements rather than replaces the recurring
push signal.

Require real-image environment variables and files before cargo so a green
skip cannot count. Upload with `actions/upload-artifact` under `if: always()`
from the configured evidence directory, with unique run/attempt name and
explicit retention; pin new third-party actions according to repository policy.

Security acceptance: restrict self-hosted execution to pushes on `main`, use
read-only permissions, expose no PR/`workflow_run` route or untrusted checkout
ref, and never interpolate manual inputs into shell. A manual long lane needs
an authorized environment, validated numeric ranges/run ID, and concurrency
that does not cancel an active acceptance.

## Documentation

Update `docs/ci/intel-runner.md` with exact local/CI commands, double gating,
budgets, timeouts, evidence locations, resume semantics, image/revision pins,
and the final hugepage conclusion. Update the relevant SDK/architecture docs
with Ms3 input ordering and Ms5 record/replay ownership: guest-sdk consumes
decoded decisions; hypervisor owns DHILOG serialization and VerifyReplay.

## Proof

Push the wiring and obtain one linked green `in_vm` run on the intended runner.
Verify from logs that each named gate body executed and the configured replay
iteration count completed; a skipped ignored test is not green evidence.
Confirm artifacts are retained and downloadable. Then close the CI-lane and
docs beads with the workflow link, commit SHA, and recurring budget.
