//! SDK side of the region-registration IPC (`/run/detguest/agent.sock`,
//! API.md §1.5): one cached AF_UNIX SOCK_SEQPACKET connection, strict
//! send-one-recv-one, no timeouts (determinism — a hung agent means a hung
//! workload, which the supervise tier owns).
//!
//! Permitted-unsafe module: libc socket plumbing (std does not expose
//! SOCK_SEQPACKET).
#![allow(unsafe_code)]

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::Mutex;

use detguest_wire::regionipc::{
    self, Reply, Request, AGENT_SOCK_PATH, REGIONIPC_MAX_DATAGRAM, STATUS_MANIFEST_FULL,
    STATUS_NAME_TOO_LONG, STATUS_NOT_PINNED, STATUS_OK, STATUS_TOO_MANY_EXTENTS,
};

use crate::regions::RegionError;

/// Test/debug override for the socket path; production initramfs sets no
/// environment beyond `DETGUEST_CHANNEL_FD`, so the default always applies
/// in-guest. Read once per connect attempt.
const SOCK_PATH_ENV: &str = "DETGUEST_AGENT_SOCK";

static CLIENT: Mutex<Option<AgentClient>> = Mutex::new(None);

/// Serializes tests that touch the process-global client/env override.
#[cfg(test)]
pub(crate) static TEST_SERIAL: Mutex<()> = Mutex::new(());

/// Drop the cached connection so a test server observes EOF.
#[cfg(test)]
pub(crate) fn drop_cached_client_for_test() {
    *CLIENT.lock().unwrap() = None;
}

#[derive(Debug)]
struct AgentClient {
    fd: OwnedFd,
}

impl AgentClient {
    fn connect() -> Result<AgentClient, RegionError> {
        let path = std::env::var(SOCK_PATH_ENV).unwrap_or_else(|_| AGENT_SOCK_PATH.to_string());
        // SAFETY: plain socket(2) + connect(2) with a valid sockaddr_un.
        unsafe {
            let raw = libc::socket(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0);
            if raw < 0 {
                return Err(RegionError::AgentUnavailable);
            }
            let fd = OwnedFd::from_raw_fd(raw);
            let mut addr: libc::sockaddr_un = std::mem::zeroed();
            addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
            let bytes = path.as_bytes();
            if bytes.len() >= addr.sun_path.len() {
                return Err(RegionError::AgentUnavailable);
            }
            for (dst, src) in addr.sun_path.iter_mut().zip(bytes) {
                *dst = *src as libc::c_char;
            }
            let len = (std::mem::offset_of!(libc::sockaddr_un, sun_path) + bytes.len() + 1)
                as libc::socklen_t;
            if libc::connect(
                fd.as_raw_fd(),
                &addr as *const libc::sockaddr_un as *const libc::sockaddr,
                len,
            ) != 0
            {
                return Err(RegionError::AgentUnavailable);
            }
            Ok(AgentClient { fd })
        }
    }

    /// Blocking request/reply. Any transport failure is `AgentUnavailable`.
    fn call(&self, req: &Request<'_>) -> Result<Reply, RegionError> {
        let mut buf = [0u8; REGIONIPC_MAX_DATAGRAM];
        let n = regionipc::encode_request(req, &mut buf).map_err(|_| RegionError::NameTooLong)?;
        // SAFETY: blocking send of one datagram on our connected socket.
        let sent = unsafe {
            libc::send(
                self.fd.as_raw_fd(),
                buf.as_ptr().cast(),
                n,
                libc::MSG_NOSIGNAL,
            )
        };
        if sent != n as isize {
            return Err(RegionError::AgentUnavailable);
        }
        let mut reply = [0u8; REGIONIPC_MAX_DATAGRAM];
        // SAFETY: blocking recv of one datagram into a local buffer.
        let got = unsafe {
            libc::recv(
                self.fd.as_raw_fd(),
                reply.as_mut_ptr().cast(),
                reply.len(),
                0,
            )
        };
        if got <= 0 {
            return Err(RegionError::AgentUnavailable);
        }
        regionipc::decode_reply(&reply[..got as usize]).map_err(|_| RegionError::AgentUnavailable)
    }
}

/// Run `req` against the cached agent connection, connecting lazily. A
/// failed connect is not cached (retried next call); a transport error
/// drops the cached connection so the next call reconnects.
pub(crate) fn call(req: &Request<'_>) -> Result<Reply, RegionError> {
    let mut slot = CLIENT.lock().map_err(|_| RegionError::AgentUnavailable)?;
    if slot.is_none() {
        *slot = Some(AgentClient::connect()?);
    }
    let client = slot.as_ref().expect("connected above");
    match client.call(req) {
        Ok(reply) => Ok(reply),
        Err(e) => {
            *slot = None;
            Err(e)
        }
    }
}

