//! Minimal reference-workload control driver for `[unit.control]`.
//!
//! The protocol wire format is owned by reference-workload's
//! `refwork-protocol` crate. To keep the initramfs agent dependency-light and
//! avoid cross-repo path dependencies, this module implements only the stable
//! postcard byte shapes required for the boot-time leg:
//! `Hello -> HelloAck -> LoadGame -> GameLoaded -> Ready -> Start`.
#![allow(unsafe_code)]

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use crate::boot::UnitControl;

const CONTROL_FD: i32 = 3;
const MAX_DATAGRAM: usize = 4096;

/// Reply-wait spin cap. Each iteration is a non-blocking recv + the idle
/// callback (region-IPC servicing) + sched_yield — roughly a microsecond —
/// so the prod cap represents minutes, sized against the slowest legitimate
/// leg (a reference workload's LoadGame doing real device I/O), not against
/// region publication. Hitting it means the workload is dead or wedged:
/// boot-fault instead of hanging the boot forever (this is a deliberate
/// semantic change from the old indefinitely-blocking recv).
#[cfg(test)]
const CONTROL_RECV_POLL_LIMIT: usize = 512;
#[cfg(not(test))]
const CONTROL_RECV_POLL_LIMIT: usize = 200_000_000;

#[derive(Debug)]
pub(crate) struct ControlSocket {
    fd: OwnedFd,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ControlReply {
    HelloAck {
        proto_version: u16,
    },
    GameLoaded,
    /// Informational region announcement (refwork-protocol tag 7,
    /// `RegisterRegion`) sent between GameLoaded and Ready. The agent
    /// gates Ready on the detchannel manifest, not on these messages, so
    /// they are skipped wherever a reply is awaited. The staged M9
    /// fixture's minimal dialect never sent them — the real harness does
    /// (found on the first real boot, 2026-07-04).
    Region,
    Ready {
        frame: u64,
    },
    Fault {
        detail: String,
    },
}

pub(crate) fn socketpair() -> io::Result<(ControlSocket, OwnedFd)> {
    let mut fds = [0i32; 2];
    let rc = unsafe {
        libc::socketpair(
            libc::AF_UNIX,
            libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC,
            0,
            fds.as_mut_ptr(),
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    let parent = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    let child = unsafe { OwnedFd::from_raw_fd(fds[1]) };
    Ok((ControlSocket { fd: parent }, child))
}

pub(crate) fn child_fd_number() -> i32 {
    CONTROL_FD
}

/// Drive the boot-time control leg. `game_path` is the LoadGame.dev_path to
/// send — resolved by the caller (verbatim `game_dev`, or the materialized
/// image path under `game_source = "pv-blk"`; API.md §7.1). `idle` runs on
/// every empty poll of the control socket — the workload registers regions
/// over agent.sock *between* these replies (e.g. after GameLoaded, before
/// Ready), so the caller passes a region-IPC servicing closure here; a
/// blocking recv would deadlock the boot (ARCHITECTURE.md §5).
pub(crate) fn drive_refwork_start(
    sock: &ControlSocket,
    control: &UnitControl,
    game_path: &str,
    mut idle: impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
    if control.protocol != "refwork-ctl" {
        return Err(format!(
            "unsupported unit.control protocol {:?}",
            control.protocol
        ));
    }
    let proto_version = u16::try_from(control.proto_version).map_err(|_| {
        format!(
            "refwork proto_version {} exceeds u16",
            control.proto_version
        )
    })?;

    sock.send(&encode_hello(proto_version))
        .map_err(|e| format!("send refwork Hello: {e}"))?;
    match sock
        .recv(&mut idle)
        .map_err(|e| format!("recv refwork HelloAck: {e}"))?
    {
        ControlReply::HelloAck {
            proto_version: reply_version,
        } if reply_version == proto_version => {}
        ControlReply::HelloAck {
            proto_version: reply_version,
        } => {
            return Err(format!(
                "refwork HelloAck proto_version {reply_version} != {proto_version}"
            ));
        }
        ControlReply::Fault { detail } => {
            return Err(format!("refwork fault after Hello: {detail}"))
        }
        other => return Err(format!("expected refwork HelloAck, got {other:?}")),
    }

    sock.send(&encode_load_game(game_path))
        .map_err(|e| format!("send refwork LoadGame: {e}"))?;
    loop {
        match sock
            .recv(&mut idle)
            .map_err(|e| format!("recv refwork GameLoaded: {e}"))?
        {
            ControlReply::GameLoaded => break,
            ControlReply::Region => continue,
            ControlReply::Fault { detail } => {
                return Err(format!("refwork fault after LoadGame: {detail}"));
            }
            other => return Err(format!("expected refwork GameLoaded, got {other:?}")),
        }
    }

    loop {
        match sock
            .recv(&mut idle)
            .map_err(|e| format!("recv refwork Ready: {e}"))?
        {
            ControlReply::Ready { frame: 0 } => break,
            ControlReply::Region => continue,
            ControlReply::Ready { frame } => {
                return Err(format!("refwork Ready frame must be 0, got {frame}"));
            }
            ControlReply::Fault { detail } => {
                return Err(format!("refwork fault before Start: {detail}"));
            }
            other => return Err(format!("expected refwork Ready, got {other:?}")),
        }
    }

    sock.send(&[0x02])
        .map_err(|e| format!("send refwork Start: {e}"))?;
    Ok(())
}

impl ControlSocket {
    fn send(&self, bytes: &[u8]) -> io::Result<()> {
        let rc = unsafe {
            libc::send(
                self.fd.as_raw_fd(),
                bytes.as_ptr().cast(),
                bytes.len(),
                libc::MSG_NOSIGNAL,
            )
        };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        if rc as usize != bytes.len() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "short control datagram write",
            ));
        }
        Ok(())
    }

