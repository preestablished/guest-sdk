# Action Items

### Critical
- [ ] [channel.rs:91-114,175-176 / commands.rs:33-62] Ring C/I producer seqs (`next_seq_c`/`next_seq_i`) are host-only state that the doc wrongly claims is reconstructible from the event stream; `attach` resets them to 0 and there is no derive/getter/setter, so re-attach after a snapshot restore re-emits duplicate seq 0 into a ring that already holds a seq-0 record. Either derive at attach by walking `cons..prod` on C/I (records+pads), or expose `producer_seqs()`/`restore_producer_seqs()` for checkpointing — AND fix the doc comment to stop claiming C/I seqs are derivable by draining (the host never drains C/I).

### Important
- [ ] [manifest.rs:147] `read_region` does `x.gpa + to_skip` (unchecked u64) on guest-written extent data — debug panic / release wrap on a crafted `gpa`. Use `x.gpa.checked_add(to_skip).ok_or(RegionReadError::OutOfBounds)?`.
- [ ] [manifest.rs:131] `read_region` coverage check uses an unchecked `extents.iter().map(|x| x.len).sum()`; a wrapping sum can bypass the `covered < region.len` guard. Use `try_fold(0u64, |a, x| a.checked_add(x.len))`.
- [ ] [channel.rs:229 vs API.md:340] `drop_counters` returns `Result<DropCounters, MemError>` but API.md §2 declares it infallible (`-> DropCounters`). Reconcile spec and code (prefer updating the spec to the fallible form, with rationale).
- [ ] [drain.rs:17-36 vs API.md:356-362] `GuestEvent.payload` is `OwnedPayload`, but API.md §2 declares `EventPayload`. The owned form is correct; document `OwnedPayload` in API.md §2 so the normative type matches the code.

### Suggestions
- [ ] [drain.rs:269-275] Duplicate `NameIntern` with a different name is silently first-wins and uncounted; add an `intern_redefined` metric (keep first-wins) consistent with `unknown_kind_records`/`unmatched_injects`.
- [ ] [drain.rs:273] Document that `intern_name()` applies lossy UTF-8 while `GuestEvent.name` keeps raw bytes, so the two views can differ.
- [ ] [manifest.rs:113-153] Add a doc line + unit test pinning `read_region` empty-buffer (`offset == len`) and over-coverage (extents summing > region.len) behavior.
- [ ] [tests/loopback.rs] Assert that a ring wrap actually occurred / pads were consumed (e.g. count producer index crossing `size`), mirroring the existing `drops.w_records > 0` check.
- [ ] [tests/loopback.rs:170-174] The doorbell-retry branch for critical events on a full ring is likely never hit; add a counter + `assert!(doorbells > 0)` or a dedicated critical-burst-without-drain to cover it.
- [ ] [guestmem.rs:74] `add_segment` overlap loop computes existing-segment ends with unchecked `+`; use the same `checked_add(...).expect(...)` as line 70 for consistency (test-only, minor).
