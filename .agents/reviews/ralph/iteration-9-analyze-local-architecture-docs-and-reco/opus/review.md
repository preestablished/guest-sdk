# Review: opus

## Findings

- High: `crates/detguest-sdk/src/inject.rs` still returns `FaultDecision::Proceed` without emitting `InjectQuery` or performing the detcall round trip.
- Medium: `crates/detguest-sdk/src/regions.rs` still accepts `register_region()` without publishing a region to the agent or manifest.
- Low/test gap: `tests/vm/workloads/src/bin/testload.rs` does not exercise the deferred M4/M5 `register_region()` and `inject_point()` paths.

## Resolution

- Deferred to existing beads: `guest-sdk-m5-sdk-inject-point` and `guest-sdk-m4-sdk-register-region`.
- Added bead notes so these review observations stay attached to the owning future work.

## Verification

- Reviewer ran per-crate tests for SDK, host, agent, workloads, VM harness, and wire. All passed.
- KVM ignored tests were not run.
