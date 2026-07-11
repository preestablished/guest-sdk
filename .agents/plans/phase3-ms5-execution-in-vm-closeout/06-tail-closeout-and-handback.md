# Package 06 — Tail Disposition, Quality Gates, And Handback

Do not equate the flagship 1000-run with the whole request. Re-enumerate the
live graph from `bd`, not from this plan, and account for every named tail bead.

## A. Remaining implementation/disposition table

For each bead below, `bd show` it, inspect its own acceptance and dependencies,
then implement and close it. The M4 stragglers/docs are live dependencies of
final-quality, so a follow-up note alone cannot produce tail closeout. Only an
explicit acceptance-owner waiver plus dependency amendment/superseding bead
may defer one; then report the phase tail as deferred, not closed.

- `guest-sdk-m5-reference-workload-contract-tests`
- `guest-sdk-m4-capture-contract-tests`
- `guest-sdk-m4-reverify-churn-test`
- `guest-sdk-m4-sdk-stats-region-autoreg`
- `guest-sdk-m4-docs-contracts` if still open/blocked
- `guest-sdk-m3-docs-as-built`
- `guest-sdk-m5-docs-replay`
- `guest-sdk-m3m5-final-quality-gates`
- `guest-sdk-m3m5-handoff-closeout`
- the M3, M4, M5 epics/root if their child closure criteria are met

Contract-test work must validate the current refwork/capture artifacts without
taking ownership of those repos. The churn and stats beads require their
specified behavioral tests; do not close them as administrative cleanup.

Before claiming final-quality, require `bd blocked` to show zero relevant live
blockers across the enumerated tail.

## B. Final quality gates

Run the exact live final-quality gates: fmt check; clippy with workflow flags;
workspace tests; `detguest-wire --no-default-features`; named Miri ring tests;
Loom ring tests; musl release build; docs/build checks; the focused KVM
acceptances from packages 02–04, default Intel preflight, and the final named
CI workflow proof. Preserve command, exit status, duration, and output/artifact
pointer. Re-run the real-path deliberate mismatch once from a clean checkout.

Mechanically query every bead named by the request and this package. The final
table includes status, closure/disposition reason, implementing SHA, and
evidence path. No blanket `bd close` operation is acceptable.

Also run `bd lint`, `bd preflight`, and `bd doctor --check=conventions`. If an
exact toolchain is unavailable, record/file the blocker rather than silently
substituting a weaker gate.

## C. Resolution and exit-gate citation

Add the next numbered resolution markdown file under
`.agents/requests/phase3-ms5-execution-in-vm-closeout/` containing:

- package SHAs and cross-repo revision/image matrix;
- prep A/B/C results and the 10-run timing measurement;
- Ms3 boot/event/PAD_SET evidence;
- Ms5 inject/reattach and negative evidence;
- 1000-run manifest, ordered summary digest, chunk/resume history, four-surface
  result, fault/input censuses, and VerifyReplay references;
- linked recurring CI run and its stated iteration budget;
- complete bead disposition table and quality-gate outputs; and
- the exact Phase-3 exit-gate-2 citation path.

Update the `m5-reference-workload-*` beads themselves as well as the resolution
file. Post the exit-gate citation in the phase handoff location named by the
current phase documents; verify every link/path and revision resolves.

## D. Repository close protocol

Commit intentionally scoped changes, update/close beads with their actual
implementing SHAs, run `bd dolt push`, `git pull --rebase`, and `git push`.
Finish only after `git status` reports a clean branch up to date with origin
and the linked CI run is green. If an authorized waiver leaves a tail item
deferred, file the follow-up bead before handback, amend the graph explicitly,
and label the handback deferred rather than claiming Phase-3 closeout.