    /// Bounded non-blocking reply wait: poll with `MSG_DONTWAIT`, running
    /// `idle` (region-IPC servicing) + `sched_yield` between polls so a
    /// workload blocked on a register reply cannot deadlock this wait.
    fn recv(&self, idle: &mut impl FnMut() -> Result<(), String>) -> io::Result<ControlReply> {
        let mut buf = [0u8; MAX_DATAGRAM];
        for _ in 0..CONTROL_RECV_POLL_LIMIT {
            // SAFETY: non-blocking recv into a local buffer.
            let n = unsafe {
                libc::recv(
                    self.fd.as_raw_fd(),
                    buf.as_mut_ptr().cast(),
                    buf.len(),
                    libc::MSG_DONTWAIT,
                )
            };
            if n < 0 {
                let err = io::Error::last_os_error();
                if err.kind() != io::ErrorKind::WouldBlock {
                    return Err(err);
                }
                idle().map_err(io::Error::other)?;
                // SAFETY: plain sched_yield between polls.
                unsafe {
                    libc::sched_yield();
                }
                continue;
            }
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "control socket closed",
                ));
            }
            return decode_reply(&buf[..n as usize])
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e));
        }
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "control reply poll limit exceeded (workload dead or wedged)",
        ))
    }
}

fn encode_hello(proto_version: u16) -> Vec<u8> {
    let mut out = vec![0x00];
    push_varint(&mut out, proto_version as u64);
    out
}

fn encode_load_game(dev_path: &str) -> Vec<u8> {
    let mut out = vec![0x01];
    push_bytes(&mut out, dev_path.as_bytes());
    out
}

fn push_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    push_varint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

