//! SDK↔agent region-registration IPC over `/run/detguest/agent.sock`
//! (API.md §1.5; ARCHITECTURE.md §5).
//!
//! Transport is AF_UNIX `SOCK_SEQPACKET`: one datagram = one message, fixed
//! little-endian layout, hand-written codecs like the rest of this crate —
//! no serde, no hashing, total non-panicking decoders. The caller's pid is
//! bound via `SO_PEERCRED` on the accepted connection and never travels in a
//! message.
//!
//! `name_id` is allocated by the caller (the SDK's intern table is the single
//! name-id authority — the host folds `NameIntern` records from rings A and W
//! into one map, so a second allocator would collide); the agent echoes it in
//! the reply and in its ring-A evidence events.

use crate::{DecodeError, EncodeError};

/// Canonical agent socket path. Shared here so SDK and agent agree by
/// construction.
pub const AGENT_SOCK_PATH: &str = "/run/detguest/agent.sock";

/// Message magic `"DGRR"` as a little-endian u32.
pub const REGIONIPC_MAGIC: u32 = 0x5252_4744;
/// Protocol version this crate implements.
pub const REGIONIPC_VERSION: u16 = 1;
/// Upper bound on any regionipc datagram (largest is RegisterRegion at 94
/// bytes with a full 56-byte name).
pub const REGIONIPC_MAX_DATAGRAM: usize = 128;

/// Message kind: SDK→agent RegisterRegion.
pub const KIND_REGISTER: u16 = 1;
/// Message kind: SDK→agent UnregisterRegion.
pub const KIND_UNREGISTER: u16 = 2;
/// Message kind: agent→SDK Reply.
pub const KIND_REPLY: u16 = 3;

/// Reply status: success.
pub const STATUS_OK: u16 = 0;
/// Reply status: no free region slot in the manifest.
pub const STATUS_MANIFEST_FULL: u16 = 1;
/// Reply status: region would exceed the manifest extent pool.
pub const STATUS_TOO_MANY_EXTENTS: u16 = 2;
/// Reply status: pagemap says the bytes are not present and pinned.
pub const STATUS_NOT_PINNED: u16 = 3;
/// Reply status: region name exceeds the manifest field.
pub const STATUS_NAME_TOO_LONG: u16 = 4;
/// Reply status: malformed request datagram (SDK bug).
pub const STATUS_BAD_REQUEST: u16 = 5;
/// Reply status: peer pid is not the supervised workload.
pub const STATUS_UNKNOWN_PID: u16 = 6;
/// Reply status: unregister of an unknown or already-dead region id.
pub const STATUS_UNKNOWN_REGION: u16 = 7;
/// Reply status: agent-side I/O failure.
pub const STATUS_INTERNAL: u16 = 8;

/// `RegionEntry.flags` bit 31 (DEAD) — must be clear in a register request.
const FLAG_DEAD: u32 = 1 << 31;

const OFF_MAGIC: usize = 0;
const OFF_VERSION: usize = 4;
const OFF_KIND: usize = 6;
const HEADER_LEN: usize = 8;

const REG_OFF_FLAGS: usize = 8;
const REG_OFF_LAYOUT: usize = 12;
const REG_OFF_NAME_ID: usize = 16;
const REG_OFF_GVA: usize = 20;
const REG_OFF_LEN: usize = 28;
const REG_OFF_NAME_LEN: usize = 36;
const REG_OFF_NAME: usize = 38;

const UNREG_OFF_REGION_ID: usize = 8;
const UNREG_LEN: usize = 12;

const REPLY_OFF_STATUS: usize = 8;
const REPLY_OFF_RESERVED: usize = 10;
const REPLY_OFF_REGION_ID: usize = 12;
const REPLY_OFF_NAME_ID: usize = 16;
const REPLY_OFF_GENERATION: usize = 20;
const REPLY_LEN: usize = 28;

/// One SDK→agent request datagram.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request<'a> {
    /// Publish `[gva, gva+len)` under `name` at `layout_version`.
    Register {
        /// `RegionFlags` bits; bit 31 (DEAD) must be clear.
        flags: u32,
        /// Workload-declared layout version.
        layout_version: u32,
        /// Caller-interned name id; never 0.
        name_id: u32,
        /// Region base in the caller's address space.
        gva: u64,
        /// Region length in bytes; never 0.
        len: u64,
        /// UTF-8 name bytes, 1..=56 bytes.
        name: &'a [u8],
    },
    /// Unregister a previously registered region.
    Unregister {
        /// Manifest region slot id from the register reply.
        region_id: u32,
    },
}

