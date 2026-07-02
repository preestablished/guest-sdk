# 03 — SDK `register_region`: mlock + prefault + IPC client

Closes bead `guest-sdk-m4-sdk-register-region`. After this package the SDK
never writes the manifest; registration is real in **both** the under-agent
and standalone code paths (standalone now fails honestly instead of returning
a fake handle).

## Current state (anchors)

- Public entry: `detguest-sdk/src/lib.rs:301-313` `pub unsafe fn
  register_region(...)` — routes to `SdkState::register_region` (lib.rs:347,
  real-ish path) when the channel exists, else to
  `regions::register_region` (regions.rs:133, validate + fake handle).
- `regions::pin_and_translate` (regions.rs:159): plain `mlock`, no prefault,
  self-pagemap translation.
- `SdkState::publish_region` (lib.rs:374-448): seqlock manifest writer — moves
  to the agent in `02-…`; delete here.
- `RegionHandle::unregister`/`Drop` (regions.rs:126-131): no-ops.

## New behavior

`register_region(name, layout_version, ptr, len, flags)`:

1. `validate_region` (unchanged: name length, null check).
2. `len == 0` → return `Err(RegionError::NotPinned)`. (Behavior change,
   stated precisely: today under the agent a zero-length region writes a real
   manifest entry with `extent_n = 0` plus NameIntern/RegionRegister events;
   standalone returns the fake handle. Both become an error — a zero-length
   publication is meaningless to the host and unsupported by the IPC codec.
   Verified: no in-repo workload registers a zero-length region.)
3. **Pin + prefault** (`regions::pin_and_prefault`, replaces
   `pin_and_translate`): `libc::mlock(ptr, len)` (keep plain mlock — mlock2
   without ONFAULT is equivalent and mlock is already proven in-guest, see
   `m9_refwork_contract.rs:126`), then touch one byte per 4 KiB page with
   `read_volatile` so every page is faulted-in and resident before the agent
   walks pagemap. mlock failure → `NotPinned`.
4. **Intern first** (under-agent case): `intern_name(name, 0)` as today —
   the SDK `InternTable` is the single name_id authority and this emits the
   ring-W `NameIntern` exactly as the current code does (plain
   `emit_w_event`, no doorbell). Standalone (no channel): no intern table —
   see the standalone note below.
5. **IPC register** via the cached agent connection (below):
   `Request::Register{flags: flags.bits(), layout_version, name_id, gva: ptr
   as u64, len, name}` → `Reply`. Map status codes per `01-…` table.
6. On OK: if the channel is mapped, emit
   `RegionRegister(RegionEvent{region_id, name_id, layout_version,
   manifest_generation: gen as u32})` on **ring W with doorbell**
   (`emit_w_event_with_doorbell`, `EventClass::Critical`) — byte-for-byte
   today's stream shape and doorbell discipline. No channel → skip emission
   (registration still real).
6. Return `RegionHandle{region_id}` wired for unregister.

Standalone routing (no `SdkState`): return `Err(RegionError::AgentUnavailable)`
without touching the socket — there is no intern table to allocate a name_id
from and no production case where the agent exists but the channel doesn't.
(The client↔server integration tests drive `AgentClient` directly with an
explicit name_id, so they don't need the public standalone path.) Delete
`SdkState::publish_region` and the SDK's `build_extents` +
`pin_and_translate` + `translate::gva_to_gpa` usage for regions; their unit
tests move to the agent per `02-…` (do not lose the coverage).
`detguest-sdk/src/translate.rs` stays if other SDK code uses it (channel GPA
is the agent's job, but check `inject.rs`/`channel.rs` before deleting).

## The IPC client

New `detguest-sdk/src/agent_client.rs` (std-only — the SDK links std; confirm
and follow the crate's existing cfg discipline):

- `AgentClient::connect() -> Result<AgentClient, RegionError>`:
  `socket(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC)` + `connect(AGENT_SOCK_PATH)`
  (path const from `detguest_wire::regionipc`). Failure →
  `AgentUnavailable`.
- `fn call(&self, req: &Request) -> Result<Reply, RegionError>`: blocking
  send, blocking recv of exactly one datagram, decode. EOF/short →
  `AgentUnavailable`. **No timeout** (determinism; see `01-…`).
- Cache: one global `OnceLock`-style slot (mirror how `SDK: OnceLock<Sdk>` is
  held in lib.rs) holding `Mutex<Option<AgentClient>>`; connect lazily on
  first registration. A failed connect is NOT cached (retry next call) — but a
  connected client is reused for the process lifetime.
- Raw libc like the rest of the SDK (check whether `std::os::unix::net`
  supports SEQPACKET — it does not expose `UnixSeqpacket` on stable; use libc
  directly, matching `control.rs`/`m9_refwork_contract.rs` style).

## RegionHandle lifecycle

- `RegionHandle::unregister(self)`: send `Request::Unregister{region_id}`,
  best-effort (ignore `AgentUnavailable`), consume self, skip Drop
  (`mem::forget` or an internal flag).
- `Drop`: same best-effort send. Document that munmap/unpin of the memory
  itself remains the workload's business — the SDK does not munlock (pages may
  overlap other regions; keep v1 simple, munlock is NOT called).
- **Consequence for every workload (review blocker):** handles must be held
  for the process lifetime or the region goes DEAD immediately. The m9
  fixture's `publish_regions()` binds handles to locals that drop on return
  — it MUST be changed to hold them (e.g. `std::mem::forget(handle)` with a
  comment, or return them to `main` and hold across the frame loop). Same
  requirement for the new `m4_regions.rs`. This lands with `06-…` §A/§B but
  is a correctness precondition of this package's semantics — note it in the
  `register_region` doc comment.

## Tests

- `regions.rs` unit tests: prefault touch loop covers exact page boundaries
  (len == 1, len == 4096, len == 4097, tail partial page); mlock failure path
  (mlock of an unmapped address errors → NotPinned) — runnable on the dev
  host without privileges (mlock of a small mapped buffer within RLIMIT_MEMLOCK).
- Client↔server integration test (lives in detguest-agent's test tier where
  both ends are available, or a workspace `tests/` dir if one exists):
  spawn the agent's `RegionIpc` bound to a temp path in-process, point the
  client at it (make `AGENT_SOCK_PATH` overridable for tests via
  `AgentClient::connect_to(path)`; production entry stays hardwired),
  register a real mlocked buffer with an injected translator server-side,
  assert handle ids and manifest contents.
- Regression guard (request acceptance #1): a test that fails if the path
  silently regresses to a no-op — e.g. standalone `register_region` with no
  agent socket must return `Err(AgentUnavailable)`, and under the in-process
  server the manifest must contain the region's real extents (not empty, not
  `region_id 0` fake). Grep for tests asserting the old fake-handle behavior
  and update them deliberately.
- Golden hash: verified by review — no workload registers regions and the
  golden's filter drops RegionRegister, so `0x3b0d3ebc93e4ba51` must not
  shift. Treat a shift as a bug in this package (likely intern-order drift),
  not a regeneration case.

## Done when

- `cargo test -p detguest-sdk -p detguest-agent -p detguest-host --locked`
  green; clippy clean; no_std lane unaffected (SDK is std; wire stays no_std).
- The M9 fixture works against the new path with exactly two changes (both in
  `06-…` §A): the framebuffer size bump and holding the region handles for
  process lifetime — proven at the VM tier in `06-…`.