fn push_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn decode_reply(bytes: &[u8]) -> Result<ControlReply, String> {
    let mut cur = Cursor { bytes, at: 0 };
    let tag = cur.varint()?;
    let reply = match tag {
        5 => {
            let proto_version = u16::try_from(cur.varint()?)
                .map_err(|_| "HelloAck proto_version overflow".to_string())?;
            let _emu = cur.string()?;
            let _emu_version = cur.string()?;
            ControlReply::HelloAck { proto_version }
        }
        6 => {
            cur.take(32)?;
            let _mapper = cur.string()?;
            let _sram_size = cur.varint()?;
            ControlReply::GameLoaded
        }
        // Payload deliberately not parsed — but consumed, to satisfy the
        // trailing-bytes check: one message per datagram and the agent
        // ignores region announcements (see ControlReply::Region).
        7 => {
            cur.at = cur.bytes.len();
            ControlReply::Region
        }
        8 => ControlReply::Ready {
            frame: cur.varint()?,
        },
        10 => {
            let _frame = cur.varint()?;
            let _code = cur.varint()?;
            let detail = cur.string()?;
            ControlReply::Fault { detail }
        }
        other => return Err(format!("unknown refwork control reply tag {other}")),
    };
    if cur.at != bytes.len() {
        return Err("trailing bytes in refwork control reply".into());
    }
    Ok(reply)
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self
            .at
            .checked_add(n)
            .ok_or_else(|| "control datagram offset overflow".to_string())?;
        if end > self.bytes.len() {
            return Err("truncated refwork control datagram".into());
        }
        let out = &self.bytes[self.at..end];
        self.at = end;
        Ok(out)
    }

    fn string(&mut self) -> Result<String, String> {
        let len =
            usize::try_from(self.varint()?).map_err(|_| "string length overflow".to_string())?;
        let bytes = self.take(len)?;
        std::str::from_utf8(bytes)
            .map(|s| s.to_string())
            .map_err(|e| format!("control string is not UTF-8: {e}"))
    }

    fn varint(&mut self) -> Result<u64, String> {
        let mut value = 0u64;
        let mut shift = 0;
        loop {
            let byte = *self
                .take(1)?
                .first()
                .ok_or_else(|| "truncated varint".to_string())?;
            value |= ((byte & 0x7F) as u64)
                .checked_shl(shift)
                .ok_or_else(|| "control varint overflow".to_string())?;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
            shift += 7;
            if shift >= 64 {
                return Err("control varint overflow".into());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_request_golden_bytes_match_refwork_protocol() {
        assert_eq!(encode_hello(1), [0x00, 0x01]);
        assert_eq!(
            encode_load_game("/dev/vdb"),
            [0x01, 0x08, b'/', b'd', b'e', b'v', b'/', b'v', b'd', b'b']
        );
    }

    #[test]
    fn load_game_golden_for_the_materialized_path() {
        // The exact bytes a workload sees under game_source = "pv-blk"
        // (API.md §7.1): tag 0x01, varint length, then the path.
        let path = crate::pvblk::GAME_IMG_PATH;
        let mut want = vec![0x01, path.len() as u8];
        want.extend_from_slice(path.as_bytes());
        assert_eq!(encode_load_game(path), want);
        assert_eq!(path.len(), 22); // varint stays single-byte
    }

    #[test]
    fn decodes_required_harness_replies() {
        assert_eq!(
            decode_reply(&[0x05, 0x01, 0x03, b'e', b'm', b'u', 0x01, b'1']).unwrap(),
            ControlReply::HelloAck { proto_version: 1 }
        );

        let mut game_loaded = vec![0x06];
        game_loaded.extend_from_slice(&[0u8; 32]);
        game_loaded.extend_from_slice(&[0x04, b'm', b'm', b'c', b'3', 0x00]);
        assert_eq!(
            decode_reply(&game_loaded).unwrap(),
            ControlReply::GameLoaded
        );

        assert_eq!(
            decode_reply(&[0x08, 0x00]).unwrap(),
            ControlReply::Ready { frame: 0 }
        );
        // Tag 7 (RegisterRegion) is informational and skipped: any payload
        // decodes to Region without being parsed.
        assert_eq!(
            decode_reply(&[0x07, 0x04, b'w', b'r', b'a', b'm', 0x00]).unwrap(),
            ControlReply::Region
        );
    }
}