/// The agent→SDK reply datagram.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reply {
    /// One of the `STATUS_*` codes.
    pub status: u16,
    /// Manifest region slot id; valid iff `status == STATUS_OK`.
    pub region_id: u32,
    /// Echo of the request's name id (register); valid iff OK.
    pub name_id: u32,
    /// Post-write (even) manifest generation; valid iff OK.
    pub manifest_generation: u64,
}

/// Maximum region name bytes accepted on the wire (mirrors
/// [`crate::manifest::MAX_REGION_NAME`]).
pub const MAX_NAME_LEN: usize = crate::manifest::MAX_REGION_NAME;

fn write_header(out: &mut [u8], kind: u16) {
    out[OFF_MAGIC..OFF_MAGIC + 4].copy_from_slice(&REGIONIPC_MAGIC.to_le_bytes());
    out[OFF_VERSION..OFF_VERSION + 2].copy_from_slice(&REGIONIPC_VERSION.to_le_bytes());
    out[OFF_KIND..OFF_KIND + 2].copy_from_slice(&kind.to_le_bytes());
}

fn check_header(bytes: &[u8]) -> Result<u16, DecodeError> {
    if bytes.len() < HEADER_LEN {
        return Err(DecodeError::Truncated);
    }
    let magic = u32::from_le_bytes(bytes[OFF_MAGIC..OFF_MAGIC + 4].try_into().unwrap());
    if magic != REGIONIPC_MAGIC {
        return Err(DecodeError::BadMagic);
    }
    let version = u16::from_le_bytes(bytes[OFF_VERSION..OFF_VERSION + 2].try_into().unwrap());
    if version != REGIONIPC_VERSION {
        return Err(DecodeError::BadVersion);
    }
    Ok(u16::from_le_bytes(
        bytes[OFF_KIND..OFF_KIND + 2].try_into().unwrap(),
    ))
}

/// Encode a request into `out`; returns the datagram length.
pub fn encode_request(req: &Request<'_>, out: &mut [u8]) -> Result<usize, EncodeError> {
    match *req {
        Request::Register {
            flags,
            layout_version,
            name_id,
            gva,
            len,
            name,
        } => {
            if name.is_empty() || name.len() > MAX_NAME_LEN {
                return Err(EncodeError::FieldTooLong);
            }
            let total = REG_OFF_NAME + name.len();
            if out.len() < total {
                return Err(EncodeError::BufferTooSmall);
            }
            write_header(out, KIND_REGISTER);
            out[REG_OFF_FLAGS..REG_OFF_FLAGS + 4].copy_from_slice(&flags.to_le_bytes());
            out[REG_OFF_LAYOUT..REG_OFF_LAYOUT + 4].copy_from_slice(&layout_version.to_le_bytes());
            out[REG_OFF_NAME_ID..REG_OFF_NAME_ID + 4].copy_from_slice(&name_id.to_le_bytes());
            out[REG_OFF_GVA..REG_OFF_GVA + 8].copy_from_slice(&gva.to_le_bytes());
            out[REG_OFF_LEN..REG_OFF_LEN + 8].copy_from_slice(&len.to_le_bytes());
            out[REG_OFF_NAME_LEN..REG_OFF_NAME_LEN + 2]
                .copy_from_slice(&(name.len() as u16).to_le_bytes());
            out[REG_OFF_NAME..total].copy_from_slice(name);
            Ok(total)
        }
        Request::Unregister { region_id } => {
            if out.len() < UNREG_LEN {
                return Err(EncodeError::BufferTooSmall);
            }
            write_header(out, KIND_UNREGISTER);
            out[UNREG_OFF_REGION_ID..UNREG_OFF_REGION_ID + 4]
                .copy_from_slice(&region_id.to_le_bytes());
            Ok(UNREG_LEN)
        }
    }
}