/// Map a non-OK reply status onto the public error type (deterministic
/// mapping per API.md §1.5).
pub(crate) fn status_to_error(status: u16) -> RegionError {
    debug_assert_ne!(status, STATUS_OK);
    match status {
        STATUS_MANIFEST_FULL => RegionError::ManifestFull,
        STATUS_TOO_MANY_EXTENTS => RegionError::TooManyExtents,
        STATUS_NOT_PINNED => RegionError::NotPinned,
        STATUS_NAME_TOO_LONG => RegionError::NameTooLong,
        // BAD_REQUEST / UNKNOWN_PID / UNKNOWN_REGION / INTERNAL: the agent
        // path is present but unusable from here.
        _ => RegionError::AgentUnavailable,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use detguest_wire::regionipc::STATUS_UNKNOWN_PID;

    /// Minimal in-process SEQPACKET server: binds `path`, accepts one
    /// connection, answers every request with `reply`. Returns the join
    /// handle; the server exits on peer EOF.
    pub(crate) fn spawn_test_server(
        path: &str,
        reply: Reply,
    ) -> std::thread::JoinHandle<Vec<Vec<u8>>> {
        let path = path.to_string();
        // SAFETY: server-side socket/bind/listen with a valid sockaddr_un.
        let listener = unsafe {
            let raw = libc::socket(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0);
            assert!(raw >= 0);
            let fd = OwnedFd::from_raw_fd(raw);
            let mut addr: libc::sockaddr_un = std::mem::zeroed();
            addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
            for (dst, src) in addr.sun_path.iter_mut().zip(path.as_bytes()) {
                *dst = *src as libc::c_char;
            }
            let len = (std::mem::offset_of!(libc::sockaddr_un, sun_path) + path.len() + 1)
                as libc::socklen_t;
            let _ = std::fs::remove_file(&path);
            assert_eq!(
                libc::bind(
                    fd.as_raw_fd(),
                    &addr as *const libc::sockaddr_un as *const libc::sockaddr,
                    len,
                ),
                0,
                "bind {path}: {}",
                std::io::Error::last_os_error()
            );
            assert_eq!(libc::listen(fd.as_raw_fd(), 1), 0);
            fd
        };
        std::thread::spawn(move || {
            let mut seen = Vec::new();
            // SAFETY: blocking accept + per-datagram recv/send loop.
            unsafe {
                let conn = libc::accept(
                    listener.as_raw_fd(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                );
                assert!(conn >= 0);
                let conn = OwnedFd::from_raw_fd(conn);
                loop {
                    let mut buf = [0u8; REGIONIPC_MAX_DATAGRAM];
                    let n = libc::recv(conn.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), 0);
                    if n <= 0 {
                        break;
                    }
                    seen.push(buf[..n as usize].to_vec());
                    let mut out = [0u8; REGIONIPC_MAX_DATAGRAM];
                    let len = regionipc::encode_reply(&reply, &mut out).unwrap();
                    libc::send(
                        conn.as_raw_fd(),
                        out.as_ptr().cast(),
                        len,
                        libc::MSG_NOSIGNAL,
                    );
                }
            }
            seen
        })
    }

    #[test]
    fn call_round_trips_through_a_real_socket() {
        let _serial = TEST_SERIAL.lock().unwrap();
        let path = format!("/tmp/detguest-sdk-client-test-{}.sock", std::process::id());
        let reply = Reply {
            status: STATUS_OK,
            region_id: 4,
            name_id: 9,
            manifest_generation: 6,
        };
        let server = spawn_test_server(&path, reply);
        std::env::set_var(SOCK_PATH_ENV, &path);
        let got = call(&Request::Register {
            flags: 0,
            layout_version: 1,
            name_id: 9,
            gva: 0x1000,
            len: 4096,
            name: b"wram",
        })
        .unwrap();
        assert_eq!(got, reply);
        // Second call reuses the cached connection.
        let got = call(&Request::Unregister { region_id: 4 }).unwrap();
        assert_eq!(got, reply);
        // Drop the cached client so the server thread sees EOF.
        *CLIENT.lock().unwrap() = None;
        std::env::remove_var(SOCK_PATH_ENV);
        let seen = server.join().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(matches!(
            regionipc::decode_request(&seen[0]).unwrap(),
            Request::Register { name_id: 9, .. }
        ));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn connect_failure_is_agent_unavailable_and_not_cached() {
        let _serial = TEST_SERIAL.lock().unwrap();
        // No server bound at the default path on a dev host.
        assert_eq!(
            AgentClient::connect().map(|_| ()).unwrap_err(),
            RegionError::AgentUnavailable
        );
    }

    #[test]
    fn status_mapping_is_deterministic() {
        assert_eq!(
            status_to_error(STATUS_MANIFEST_FULL),
            RegionError::ManifestFull
        );
        assert_eq!(
            status_to_error(STATUS_TOO_MANY_EXTENTS),
            RegionError::TooManyExtents
        );
        assert_eq!(status_to_error(STATUS_NOT_PINNED), RegionError::NotPinned);
        assert_eq!(
            status_to_error(STATUS_NAME_TOO_LONG),
            RegionError::NameTooLong
        );
        assert_eq!(
            status_to_error(STATUS_UNKNOWN_PID),
            RegionError::AgentUnavailable
        );
    }
}
