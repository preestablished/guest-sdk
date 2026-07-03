# Request: Materialize The Game Device — The Last Gap Before READY

## Who Is Asking

The rom-operator-bridge session driving Phase 3 step 2 (first boot of
the real workload image), 2026-07-03. Fifth request in the series; the
prior four all resolved (trails in the sibling repos).

## The Finding, Reproduced Precisely

The rebuilt package-04 image now boots end-to-end **except for one
thing**. Serial + event capture (new diagnostic:
`tests/vm/tests/boot_probe.rs` in this repo) shows:

```text
kernel 6.12.93 boots → Run /init as init process
GuestEvent Hello   { proto_version: 1, agent_version: 256, capabilities: 3 }
GuestEvent WorkloadStarted { guest_pid: 15, unit: 0 }
GuestEvent LogLine "refwork fault after LoadGame: cannot read game
                    path `/dev/vdb`: No such file or directory"
→ agent powers off (GUEST_HALTED under the real worker)
```

Mounts, channel bring-up, boot.toml parse, unit spawn, the refwork-ctl
handshake, fault reporting — all work. (A prior blocker, the image's
`/init` being a shell script in a shell-less image, is already fixed in
reference-workload `fbf32c6`.)

## The Gap

`boot.toml` promises `game_dev = "/dev/vdb"` and the harness does an
ordinary filesystem read of that path. But **no `/dev/vdb` exists in
the guest**: the pinned kernel has no pv-blk block driver, and the M9
staged fixture never needed one — `m9_refwork_contract.rs` read the
game via raw pv-blk MMIO through `/dev/mem` (its own probe code). The
`game_dev` string was protocol theater in the fixture era. Nobody owns
the guest-side path from the pv-blk device to a readable game file.

## The Ask

Close the gap on the agent side (your `[unit.control]`/API.md §7.1
surface — `01-options.md` argues why and offers alternatives): before
driving `LoadGame`, the agent materializes the game bytes from pv-blk
into a file the unit can read, and passes that path. Everything
downstream is already proven waiting: the harness faults precisely on a
bad path today and loads/validates the ROM once it can read one.

## Files

| File | Contents |
|---|---|
| `01-options.md` | Three implementation options with a recommendation; determinism constraints |
| `02-verification.md` | How to reproduce, the ready-made probe, and what green looks like |