/// Decode one request datagram. Trailing bytes are rejected; every failure is
/// a typed error (decoders never panic on arbitrary bytes).
pub fn decode_request(bytes: &[u8]) -> Result<Request<'_>, DecodeError> {
    match check_header(bytes)? {
        KIND_REGISTER => {
            if bytes.len() < REG_OFF_NAME {
                return Err(DecodeError::Truncated);
            }
            let flags =
                u32::from_le_bytes(bytes[REG_OFF_FLAGS..REG_OFF_FLAGS + 4].try_into().unwrap());
            let layout_version = u32::from_le_bytes(
                bytes[REG_OFF_LAYOUT..REG_OFF_LAYOUT + 4]
                    .try_into()
                    .unwrap(),
            );
            let name_id = u32::from_le_bytes(
                bytes[REG_OFF_NAME_ID..REG_OFF_NAME_ID + 4]
                    .try_into()
                    .unwrap(),
            );
            let gva = u64::from_le_bytes(bytes[REG_OFF_GVA..REG_OFF_GVA + 8].try_into().unwrap());
            let len = u64::from_le_bytes(bytes[REG_OFF_LEN..REG_OFF_LEN + 8].try_into().unwrap());
            let name_len = u16::from_le_bytes(
                bytes[REG_OFF_NAME_LEN..REG_OFF_NAME_LEN + 2]
                    .try_into()
                    .unwrap(),
            ) as usize;
            if name_len == 0 || name_len > MAX_NAME_LEN {
                return Err(DecodeError::BadField);
            }
            if bytes.len() < REG_OFF_NAME + name_len {
                return Err(DecodeError::Truncated);
            }
            if bytes.len() != REG_OFF_NAME + name_len {
                return Err(DecodeError::BadLen);
            }
            if flags & FLAG_DEAD != 0 || name_id == 0 || len == 0 {
                return Err(DecodeError::BadField);
            }
            Ok(Request::Register {
                flags,
                layout_version,
                name_id,
                gva,
                len,
                name: &bytes[REG_OFF_NAME..REG_OFF_NAME + name_len],
            })
        }
        KIND_UNREGISTER => {
            if bytes.len() < UNREG_LEN {
                return Err(DecodeError::Truncated);
            }
            if bytes.len() != UNREG_LEN {
                return Err(DecodeError::BadLen);
            }
            Ok(Request::Unregister {
                region_id: u32::from_le_bytes(
                    bytes[UNREG_OFF_REGION_ID..UNREG_OFF_REGION_ID + 4]
                        .try_into()
                        .unwrap(),
                ),
            })
        }
        // `kind` is u16 but `DecodeError::UnknownKind` carries u8; unknown
        // kinds (including a stray Reply sent agent-ward) map to BadField.
        _ => Err(DecodeError::BadField),
    }
}

/// Encode a reply into `out`; returns the datagram length.
pub fn encode_reply(reply: &Reply, out: &mut [u8]) -> Result<usize, EncodeError> {
    if out.len() < REPLY_LEN {
        return Err(EncodeError::BufferTooSmall);
    }
    write_header(out, KIND_REPLY);
    out[REPLY_OFF_STATUS..REPLY_OFF_STATUS + 2].copy_from_slice(&reply.status.to_le_bytes());
    out[REPLY_OFF_RESERVED..REPLY_OFF_RESERVED + 2].copy_from_slice(&0u16.to_le_bytes());
    out[REPLY_OFF_REGION_ID..REPLY_OFF_REGION_ID + 4]
        .copy_from_slice(&reply.region_id.to_le_bytes());
    out[REPLY_OFF_NAME_ID..REPLY_OFF_NAME_ID + 4].copy_from_slice(&reply.name_id.to_le_bytes());
    out[REPLY_OFF_GENERATION..REPLY_OFF_GENERATION + 8]
        .copy_from_slice(&reply.manifest_generation.to_le_bytes());
    Ok(REPLY_LEN)
}

