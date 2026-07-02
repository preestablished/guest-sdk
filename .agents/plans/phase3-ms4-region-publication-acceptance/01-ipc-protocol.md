# 01 — Region-registration IPC protocol (agent.sock)

Closes bead `guest-sdk-m4-agent-ipc-protocol`. Defines the SDK↔agent protocol
for `/run/detguest/agent.sock` per ARCHITECTURE.md §5 and API.md §1.5.

## Constraints (from the bead + repo conventions)

- AF_UNIX **SOCK_SEQPACKET**; one datagram = one message.
- **No serde, no random hashing** in guest hot paths — hand-rolled fixed-layout
  little-endian codec, same style as `detguest-wire` (total, non-panicking
  decoders; every decode error is a typed variant).
- Pid binding comes from `SO_PEERCRED` on the accepted connection, **never**
  from message contents.
- Deterministic error mapping: every failure mode has exactly one status code.

## Where the code lives

New module `crates/detguest-wire/src/regionipc.rs` (exported from
`detguest-wire/src/lib.rs`). Both the SDK client and the agent server must
share one codec; `detguest-wire` is the existing shared no-std-friendly crate
and already holds every other wire format. (The bead's file-reservation note
predates this layout; API.md is the normative home for the spec text — see
`07-…` for the doc update.)

Keep the module `no_std`-compatible like the rest of the crate if feasible
(alloc for name bytes is fine — check how `events.rs` handles borrowed
payloads and mirror it: borrow `&[u8]` name from the datagram buffer).

## Messages

All integers little-endian. Datagram max size: `REGIONIPC_MAX_DATAGRAM = 128`
bytes (largest message is RegisterRegion: 4+2+2+4+4+4+8+8+2+56 = 94).

### Common header (8 bytes)

| off | type | field | value |
|---|---|---|---|
| 0 | u32 | magic | `REGIONIPC_MAGIC = 0x52524744` ("DGRR" LE) |
| 4 | u16 | proto_version | `REGIONIPC_VERSION = 1` |
| 6 | u16 | kind | message kind |

Kinds: `1 = RegisterRegion` (SDK→agent), `2 = UnregisterRegion` (SDK→agent),
`3 = Reply` (agent→SDK). Unknown kind / bad magic / bad version at the server
→ Reply with `STATUS_BAD_REQUEST` when a reply address exists (connection is
open), and the connection is closed after replying.

### RegisterRegion (kind 1), payload after header

| off | type | field | notes |
|---|---|---|---|
| 8 | u32 | flags | `RegionFlags` bits; bit 31 (DEAD) must be clear |
| 12 | u32 | layout_version | workload-declared |
| 16 | u32 | name_id | caller-interned id (SDK `InternTable` is the single name_id authority — the host folds ring-A and ring-W `NameIntern` into one map, so the agent must NOT run its own counter); != 0 |
| 20 | u64 | gva | region base in caller's address space |
| 28 | u64 | len | bytes; > 0 |
| 36 | u16 | name_len | 1..=56 (`MAX_REGION_NAME`) |
| 38 | .. | name | UTF-8, exactly `name_len` bytes, no padding |

### UnregisterRegion (kind 2)

| off | type | field |
|---|---|---|
| 8 | u32 | region_id |

### Reply (kind 3)

| off | type | field | notes |
|---|---|---|---|
| 8 | u16 | status | see codes |
| 10 | u16 | reserved | 0 |
| 12 | u32 | region_id | valid iff status == OK |
| 16 | u32 | name_id | echo of the request's name_id, iff OK (register only) |
| 20 | u64 | manifest_generation | post-write even generation, iff OK |

### Status codes (deterministic mapping)

