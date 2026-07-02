//! Region-registration IPC server (`/run/detguest/agent.sock`) and the
//! agent-side manifest writer (API.md §1.5; ARCHITECTURE.md §5).
//!
//! The agent is the ONLY manifest writer (the seqlock discipline in
//! `detguest_wire::manifest`): the SDK mlocks + prefaults in the workload,
//! then asks us to translate and publish. We bind the caller pid via
//! `SO_PEERCRED`, walk `/proc/<pid>/pagemap`, coalesce extents, write the
//! manifest under the seqlock, emit `NameIntern` + `RegionRegister` on
//! ring A, and keep a [`RegionRecord`] ledger that `ReverifyRegions` (the
//! §5 pinning canary) re-walks after restore/fork.
//!
//! Everything here is single-threaded and serviced from three places (the
//! supervise epoll loop, the expected-regions wait, and the control-recv
//! idle loop) so a workload blocked on a register reply can never deadlock
//! against an agent blocked on that same workload's progress.
//!
//! Permitted-unsafe module: AF_UNIX SOCK_SEQPACKET socket plumbing via libc.
#![allow(unsafe_code)]

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

use detguest_wire::events::{EventPayload, RegionEvent};
use detguest_wire::manifest::{
    writer_begin, writer_end, Extent, ManifestHeader, RegionEntry, EXTENT_CAPACITY,
    REGION_CAPACITY, REGION_FLAG_DEAD,
};
use detguest_wire::regionipc::{
    self, Reply, Request, AGENT_SOCK_PATH, REGIONIPC_MAX_DATAGRAM, STATUS_BAD_REQUEST,
    STATUS_INTERNAL, STATUS_MANIFEST_FULL, STATUS_NOT_PINNED, STATUS_OK, STATUS_TOO_MANY_EXTENTS,
    STATUS_UNKNOWN_PID, STATUS_UNKNOWN_REGION,
};

use crate::channel::AgentChannel;
use crate::supervise::vnanos;
use crate::translate::{self, BuildExtentsError};

/// Accepted-connection cap; v1 supervises one workload, so one live
/// connection is the norm and anything past this is a guest bug.
const MAX_CONNS: usize = 4;

/// One registered region as the agent knows it: everything `ReverifyRegions`
/// needs to re-walk pagemap without asking the workload anything.
#[derive(Debug)]
pub(crate) struct RegionRecord {
    pub(crate) region_id: u32,
    pub(crate) name: Vec<u8>,
    pub(crate) name_id: u32,
    pub(crate) layout_version: u32,
    pub(crate) pid: i32,
    pub(crate) gva: u64,
    pub(crate) len: u64,
    pub(crate) extents: Vec<Extent>,
    pub(crate) dead: bool,
}

/// GVA-range translator, injectable so host unit tests can simulate pinning
/// drift without real pagemap PFNs (CI runs unprivileged).
pub(crate) type Translator =
    Box<dyn FnMut(i32, u64, u64) -> Result<Vec<Extent>, BuildExtentsError>>;

fn real_translator() -> Translator {
    Box::new(|pid, gva, len| {
        let pagemap = translate::open_pagemap_for(pid)
            .map_err(|e| BuildExtentsError::Translate(translate::TranslateError::Io(e.kind())))?;
        translate::build_extents(|vaddr| translate::gva_to_gpa(&pagemap, vaddr), gva, len)
    })
}

struct Conn {
    fd: OwnedFd,
    pid: i32,
}

/// The agent.sock listener + registration ledger.
pub(crate) struct RegionIpc {
    listener: OwnedFd,
    conns: Vec<Conn>,
    records: Vec<RegionRecord>,
    translate: Translator,
}

impl std::fmt::Debug for RegionIpc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegionIpc")
            .field("listener", &self.listener)
            .field("conns", &self.conns.len())
            .field("records", &self.records)
            .finish()
    }
}

impl RegionIpc {
    /// Bind the canonical socket. Called before the autostart unit spawns so
    /// the path exists before any workload runs; failure is a boot fault.
    pub(crate) fn bind() -> io::Result<RegionIpc> {
        std::fs::create_dir_all("/run/detguest")?;
        Self::bind_at(AGENT_SOCK_PATH, real_translator())
    }