/// Decode one reply datagram (exact length; trailing bytes rejected).
pub fn decode_reply(bytes: &[u8]) -> Result<Reply, DecodeError> {
    if check_header(bytes)? != KIND_REPLY {
        return Err(DecodeError::BadField);
    }
    if bytes.len() < REPLY_LEN {
        return Err(DecodeError::Truncated);
    }
    if bytes.len() != REPLY_LEN {
        return Err(DecodeError::BadLen);
    }
    Ok(Reply {
        status: u16::from_le_bytes(
            bytes[REPLY_OFF_STATUS..REPLY_OFF_STATUS + 2]
                .try_into()
                .unwrap(),
        ),
        region_id: u32::from_le_bytes(
            bytes[REPLY_OFF_REGION_ID..REPLY_OFF_REGION_ID + 4]
                .try_into()
                .unwrap(),
        ),
        name_id: u32::from_le_bytes(
            bytes[REPLY_OFF_NAME_ID..REPLY_OFF_NAME_ID + 4]
                .try_into()
                .unwrap(),
        ),
        manifest_generation: u64::from_le_bytes(
            bytes[REPLY_OFF_GENERATION..REPLY_OFF_GENERATION + 8]
                .try_into()
                .unwrap(),
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip_request(req: Request<'_>) {
        let mut buf = [0u8; REGIONIPC_MAX_DATAGRAM];
        let n = encode_request(&req, &mut buf).unwrap();
        assert!(n <= REGIONIPC_MAX_DATAGRAM);
        assert_eq!(decode_request(&buf[..n]).unwrap(), req);
    }

    #[test]
    fn register_round_trips_min_and_max_names() {
        round_trip_request(Request::Register {
            flags: 0x3,
            layout_version: 1,
            name_id: 7,
            gva: 0x7f00_dead_b000,
            len: 229_376,
            name: b"w",
        });
        let name = [b'n'; MAX_NAME_LEN];
        round_trip_request(Request::Register {
            flags: 0,
            layout_version: 9,
            name_id: u32::MAX,
            gva: u64::MAX,
            len: 1,
            name: &name,
        });
    }

    #[test]
    fn unregister_and_reply_round_trip() {
        round_trip_request(Request::Unregister { region_id: 63 });
        let reply = Reply {
            status: STATUS_OK,
            region_id: 3,
            name_id: 5,
            manifest_generation: 12,
        };
        let mut buf = [0u8; REGIONIPC_MAX_DATAGRAM];
        let n = encode_reply(&reply, &mut buf).unwrap();
        assert_eq!(n, REPLY_LEN);
        assert_eq!(decode_reply(&buf[..n]).unwrap(), reply);
    }

    #[test]
    fn max_register_fits_documented_cap() {
        let name = [b'n'; MAX_NAME_LEN];
        let mut buf = [0u8; REGIONIPC_MAX_DATAGRAM];
        let n = encode_request(
            &Request::Register {
                flags: 0,
                layout_version: 1,
                name_id: 1,
                gva: 0,
                len: 1,
                name: &name,
            },
            &mut buf,
        )
        .unwrap();
        assert_eq!(n, 94);
    }

    fn encoded_register() -> ([u8; REGIONIPC_MAX_DATAGRAM], usize) {
        let mut buf = [0u8; REGIONIPC_MAX_DATAGRAM];
        let n = encode_request(
            &Request::Register {
                flags: 1,
                layout_version: 1,
                name_id: 2,
                gva: 0x1000,
                len: 4096,
                name: b"wram",
            },
            &mut buf,
        )
        .unwrap();
        (buf, n)
    }

    #[test]
    fn malformed_datagrams_are_typed_errors() {
        let (buf, n) = encoded_register();

        // Short: below header, below fixed prefix, below name end.
        assert_eq!(decode_request(&buf[..4]), Err(DecodeError::Truncated));
        assert_eq!(decode_request(&buf[..20]), Err(DecodeError::Truncated));
        assert_eq!(decode_request(&buf[..n - 1]), Err(DecodeError::Truncated));

        // Trailing bytes.
        assert_eq!(decode_request(&buf[..n + 1]), Err(DecodeError::BadLen));

        // Bad magic / version / kind.
        let mut bad = buf;
        bad[0] ^= 1;
        assert_eq!(decode_request(&bad[..n]), Err(DecodeError::BadMagic));
        let mut bad = buf;
        bad[4] = 0xFF;
        assert_eq!(decode_request(&bad[..n]), Err(DecodeError::BadVersion));
        let mut bad = buf;
        bad[6] = 0x77;
        bad[7] = 0x77;
        assert_eq!(decode_request(&bad[..n]), Err(DecodeError::BadField));

        // Reply kind sent as a request.
        let mut reply_buf = [0u8; REGIONIPC_MAX_DATAGRAM];
        let rn = encode_reply(
            &Reply {
                status: STATUS_OK,
                region_id: 0,
                name_id: 1,
                manifest_generation: 2,
            },
            &mut reply_buf,
        )
        .unwrap();
        assert_eq!(decode_request(&reply_buf[..rn]), Err(DecodeError::BadField));
    }

    #[test]
    fn bad_fields_are_rejected() {
        // name_len 0.
        let (mut buf, n) = encoded_register();
        buf[REG_OFF_NAME_LEN] = 0;
        buf[REG_OFF_NAME_LEN + 1] = 0;
        assert_eq!(decode_request(&buf[..n]), Err(DecodeError::BadField));

        // name_len > 56.
        let (mut buf, n) = encoded_register();
        buf[REG_OFF_NAME_LEN] = 57;
        assert_eq!(decode_request(&buf[..n]), Err(DecodeError::BadField));

        // name_len beyond the datagram (within cap).
        let (mut buf, n) = encoded_register();
        buf[REG_OFF_NAME_LEN] = 56;
        assert_eq!(decode_request(&buf[..n]), Err(DecodeError::Truncated));

        // DEAD flag set.
        let (mut buf, n) = encoded_register();
        buf[REG_OFF_FLAGS + 3] |= 0x80;
        assert_eq!(decode_request(&buf[..n]), Err(DecodeError::BadField));

        // name_id 0.
        let (mut buf, n) = encoded_register();
        buf[REG_OFF_NAME_ID..REG_OFF_NAME_ID + 4].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(decode_request(&buf[..n]), Err(DecodeError::BadField));

        // len 0.
        let (mut buf, n) = encoded_register();
        buf[REG_OFF_LEN..REG_OFF_LEN + 8].copy_from_slice(&0u64.to_le_bytes());
        assert_eq!(decode_request(&buf[..n]), Err(DecodeError::BadField));

        // Non-UTF-8 name is accepted at the codec layer (names are bytes
        // here; UTF-8 policy lives at the manifest layer).
        let (mut buf, n) = encoded_register();
        buf[REG_OFF_NAME] = 0xFF;
        assert!(decode_request(&buf[..n]).is_ok());
    }

    #[test]
    fn unregister_length_is_exact() {
        let mut buf = [0u8; REGIONIPC_MAX_DATAGRAM];
        let n = encode_request(&Request::Unregister { region_id: 1 }, &mut buf).unwrap();
        assert_eq!(n, UNREG_LEN);
        assert_eq!(decode_request(&buf[..n - 1]), Err(DecodeError::Truncated));
        assert_eq!(decode_request(&buf[..n + 1]), Err(DecodeError::BadLen));
    }

    #[test]
    fn reply_length_is_exact() {
        let mut buf = [0u8; REGIONIPC_MAX_DATAGRAM];
        let n = encode_reply(
            &Reply {
                status: STATUS_NOT_PINNED,
                region_id: 0,
                name_id: 0,
                manifest_generation: 0,
            },
            &mut buf,
        )
        .unwrap();
        assert_eq!(decode_reply(&buf[..n - 1]), Err(DecodeError::Truncated));
        assert_eq!(decode_reply(&buf[..n + 1]), Err(DecodeError::BadLen));
        assert_eq!(decode_reply(&buf[..n]).unwrap().status, STATUS_NOT_PINNED);
    }

    #[test]
    fn oversized_name_rejected_at_encode() {
        let name = [b'n'; MAX_NAME_LEN + 1];
        let mut buf = [0u8; REGIONIPC_MAX_DATAGRAM];
        assert_eq!(
            encode_request(
                &Request::Register {
                    flags: 0,
                    layout_version: 1,
                    name_id: 1,
                    gva: 0,
                    len: 1,
                    name: &name,
                },
                &mut buf,
            ),
            Err(EncodeError::FieldTooLong)
        );
        assert_eq!(
            encode_request(
                &Request::Register {
                    flags: 0,
                    layout_version: 1,
                    name_id: 1,
                    gva: 0,
                    len: 1,
                    name: b"",
                },
                &mut buf,
            ),
            Err(EncodeError::FieldTooLong)
        );
    }

    #[test]
    fn arbitrary_bytes_never_panic() {
        // Cheap in-test sweep alongside the crate fuzz target.
        for len in 0..REGIONIPC_MAX_DATAGRAM {
            let bytes: alloc::vec::Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            let _ = decode_request(&bytes);
            let _ = decode_reply(&bytes);
        }
    }
}
