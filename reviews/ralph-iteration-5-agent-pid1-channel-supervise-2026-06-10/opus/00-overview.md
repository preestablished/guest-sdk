# Review Overview — detguest-agent PID 1 (Milestone 2 guest side)

- **Branch:** `ralph/iteration-5-agent-pid1-channel-supervise`
  (HEAD `9e4097d`; the working branch name in git is the stale
  `ralph/iteration-2-…`, but the tip commit is the iteration-5 checkpoint)
- **Base:** `main`
- **Date:** 2026-06-10
- **Reviewer:** Claude Opus
- **Scope:** `crates/detguest-agent/` (boot.rs, channel.rs, commands.rs, pio.rs,
  runtime.rs, supervise.rs, translate.rs, lib.rs, main.rs) + `image/boot.toml.m2`,
  reviewed against ARCHITECTURE.md §2–§4/§6/§7 and API.md §3/§5/§6/§7.

## Summary

This is a strong, spec-literate implementation of the in-guest PID 1 agent. The
boot sequence in `runtime.rs` follows ARCHITECTURE.md §4 step-for-step (mounts →
alloc → iopl → IDENT → pagemap → CHANNEL_INIT → `agent_ready` → Hello → boot.toml →
autostart → Ready), pre-channel failures correctly fall back to `eprintln` + power-off
(no LogLine possible before the channel exists), and `power_off` is properly guarded on
`PID == 1`. The unsafe surface is small, module-scoped, and well-commented; the
`init_at` zero-init contract is satisfied by `ftruncate`+`MAP_SHARED` (mmap of a freshly
ftruncated file is zero-filled, as the SAFETY comment reasons). `boot.rs` covers every
§7.2 rule (major check, dense/unique ids, absolute exec, default args/log_mask, region
name cap, dup region names, refwork-ctl `game_dev` requirement, autostart→missing-unit).
The fork/exec in `supervise.rs` builds all `CString`s before `fork()` and uses `_exit`,
avoiding the classic allocate-after-fork hazard, and the partial-line flush ordering at
reap (drain → finish → WorkloadExited) matches the M2 LogLine-framing acceptance. Tests
(17 agent unit tests + full workspace), clippy `--all-targets`, and the musl release
build all pass clean.

Two issues hold back an unqualified approval. The most important is a **compile-time
reordering hazard in `pio.rs`**: the doorbell `OUT` is marked `options(nomem, …)`,
which permits the compiler to hoist the port write above the `Release` store that
publishes the just-written ring record — directly undermining the normative
"record visible before the OUT" discipline (ARCHITECTURE.md §2 / API.md §5). The second
is the **ring-I relay seq derivation** in `channel.rs`: `seq = prod / total` neither
matches the host producer's monotonic `next_seq_i` counter nor survives a tail pad, and
reuses one seq for both the pad and the record — a §7-rule-3 conformance gap that the
token-matched COOP/FORCED protocol happens to tolerate today but should not be relied on.

## Verdict

**REQUEST_CHANGES** — one Critical (pio `nomem` doorbell ordering) and one Important
(ring-I relay seq derivation) finding. Both are localized; everything else is solid.

## Stats

| Metric | Value |
|---|---|
| Files reviewed | 9 agent source files + boot.toml.m2 |
| Lines added (vs main) | ~2000 |
| Critical findings | 1 |
| Important findings | 2 |
| Suggestions | 7 |
| Tests | 17 agent unit tests pass; `cargo test --workspace` green |
| Clippy | clean (`--workspace --all-targets`) |
| musl release build | `x86_64-unknown-linux-musl -p detguest-agent` succeeds |