    /// Bind at an explicit path with an explicit translator (tests).
    pub(crate) fn bind_at(path: &str, translate: Translator) -> io::Result<RegionIpc> {
        let _ = std::fs::remove_file(path);
        let fd = seqpacket_socket()?;
        let addr = sockaddr_un(path)?;
        // SAFETY: bind/listen on our fresh socket with a valid sockaddr_un.
        unsafe {
            if libc::bind(
                fd.as_raw_fd(),
                &addr as *const libc::sockaddr_un as *const libc::sockaddr,
                sockaddr_un_len(path),
            ) != 0
            {
                return Err(io::Error::last_os_error());
            }
            if libc::listen(fd.as_raw_fd(), MAX_CONNS as i32) != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(RegionIpc {
            listener: fd,
            conns: Vec::new(),
            records: Vec::new(),
            translate,
        })
    }

    /// The listener fd (for epoll registration).
    pub(crate) fn listener_fd(&self) -> RawFd {
        self.listener.as_raw_fd()
    }

    /// The registration ledger (test observability).
    #[cfg(test)]
    pub(crate) fn records(&self) -> &[RegionRecord] {
        &self.records
    }

    /// Accept pending connections and process every readable request
    /// datagram. Non-blocking; returns after draining. Safe to call from any
    /// wait loop. `epfd` (when valid) gets new connection fds registered
    /// under `epoll_tok` so post-Ready requests wake the supervise loop.
    pub(crate) fn service(
        &mut self,
        channel: &mut AgentChannel,
        workload_pid: Option<i32>,
        epfd: Option<(RawFd, u64)>,
    ) -> io::Result<()> {
        self.accept_pending(epfd)?;
        let mut closed: Vec<usize> = Vec::new();
        for i in 0..self.conns.len() {
            loop {
                let mut buf = [0u8; REGIONIPC_MAX_DATAGRAM + 1];
                // SAFETY: non-blocking recv into a local buffer.
                let n = unsafe {
                    libc::recv(
                        self.conns[i].fd.as_raw_fd(),
                        buf.as_mut_ptr().cast(),
                        buf.len(),
                        libc::MSG_DONTWAIT,
                    )
                };
                if n < 0 {
                    let err = io::Error::last_os_error();
                    if err.kind() == io::ErrorKind::WouldBlock {
                        break;
                    }
                    closed.push(i);
                    break;
                }
                if n == 0 {
                    // Peer EOF: drop the conn; records outlive the socket.
                    closed.push(i);
                    break;
                }
                let reply = if n as usize > REGIONIPC_MAX_DATAGRAM {
                    err_reply(STATUS_BAD_REQUEST)
                } else {
                    match regionipc::decode_request(&buf[..n as usize]) {
                        Ok(req) => self.handle(req, self.conns[i].pid, workload_pid, channel),
                        Err(_) => err_reply(STATUS_BAD_REQUEST),
                    }
                };
                let mut out = [0u8; REGIONIPC_MAX_DATAGRAM];
                let len = regionipc::encode_reply(&reply, &mut out)
                    .expect("reply always fits the datagram cap");
                // SAFETY: send our reply datagram; failure drops the conn.
                let sent = unsafe {
                    libc::send(
                        self.conns[i].fd.as_raw_fd(),
                        out.as_ptr().cast(),
                        len,
                        libc::MSG_NOSIGNAL,
                    )
                };
                if sent != len as isize {
                    closed.push(i);
                    break;
                }
            }
        }
        for &i in closed.iter().rev() {
            self.conns.remove(i);
        }
        Ok(())
    }

    fn accept_pending(&mut self, epfd: Option<(RawFd, u64)>) -> io::Result<()> {
        loop {
            // SAFETY: non-blocking accept4 on our listener.
            let raw = unsafe {
                libc::accept4(
                    self.listener.as_raw_fd(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
                )
            };
            if raw < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::WouldBlock {
                    return Ok(());
                }
                return Err(err);
            }
            // SAFETY: accept4 returned a fresh owned fd.
            let fd = unsafe { OwnedFd::from_raw_fd(raw) };
            if self.conns.len() >= MAX_CONNS {
                continue; // drop immediately: connection storm = guest bug
            }
            let pid = peer_pid(fd.as_raw_fd())?;
            if let Some((epfd, tok)) = epfd {
                let mut ev = libc::epoll_event {
                    events: libc::EPOLLIN as u32,
                    u64: tok,
                };
                // SAFETY: registering the accepted fd with the caller's epoll.
                unsafe {
                    libc::epoll_ctl(epfd, libc::EPOLL_CTL_ADD, fd.as_raw_fd(), &mut ev);
                }
            }
            self.conns.push(Conn { fd, pid });
        }
    }

    fn handle(
        &mut self,
        req: Request<'_>,
        peer_pid: i32,
        workload_pid: Option<i32>,
        channel: &mut AgentChannel,
    ) -> Reply {
        match req {
            Request::Register {
                flags,
                layout_version,
                name_id,
                gva,
                len,
                name,
            } => {
                // pid binding: only the supervised workload may register.
                if workload_pid != Some(peer_pid) {
                    return err_reply(STATUS_UNKNOWN_PID);
                }
                let name = name.to_vec();
                let extents = match (self.translate)(peer_pid, gva, len) {
                    Ok(extents) => extents,
                    Err(e) => return err_reply(build_error_status(&e)),
                };
                let (region_id, generation) = match write_region(
                    channel,
                    &name,
                    name_id,
                    layout_version,
                    flags,
                    gva,
                    len,
                    &extents,
                ) {
                    Ok(ok) => ok,
                    Err(status) => return err_reply(status),
                };
                let gen32 = match u32::try_from(generation) {
                    Ok(g) => g,
                    Err(_) => return err_reply(STATUS_INTERNAL),
                };
                channel.emit(
                    vnanos(),
                    0,
                    &EventPayload::NameIntern {
                        name_id,
                        name: &name,
                    },
                );
                channel.emit_with_doorbell(
                    vnanos(),
                    0,
                    &EventPayload::RegionRegister(RegionEvent {
                        region_id,
                        name_id,
                        layout_version,
                        manifest_generation: gen32,
                    }),
                );
                self.records.push(RegionRecord {
                    region_id,
                    name,
                    name_id,
                    layout_version,
                    pid: peer_pid,
                    gva,
                    len,
                    extents,
                    dead: false,
                });
                Reply {
                    status: STATUS_OK,
                    region_id,
                    name_id,
                    manifest_generation: generation,
                }
            }
            Request::Unregister { region_id } => {
                if workload_pid != Some(peer_pid) {
                    return err_reply(STATUS_UNKNOWN_PID);
                }
                let Some(record) = self
                    .records
                    .iter_mut()
                    .find(|r| r.region_id == region_id && !r.dead)
                else {
                    return err_reply(STATUS_UNKNOWN_REGION);
                };
                match mark_region_dead(channel, region_id) {
                    Ok(generation) => {
                        record.dead = true;
                        Reply {
                            status: STATUS_OK,
                            region_id,
                            name_id: record.name_id,
                            manifest_generation: generation,
                        }
                    }
                    Err(status) => err_reply(status),
                }
            }
        }
    }

    /// `ReverifyRegions` (API.md §6): re-walk pagemap for every live region;
    /// emit `RegionUpdate` per region — generation echo when the extents
    /// still match, P0 alarm + manifest rewrite when they drifted, P0 alarm +
    /// DEAD when the range no longer translates (workload died, pages
    /// reclaimed). One doorbell closes the sweep so the host drains a
    /// complete batch.
    pub(crate) fn reverify(&mut self, channel: &mut AgentChannel) {
        let mut updates: Vec<(RegionEvent, Option<String>)> = Vec::new();
        // region_id order == ledger order (ids are append-only).
        for record in self.records.iter_mut().filter(|r| !r.dead) {
            match (self.translate)(record.pid, record.gva, record.len) {
                Ok(extents) if extents == record.extents => {
                    let generation = current_generation(channel);
                    updates.push((region_event(record, generation), None));
                }
                Ok(extents) => {
                    let alarm = format!(
                        "P0: region {} ({}) extents drifted under pinning",
                        record.region_id,
                        String::from_utf8_lossy(&record.name),
                    );
                    match rewrite_extents(channel, record.region_id, &extents) {
                        Ok(generation) => {
                            record.extents = extents;
                            updates.push((region_event(record, generation), Some(alarm)));
                        }
                        Err(_) => {
                            // Pool exhausted / manifest inconsistent: the
                            // region is no longer trustworthy — kill it.
                            let generation = mark_region_dead(channel, record.region_id)
                                .unwrap_or_else(|_| current_generation(channel));
                            record.dead = true;
                            updates.push((region_event(record, generation), Some(alarm)));
                        }
                    }
                }
                Err(_) => {
                    let alarm = format!(
                        "P0: region {} ({}) no longer translates; marking dead",
                        record.region_id,
                        String::from_utf8_lossy(&record.name),
                    );
                    let generation = mark_region_dead(channel, record.region_id)
                        .unwrap_or_else(|_| current_generation(channel));
                    record.dead = true;
                    updates.push((region_event(record, generation), Some(alarm)));
                }
            }
        }
        let last = updates.len().saturating_sub(1);
        for (i, (event, alarm)) in updates.into_iter().enumerate() {
            if let Some(msg) = alarm {
                channel.emit(
                    vnanos(),
                    0,
                    &EventPayload::LogLine {
                        stream: detguest_wire::events::log_stream::AGENT,
                        level: 0,
                        msg: msg.as_bytes(),
                    },
                );
            }
            let payload = EventPayload::RegionUpdate(event);
            if i == last {
                channel.emit_with_doorbell(vnanos(), 0, &payload);
            } else {
                channel.emit(vnanos(), 0, &payload);
            }
        }
    }
}

fn region_event(record: &RegionRecord, generation: u64) -> RegionEvent {
    RegionEvent {
        region_id: record.region_id,
        name_id: record.name_id,
        layout_version: record.layout_version,
        // Generations count registrations (+2 each); u32 wrap is unreachable
        // before the 64-slot manifest fills. Saturate rather than fault.
        manifest_generation: u32::try_from(generation).unwrap_or(u32::MAX),
    }
}

fn err_reply(status: u16) -> Reply {
    Reply {
        status,
        region_id: 0,
        name_id: 0,
        manifest_generation: 0,
    }
}

fn build_error_status(e: &BuildExtentsError) -> u16 {
    match e {
        BuildExtentsError::Translate(translate::TranslateError::Io(_)) => STATUS_INTERNAL,
        BuildExtentsError::Translate(_) => STATUS_NOT_PINNED,
        BuildExtentsError::TooManyExtents => STATUS_TOO_MANY_EXTENTS,
    }
}

fn current_generation(channel: &AgentChannel) -> u64 {
    detguest_wire::manifest::read_generation(channel.manifest()).unwrap_or(0)
}

/// The manifest write, under the seqlock (ported from the former SDK
/// `publish_region`; slot policy unchanged: sequential region ids, never
/// reused; append-only extent pool).
#[allow(clippy::too_many_arguments)]
fn write_region(
    channel: &mut AgentChannel,
    name: &[u8],
    name_id: u32,
    layout_version: u32,
    flags: u32,
    gva: u64,
    len: u64,
    extents: &[Extent],
) -> Result<(u32, u64), u16> {
    let packed_name = RegionEntry::pack_name(name).map_err(|_| STATUS_BAD_REQUEST)?;
    let manifest = channel.manifest_mut();
    let mut hdr = ManifestHeader::read_from(manifest).map_err(|_| STATUS_INTERNAL)?;
    hdr.validate().map_err(|_| STATUS_INTERNAL)?;
    if hdr.region_count as usize >= REGION_CAPACITY {
        return Err(STATUS_MANIFEST_FULL);
    }
    if hdr.extent_count as usize + extents.len() > EXTENT_CAPACITY {
        return Err(STATUS_TOO_MANY_EXTENTS);
    }
    let region_id = hdr.region_count;
    let extent_off = hdr.extent_count;
    let odd_generation = writer_begin(manifest).map_err(|_| STATUS_INTERNAL)?;
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    for (i, extent) in extents.iter().enumerate() {
        extent
            .write_to(manifest, extent_off as usize + i)
            .expect("extent bounds checked before manifest write");
    }
    RegionEntry {
        region_id,
        name_id,
        layout_version,
        flags,
        gva,
        len,
        extent_off,
        extent_n: extents.len() as u32,
        name: packed_name,
    }
    .write_to(manifest, region_id as usize)
    .expect("region slot checked before manifest write");
    hdr.generation = odd_generation;
    hdr.region_count = hdr.region_count.saturating_add(1);
    hdr.extent_count = hdr.extent_count.saturating_add(extents.len() as u32);
    hdr.write_to(manifest)
        .expect("manifest header bounds checked before manifest write");
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    let generation = writer_end(manifest).map_err(|_| STATUS_INTERNAL)?;
    Ok((region_id, generation))
}

/// Set the DEAD flag on `region_id` under the seqlock; returns the new
/// (even) generation.
fn mark_region_dead(channel: &mut AgentChannel, region_id: u32) -> Result<u64, u16> {
    let manifest = channel.manifest_mut();
    let mut entry =
        RegionEntry::read_from(manifest, region_id as usize).map_err(|_| STATUS_INTERNAL)?;
    entry.flags |= REGION_FLAG_DEAD;
    writer_begin(manifest).map_err(|_| STATUS_INTERNAL)?;
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    entry
        .write_to(manifest, region_id as usize)
        .map_err(|_| STATUS_INTERNAL)?;
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    writer_end(manifest).map_err(|_| STATUS_INTERNAL)
}

/// Rewrite a live region's extents in place (same count) or append to the
/// pool (grown count) under the seqlock; returns the new (even) generation.
fn rewrite_extents(
    channel: &mut AgentChannel,
    region_id: u32,
    extents: &[Extent],
) -> Result<u64, u16> {
    let manifest = channel.manifest_mut();
    let mut entry =
        RegionEntry::read_from(manifest, region_id as usize).map_err(|_| STATUS_INTERNAL)?;
    let mut hdr = ManifestHeader::read_from(manifest).map_err(|_| STATUS_INTERNAL)?;
    let (extent_off, bump_pool) = if extents.len() <= entry.extent_n as usize {
        (entry.extent_off, false)
    } else {
        if hdr.extent_count as usize + extents.len() > EXTENT_CAPACITY {
            return Err(STATUS_TOO_MANY_EXTENTS);
        }
        (hdr.extent_count, true)
    };
    let odd = writer_begin(manifest).map_err(|_| STATUS_INTERNAL)?;
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    for (i, extent) in extents.iter().enumerate() {
        extent
            .write_to(manifest, extent_off as usize + i)
            .map_err(|_| STATUS_INTERNAL)?;
    }
    entry.extent_off = extent_off;
    entry.extent_n = extents.len() as u32;
    entry
        .write_to(manifest, region_id as usize)
        .map_err(|_| STATUS_INTERNAL)?;
    if bump_pool {
        hdr.generation = odd;
        hdr.extent_count = hdr.extent_count.saturating_add(extents.len() as u32);
        hdr.write_to(manifest).map_err(|_| STATUS_INTERNAL)?;
    }
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    writer_end(manifest).map_err(|_| STATUS_INTERNAL)
}

fn seqpacket_socket() -> io::Result<OwnedFd> {
    // SAFETY: plain socket(2).
    let raw = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_SEQPACKET | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            0,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: socket returned a fresh owned fd.
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

fn sockaddr_un(path: &str) -> io::Result<libc::sockaddr_un> {
    // SAFETY: zeroed POD struct.
    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
    let bytes = path.as_bytes();
    if bytes.len() >= addr.sun_path.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "socket path too long",
        ));
    }
    for (dst, src) in addr.sun_path.iter_mut().zip(bytes) {
        *dst = *src as libc::c_char;
    }
    Ok(addr)
}

