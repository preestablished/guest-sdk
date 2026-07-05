# 04 - Verification and Handoff

Verification depends on which package 03 path was taken. Keep the evidence
small but exact: command, environment, commit, and pass/fail signal.

## Always Run

Host/unit checks:

```bash
cargo test -p detguest-wire
cargo test -p detguest-host
cargo test -p detguest-sdk
cargo test -p detguest-agent
```

VM checks on an Intel/KVM runner:

```bash
DETGUEST_VM_TESTS=1 cargo test -p detguest-vmtest --test no_timer_boot -- --nocapture
DETGUEST_VM_TESTS=1 cargo test -p detguest-vmtest --test <new-no-timer-post-ready-test> -- --nocapture
```

If the real reference-workload artifact is available:

```bash
REFWORK_READY_INITRAMFS=/path/to/initramfs.cpio \
  cargo test -p detguest-vmtest --test refwork_ready_hold \
  no_timer_real_harness_reaches_and_holds_ready -- --nocapture
```

If the failure involved READY-snapshot restore, include the new snapshot child
test in both synthetic and real-artifact forms where possible.

## If Guest-SDK Code Changed

Add red-before/green-after evidence:

- the new reproducer times out or fails on the pre-fix tree;
- the same reproducer passes after the fix;
- reverting only the fix makes the reproducer fail again, if the revert is
  cheap and isolated.

Also run the relevant existing VM suites because frame-boundary changes touch
shared behavior:

```bash
DETGUEST_VM_TESTS=1 cargo test -p detguest-vmtest --test m4_snapshot -- --nocapture
DETGUEST_VM_TESTS=1 DETGUEST_M4_CHILDREN=4 \
  cargo test -p detguest-vmtest --test m4_acceptance -- --ignored --nocapture
```

If an SDK post-frame doorbell was added, explicitly record the expected READY
and frame icount shift. Do not compare against old deployed snapshot hashes.

## If No Guest-SDK Fix Was Needed

Do not force a code change. Land only the reproducer/docs if they are useful,
and file the downstream handoff with:

- proof that no-timer live boot produces `FrameMark` and `FRAME_COUNTER`;
- proof that no-timer READY-snapshot restore produces the first next frame;
- ring W drain expectation at pv-pad `FRAME_COUNTER`;
- exact bridge/worker evidence showing where downstream behavior diverges.

The handoff should say plainly whether guest-sdk is green locally and what the
downstream worker must change.

## Resolution Notes

At completion, add or update the request resolution. Because the request
directory is missing at planning time, use this precedence:

1. If `.agents/requests/phase3-post-ready-no-frame-under-no-tick/` exists by
   then, write the resolution there.
2. If it still does not exist, create that directory with a concise
   `00-resolution.md` or attach the same content to the implementation Beads
   issue and mention that the source request was absent.

Include:

- classification from package 01;
- test matrix and actual command outputs in summary form;
- whether the fix was guest-sdk, downstream-only, or no code change;
- any required lock bump for reference-workload;
- any wall-clock budget risk that remains under no tick.

Before ending the implementation session, follow the repository close protocol:
close finished Beads issues, `bd dolt push`, commit, `git push`, and verify
`git status` is up to date with origin.
