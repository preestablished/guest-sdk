# Package 06 — Verification, Resolution, Handback

Runs after packages 01–04 land (05-A may still be gated; 05-B must be
done). This is the mapping from work to the request's acceptance
criteria, plus the handback file the phases track verifies against.

## Self-verification before writing the resolution

Run each acceptance criterion mechanically:

1. **4bc**: `bd show guest-sdk-4bc` → closed; the host-only re-seed
   test and the harness assertion exist and are green; the
   `snapshot.rs` limitation notes are gone/updated
   (`grep -n "cannot be re-seeded" tests/vm/src/harness/snapshot.rs`
   → no hits).
2. **Checklist**: `bd show` both ext-hyp beads → descriptions carry
   `ILDE-1..7` / `DRL-1..5`; the ack note exists in the hypervisor
   request dir (committed in *their* repo — verify with `git -C
   ../determinism-hypervisor log --oneline -3`).
3. **Triage**: for each bead in the package-03 table, `bd show`
   matches its disposition row — closed with reason, or NOTES citing
   the specific checklist item / round-2 item it waits on. No bead
   left saying only "blocked after current pass."
4. **Scaffold**: from a clean checkout (fresh `git clone` or
   `git stash`+clean worktree — the verifier will do this, so do it
   first): `cargo test -p detguest-vmtest` runs the fixture round
   trip, all four deliberate-mismatch negatives, and the seed-variation
   test, green with the bodies executing. Grep the stub marker
   (`MS5-STUB`) and confirm every hit cites a round-2 item +
   checklist ID.
5. **Refwork residual**: either the run record exists in the no-frame
   request dir, or the resolution documents the still-closed gate.
6. **Handoff receipt**: covered by 2 — diff table recorded, beads
   flipped/annotated, ack present.

Also run the repo's standard host-only gates (fmt, clippy, full
host-only test suite) — the phases track re-runs from clean checkout
and anything red there bounces the whole handback.

## The resolution file

Append `04-resolution.md` to
`.agents/requests/phase3-ms5-groundwork-while-blocked/` (continuing its
numbering), same convention as the five resolved request dirs. Content
the verification offer names:

- git SHAs per package (this repo; plus the hypervisor-repo SHA of the
  ack note).
- Bead dispositions: the package-03 triage table as executed, plus
  4bc and the two ext-hyp flips (with the DRL-4 caveat line).
- Checklist location (the two bead descriptions) and the diff table
  (checklist ID → handback evidence → verdict).
- Scaffold self-test output (paste the test-run summary, name the
  negative tests individually).
- The 05-A record or its gate state; the 05-B pointer.
- Note the order inversion: evidence arrived first, checklist diff on
  receipt — the request's sanctioned fallback — so the verifier knows
  which of the two valid orders to check (their step 3 anticipates
  exactly this).

## Session close (per CLAUDE.md — every working session, not just the last)

```bash
git pull --rebase
bd dolt push
git push
git status   # MUST show up to date with origin
```

Beads and code commits ride together; the hypervisor-repo ack commit
is pushed in that repo. Work is not complete until both pushes
succeed.