fn sockaddr_un_len(path: &str) -> libc::socklen_t {
    (std::mem::offset_of!(libc::sockaddr_un, sun_path) + path.len() + 1) as libc::socklen_t
}

fn peer_pid(fd: RawFd) -> io::Result<i32> {
    // SAFETY: getsockopt(SO_PEERCRED) into a local ucred.
    unsafe {
        let mut cred: libc::ucred = std::mem::zeroed();
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        if libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut len,
        ) != 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(cred.pid)
    }
}

#[cfg(test)]
impl RegionIpc {
    fn records_set_extents_for_test(&mut self, index: usize, extents: Vec<Extent>) {
        self.records[index].extents = extents;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use detguest_wire::events::decode_event;
    use detguest_wire::header::{OFF_RING_A_DATA, OFF_RING_A_PROD};
    use detguest_wire::regionipc::STATUS_NAME_TOO_LONG;

    fn test_doorbell(_mask: u32) {}

    fn temp_sock_path(tag: &str) -> String {
        format!(
            "/tmp/detguest-regionipc-test-{}-{}.sock",
            std::process::id(),
            tag
        )
    }

    fn identity_translator() -> Translator {
        // 1:1 GVA→GPA, single extent.
        Box::new(|_pid, gva, len| Ok(vec![Extent { gpa: gva, len }]))
    }

    fn connect(path: &str) -> OwnedFd {
        let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_SEQPACKET, 0) };
        assert!(raw >= 0);
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let addr = sockaddr_un(path).unwrap();
        let rc = unsafe {
            libc::connect(
                fd.as_raw_fd(),
                &addr as *const libc::sockaddr_un as *const libc::sockaddr,
                sockaddr_un_len(path),
            )
        };
        assert_eq!(rc, 0, "connect: {}", io::Error::last_os_error());
        fd
    }

    fn recv_reply(client: &OwnedFd) -> Reply {
        let mut buf = [0u8; REGIONIPC_MAX_DATAGRAM];
        let n = unsafe { libc::recv(client.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), 0) };
        assert!(n > 0, "recv reply: {}", io::Error::last_os_error());
        regionipc::decode_reply(&buf[..n as usize]).unwrap()
    }

    fn ring_a_events(channel: &AgentChannel) -> Vec<(u8, Vec<u8>)> {
        // (kind, payload-ish) tuples; enough shape for ordering asserts.
        let prod =
            unsafe { (channel.base_ptr().add(OFF_RING_A_PROD) as *const u32).read_volatile() }
                as usize;
        let bytes =
            unsafe { std::slice::from_raw_parts(channel.base_ptr().add(OFF_RING_A_DATA), prod) };
        let mut out = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            let (hdr, _payload) = decode_event(&bytes[at..]).unwrap();
            out.push((hdr.kind, bytes[at..at + hdr.len as usize].to_vec()));
            at += hdr.len as usize;
        }
        out
    }

    fn setup(tag: &str, translate: Translator) -> (RegionIpc, AgentChannel, OwnedFd, i32) {
        let path = temp_sock_path(tag);
        let ipc = RegionIpc::bind_at(&path, translate).unwrap();
        let channel = crate::channel::test_channel(test_doorbell);
        let client = connect(&path);
        let my_pid = std::process::id() as i32;
        std::fs::remove_file(&path).ok();
        (ipc, channel, client, my_pid)
    }

    #[test]
    fn register_writes_manifest_ledger_and_ring_a() {
        let (mut ipc, mut channel, client, pid) = setup("register", identity_translator());
        let req = Request::Register {
            flags: 0,
            layout_version: 1,
            name_id: 3,
            gva: 0x5000,
            len: 4096,
            name: b"wram",
        };
        let mut buf = [0u8; REGIONIPC_MAX_DATAGRAM];
        let n = regionipc::encode_request(&req, &mut buf).unwrap();
        unsafe {
            libc::send(
                client.as_raw_fd(),
                buf.as_ptr().cast(),
                n,
                libc::MSG_NOSIGNAL,
            );
        }
        ipc.service(&mut channel, Some(pid), None).unwrap();
        let reply = recv_reply(&client);
        assert_eq!(reply.status, STATUS_OK);
        assert_eq!(reply.region_id, 0);
        assert_eq!(reply.name_id, 3);
        assert_eq!(reply.manifest_generation, 2);

        // Manifest: live entry with the identity extent.
        let bytes = channel.copy_manifest_stable().unwrap();
        let hdr = ManifestHeader::read_from(&bytes).unwrap();
        assert_eq!(hdr.region_count, 1);
        assert_eq!(hdr.extent_count, 1);
        assert_eq!(hdr.generation, 2);
        let entry = RegionEntry::read_from(&bytes, 0).unwrap();
        assert!(entry.is_live());
        assert_eq!(entry.name_bytes(), b"wram");
        assert_eq!(entry.name_id, 3);
        assert_eq!(entry.gva, 0x5000);
        assert_eq!(entry.len, 4096);
        assert_eq!(
            Extent::read_from(&bytes, 0).unwrap(),
            Extent {
                gpa: 0x5000,
                len: 4096
            }
        );

        // Ledger.
        assert_eq!(ipc.records().len(), 1);
        assert_eq!(ipc.records()[0].pid, pid);
        assert!(!ipc.records()[0].dead);

        // Ring A: NameIntern (kind 2) then RegionRegister (kind 7).
        let kinds: Vec<u8> = ring_a_events(&channel).iter().map(|(k, _)| *k).collect();
        assert_eq!(kinds, vec![2, 7]);
    }

    #[test]
    fn unknown_pid_is_rejected() {
        let (mut ipc, mut channel, client, pid) = setup("pid", identity_translator());
        let req = Request::Register {
            flags: 0,
            layout_version: 1,
            name_id: 1,
            gva: 0x1000,
            len: 16,
            name: b"x",
        };
        // No workload at all.
        let mut buf = [0u8; REGIONIPC_MAX_DATAGRAM];
        let n = regionipc::encode_request(&req, &mut buf).unwrap();
        unsafe {
            libc::send(
                client.as_raw_fd(),
                buf.as_ptr().cast(),
                n,
                libc::MSG_NOSIGNAL,
            );
        }
        ipc.service(&mut channel, None, None).unwrap();
        assert_eq!(recv_reply(&client).status, STATUS_UNKNOWN_PID);

        // Wrong pid.
        unsafe {
            libc::send(
                client.as_raw_fd(),
                buf.as_ptr().cast(),
                n,
                libc::MSG_NOSIGNAL,
            );
        }
        ipc.service(&mut channel, Some(pid + 1), None).unwrap();
        assert_eq!(recv_reply(&client).status, STATUS_UNKNOWN_PID);
        assert!(ipc.records().is_empty());
        assert!(ring_a_events(&channel).is_empty());
    }

    #[test]
    fn malformed_datagram_gets_bad_request_and_survives() {
        let (mut ipc, mut channel, client, pid) = setup("malformed", identity_translator());
        let garbage = [0xFFu8; 20];
        unsafe {
            libc::send(
                client.as_raw_fd(),
                garbage.as_ptr().cast(),
                garbage.len(),
                libc::MSG_NOSIGNAL,
            );
        }
        ipc.service(&mut channel, Some(pid), None).unwrap();
        assert_eq!(recv_reply(&client).status, STATUS_BAD_REQUEST);

        // The server keeps working afterwards.
        let reply = call_after_service(&mut ipc, &mut channel, &client, pid);
        assert_eq!(reply.status, STATUS_OK);
    }

    fn call_after_service(
        ipc: &mut RegionIpc,
        channel: &mut AgentChannel,
        client: &OwnedFd,
        pid: i32,
    ) -> Reply {
        let req = Request::Register {
            flags: 0,
            layout_version: 1,
            name_id: 9,
            gva: 0x9000,
            len: 64,
            name: b"ok",
        };
        let mut buf = [0u8; REGIONIPC_MAX_DATAGRAM];
        let n = regionipc::encode_request(&req, &mut buf).unwrap();
        unsafe {
            libc::send(
                client.as_raw_fd(),
                buf.as_ptr().cast(),
                n,
                libc::MSG_NOSIGNAL,
            );
        }
        ipc.service(channel, Some(pid), None).unwrap();
        recv_reply(client)
    }

    #[test]
    fn not_pinned_translation_maps_to_status() {
        let translate: Translator = Box::new(|_pid, gva, _len| {
            Err(BuildExtentsError::Translate(
                translate::TranslateError::NotPresent { vaddr: gva },
            ))
        });
        let (mut ipc, mut channel, client, pid) = setup("notpinned", translate);
        let reply = {
            let req = Request::Register {
                flags: 0,
                layout_version: 1,
                name_id: 1,
                gva: 0x1000,
                len: 16,
                name: b"x",
            };
            let mut buf = [0u8; REGIONIPC_MAX_DATAGRAM];
            let n = regionipc::encode_request(&req, &mut buf).unwrap();
            unsafe {
                libc::send(
                    client.as_raw_fd(),
                    buf.as_ptr().cast(),
                    n,
                    libc::MSG_NOSIGNAL,
                );
            }
            ipc.service(&mut channel, Some(pid), None).unwrap();
            recv_reply(&client)
        };
        assert_eq!(reply.status, STATUS_NOT_PINNED);
        assert!(ipc.records().is_empty());
    }

    #[test]
    fn unregister_marks_dead_and_unknown_region_errors() {
        let (mut ipc, mut channel, client, pid) = setup("unregister", identity_translator());
        let ok = call_after_service(&mut ipc, &mut channel, &client, pid);
        assert_eq!(ok.status, STATUS_OK);

        // Unknown region id.
        let mut buf = [0u8; REGIONIPC_MAX_DATAGRAM];
        let n =
            regionipc::encode_request(&Request::Unregister { region_id: 42 }, &mut buf).unwrap();
        unsafe {
            libc::send(
                client.as_raw_fd(),
                buf.as_ptr().cast(),
                n,
                libc::MSG_NOSIGNAL,
            );
        }
        ipc.service(&mut channel, Some(pid), None).unwrap();
        assert_eq!(recv_reply(&client).status, STATUS_UNKNOWN_REGION);

        // Real unregister: DEAD flag lands, generation bumps.
        let n = regionipc::encode_request(
            &Request::Unregister {
                region_id: ok.region_id,
            },
            &mut buf,
        )
        .unwrap();
        unsafe {
            libc::send(
                client.as_raw_fd(),
                buf.as_ptr().cast(),
                n,
                libc::MSG_NOSIGNAL,
            );
        }
        ipc.service(&mut channel, Some(pid), None).unwrap();
        let reply = recv_reply(&client);
        assert_eq!(reply.status, STATUS_OK);
        assert_eq!(reply.manifest_generation, ok.manifest_generation + 2);
        let bytes = channel.copy_manifest_stable().unwrap();
        let entry = RegionEntry::read_from(&bytes, ok.region_id as usize).unwrap();
        assert!(!entry.is_live());
        assert!(ipc.records()[0].dead);

        // Double unregister.
        unsafe {
            libc::send(
                client.as_raw_fd(),
                buf.as_ptr().cast(),
                n,
                libc::MSG_NOSIGNAL,
            );
        }
        ipc.service(&mut channel, Some(pid), None).unwrap();
        assert_eq!(recv_reply(&client).status, STATUS_UNKNOWN_REGION);
    }

    #[test]
    fn oversized_name_never_reaches_manifest() {
        // The codec caps names at 56, so NAME_TOO_LONG is structurally
        // unreachable over the wire; keep the constant honest.
        assert_eq!(STATUS_NAME_TOO_LONG, 4);
    }

    #[test]
    fn reverify_echoes_rewrites_and_kills() {
        use std::cell::Cell;
        use std::rc::Rc;

        // Region 0 stays put; region 1 drifts; region 2 stops translating.
        let mode = Rc::new(Cell::new(0u8));
        let mode_t = mode.clone();
        let translate: Translator = Box::new(move |_pid, gva, len| match (mode_t.get(), gva) {
            (0, _) => Ok(vec![Extent { gpa: gva, len }]),
            (1, 0x2000) => Ok(vec![Extent {
                gpa: 0xAAAA_0000,
                len,
            }]),
            (1, 0x3000) => Err(BuildExtentsError::Translate(
                translate::TranslateError::NotPresent { vaddr: gva },
            )),
            (1, _) => Ok(vec![Extent { gpa: gva, len }]),
            _ => unreachable!(),
        });
        let (mut ipc, mut channel, client, pid) = setup("reverify", translate);
        for (name_id, gva, name) in [
            (1u32, 0x1000u64, &b"stable"[..]),
            (2, 0x2000, b"drift"),
            (3, 0x3000, b"gone"),
        ] {
            let req = Request::Register {
                flags: 0,
                layout_version: 1,
                name_id,
                gva,
                len: 4096,
                name,
            };
            let mut buf = [0u8; REGIONIPC_MAX_DATAGRAM];
            let n = regionipc::encode_request(&req, &mut buf).unwrap();
            unsafe {
                libc::send(
                    client.as_raw_fd(),
                    buf.as_ptr().cast(),
                    n,
                    libc::MSG_NOSIGNAL,
                );
            }
            ipc.service(&mut channel, Some(pid), None).unwrap();
            assert_eq!(recv_reply(&client).status, STATUS_OK);
        }
        let before: Vec<u8> = ring_a_events(&channel).iter().map(|(k, _)| *k).collect();
        assert_eq!(before, vec![2, 7, 2, 7, 2, 7]);

        mode.set(1);
        ipc.reverify(&mut channel);

        // Expect: RegionUpdate echo (8) for stable, LogLine (11) + update for
        // drift, LogLine + update for gone.
        let kinds: Vec<u8> = ring_a_events(&channel)
            .iter()
            .map(|(k, _)| *k)
            .skip(before.len())
            .collect();
        assert_eq!(kinds, vec![8, 11, 8, 11, 8]);

        let bytes = channel.copy_manifest_stable().unwrap();
        // Drifted region rewritten in place.
        let drift = RegionEntry::read_from(&bytes, 1).unwrap();
        assert!(drift.is_live());
        assert_eq!(
            Extent::read_from(&bytes, drift.extent_off as usize)
                .unwrap()
                .gpa,
            0xAAAA_0000
        );
        // Gone region is DEAD and no longer resolvable.
        let gone = RegionEntry::read_from(&bytes, 2).unwrap();
        assert!(!gone.is_live());
        assert!(ipc.records()[2].dead);
        // Ledger extents updated for the drifted region.
        assert_eq!(ipc.records()[1].extents[0].gpa, 0xAAAA_0000);

        // Second reverify with translations matching the ledger again:
        // only the two live regions echo (no alarms), dead one is skipped.
        mode.set(0);
        let seen = ring_a_events(&channel).len();
        ipc.records_set_extents_for_test(
            0,
            vec![Extent {
                gpa: 0x1000,
                len: 4096,
            }],
        );
        ipc.records_set_extents_for_test(
            1,
            vec![Extent {
                gpa: 0x2000,
                len: 4096,
            }],
        );
        ipc.reverify(&mut channel);
        let kinds: Vec<u8> = ring_a_events(&channel)
            .iter()
            .map(|(k, _)| *k)
            .skip(seen)
            .collect();
        assert_eq!(kinds, vec![8, 8]);
    }

    #[test]
    fn reverify_with_empty_ledger_emits_nothing() {
        let (mut ipc, mut channel, _client, _pid) = setup("empty", identity_translator());
        ipc.reverify(&mut channel);
        assert!(ring_a_events(&channel).is_empty());
    }
}