| code | const | → SDK `RegionError` |
|---|---|---|
| 0 | `STATUS_OK` | — |
| 1 | `STATUS_MANIFEST_FULL` | `ManifestFull` |
| 2 | `STATUS_TOO_MANY_EXTENTS` | `TooManyExtents` |
| 3 | `STATUS_NOT_PINNED` | `NotPinned` (pagemap says not-present/swapped/pfn-hidden) |
| 4 | `STATUS_NAME_TOO_LONG` | `NameTooLong` |
| 5 | `STATUS_BAD_REQUEST` | `AgentUnavailable` (malformed datagram — SDK bug) |
| 6 | `STATUS_UNKNOWN_PID` | `AgentUnavailable` (peer is not the supervised workload) |
| 7 | `STATUS_UNKNOWN_REGION` | `AgentUnavailable` (unregister of unknown/dead id) |
| 8 | `STATUS_INTERNAL` | `AgentUnavailable` (agent-side I/O failure) |

Client-side transport failures (socket missing, connect refused, short send,
EOF) all map to `RegionError::AgentUnavailable`.

## Session model

- SDK opens **one** connection lazily on first `register_region` and caches it
  for the process lifetime (`SdkState`). Requests are strictly
  send-one-recv-one; SEQPACKET preserves message boundaries and ordering.
- The agent accepts at most a small fixed number of concurrent connections
  (`REGIONIPC_MAX_CONNS = 4`); further accepts are immediately closed. v1 has
  a single supervised workload, so one live connection is the norm.
- No timeouts on the guest side: determinism forbids wall-clock behavior. A
  hung agent means a hung workload, which the supervise/watchdog tier owns.

## API shape

```rust
// detguest-wire/src/regionipc.rs
pub enum Request<'a> {
    Register { flags: u32, layout_version: u32, name_id: u32, gva: u64, len: u64, name: &'a [u8] },
    Unregister { region_id: u32 },
}
pub struct Reply { pub status: u16, pub region_id: u32, pub name_id: u32, pub manifest_generation: u64 }

pub fn encode_request(req: &Request<'_>, out: &mut [u8]) -> Result<usize, EncodeError>;
pub fn decode_request(bytes: &[u8]) -> Result<Request<'_>, DecodeError>;
pub fn encode_reply(reply: &Reply, out: &mut [u8]) -> Result<usize, EncodeError>;
pub fn decode_reply(bytes: &[u8]) -> Result<Reply, DecodeError>;
```

Reuse the crate's existing `EncodeError`/`DecodeError` types. One gap: the
header `kind` is u16 but `DecodeError::UnknownKind` carries u8 — map unknown
kinds to `DecodeError::BadField` (do not truncate, do not add a variant; the
server replies `STATUS_BAD_REQUEST` either way). `name_id == 0` decodes to
`BadField` as well.

## Socket path

`pub const AGENT_SOCK_PATH: &str = "/run/detguest/agent.sock";` — lives in
`detguest-wire::regionipc` so SDK and agent agree by construction. The agent
creates `/run/detguest/` (0755) and binds before the autostart unit spawns
(see `02-…`).

## Tests (bead acceptance: malformed requests covered)

In `regionipc.rs` unit tests:
- Round-trip every message kind, min/max name lengths (1 and 56).
- Malformed: short datagram (< header, < payload), bad magic, bad version,
  unknown kind, `name_len` 0, `name_len` > 56, `name_len` beyond datagram,
  trailing bytes (reject), non-UTF-8 name (accept at codec layer — name is
  bytes; UTF-8 validation is the agent's manifest-layer concern, matching
  `RegionEntry::pack_name` which takes bytes), len == 0 (reject at codec: a
  zero-length region is not registrable over IPC — the SDK short-circuits
  those locally, see `03-…`).
- Decoders never panic on arbitrary bytes: add a case to the existing fuzz
  target that exercises wire decoders (`fuzz/`) if one covers `detguest-wire`
  decoders generally — check `fuzz/fuzz_targets/` and extend the existing
  pattern; do not build new fuzz infrastructure in this package.

## Done when

- `detguest-wire` builds no-std as before (`ci.yaml` `no_std` lane), all new
  unit tests pass, fuzz target builds.
- API.md protocol appendix drafted (finalized in `07-…`).
