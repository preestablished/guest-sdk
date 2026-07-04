//! The supervise loop (ARCHITECTURE.md §4 steps 8–10): a single-threaded
//! epoll loop over the workload's stdout/stderr pipes, a SIGCHLD signalfd,
//! and a periodic timerfd. One thread = no scheduler-dependent interleavings
//! inside the agent (§4); the poll cadence (every loop pass + on SIGCHLD) is
//! deterministic (§7).
//!
//! Permitted-unsafe module: fork/exec, epoll, signalfd, timerfd plumbing via
//! libc. The pure line-framing logic ([`LineBuf`]) is safe and unit-tested.
#![allow(unsafe_code)]

use std::io;
use std::os::fd::RawFd;
use std::sync::atomic::{AtomicU64, Ordering};

use detguest_wire::events::{log_stream, EventPayload};
use detguest_wire::WorkloadCtrl;

use crate::boot::{BootManifest, Unit};
use crate::channel::AgentChannel;
use crate::control;

/// LogLine level used for workload stdout lines (2 = info).
pub const STDOUT_LEVEL: u8 = 2;
/// LogLine level used for workload stderr lines (0 = error).
pub const STDERR_LEVEL: u8 = 0;

/// Accumulates pipe bytes and yields complete lines (newline-stripped),
/// flushing oversized lines at the wire cap so a workload printing without
/// newlines cannot grow the buffer unboundedly.
#[derive(Debug, Default)]
pub struct LineBuf {
    buf: Vec<u8>,
}

impl LineBuf {
    /// Maximum line length emitted; longer lines are split at this size
    /// (matches `MAX_LOG_MSG` so nothing is silently clipped downstream).
    pub const MAX_LINE: usize = detguest_wire::events::MAX_LOG_MSG;

    /// Feed bytes; call `emit` for each completed line.
    pub fn push(&mut self, bytes: &[u8], mut emit: impl FnMut(&[u8])) {
        self.buf.extend_from_slice(bytes);
        loop {
            if let Some(nl) = self.buf.iter().position(|&b| b == b'\n') {
                emit(&self.buf[..nl]);
                self.buf.drain(..=nl);
            } else if self.buf.len() >= Self::MAX_LINE {
                emit(&self.buf[..Self::MAX_LINE]);
                self.buf.drain(..Self::MAX_LINE);
            } else {
                break;
            }
        }
    }

    /// Flush a trailing unterminated line (pipe EOF).
    pub fn finish(&mut self, mut emit: impl FnMut(&[u8])) {
        if !self.buf.is_empty() {
            emit(&self.buf);
            self.buf.clear();
        }
    }
}

/// A spawned workload (ARCHITECTURE.md §4 step 9).
pub struct Workload {
    /// Guest PID.
    pub pid: i32,
    /// Boot-manifest unit id.
    pub unit: u32,
    /// Read end of the stdout pipe (non-blocking).
    pub stdout: RawFd,
    /// Read end of the stderr pipe (non-blocking).
    pub stderr: RawFd,
    pub(crate) outbuf: LineBuf,
    pub(crate) errbuf: LineBuf,
    /// SIGSTOP'd by a FORCED quiesce (awaiting Resume → SIGCONT).
    pub stopped: bool,
}

/// Pending graceful-shutdown state: SIGKILL fires at `kill_deadline`.
pub struct ShutdownState {
    /// CLOCK_MONOTONIC_RAW ns deadline for SIGKILL.
    pub kill_deadline: u64,
}

static EVENT_VNANOS: AtomicU64 = AtomicU64::new(1);

/// Deterministic ring-record timestamp. The host also records the drain
/// icount; this field must not depend on Linux wall-clock state.
pub fn vnanos() -> u64 {
    EVENT_VNANOS.fetch_add(1, Ordering::Relaxed)
}

fn monotonic_raw_ns() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: plain clock_gettime into a local.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC_RAW, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

/// Spawn `unit` (StartWorkload / autostart shared path — ARCHITECTURE.md §4
/// step 9 and §6: argv comes from the boot manifest, never the wire):
/// stdout/stderr pipes, `DETGUEST_CHANNEL_FD`, `RLIMIT_MEMLOCK=∞`, exec.
pub fn spawn(unit: &Unit, channel_fd: RawFd, control_fd: Option<RawFd>) -> io::Result<Workload> {
    let mut out_pipe = [0i32; 2];
    let mut err_pipe = [0i32; 2];
    // SAFETY: libc pipe2/fork/exec sequence; the child only calls
    // async-signal-safe functions before exec (dup2/fcntl/setrlimit/execve).
    unsafe {
        if libc::pipe2(out_pipe.as_mut_ptr(), libc::O_CLOEXEC) != 0 {
            return Err(io::Error::last_os_error());
        }
        if libc::pipe2(err_pipe.as_mut_ptr(), libc::O_CLOEXEC) != 0 {
            let e = io::Error::last_os_error();
            libc::close(out_pipe[0]);
            libc::close(out_pipe[1]);
            return Err(e);
        }
        let exec = std::ffi::CString::new(unit.exec.as_str())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in exec path"))?;
        let mut argv_owned: Vec<std::ffi::CString> = vec![exec.clone()];
        for a in &unit.args {
            argv_owned.push(
                std::ffi::CString::new(a.as_str())
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in arg"))?,
            );
        }
        let mut child_channel_fd = channel_fd;
        let dup_channel_fd = if control_fd.is_some() && channel_fd == control::child_fd_number() {
            let fd = libc::fcntl(
                channel_fd,
                libc::F_DUPFD_CLOEXEC,
                control::child_fd_number() + 1,
            );
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            child_channel_fd = fd;
            Some(fd)
        } else {
            None
        };
        let env =
            std::ffi::CString::new(format!("DETGUEST_CHANNEL_FD={child_channel_fd}")).unwrap();

        let pid = libc::fork();
        if pid < 0 {
            if let Some(fd) = dup_channel_fd {
                libc::close(fd);
            }
            return Err(io::Error::last_os_error());
        }
        if pid == 0 {
            // Child. dup2 clears O_CLOEXEC on the duplicated ends.
            libc::dup2(out_pipe[1], 1);
            libc::dup2(err_pipe[1], 2);
            if let Some(fd) = control_fd {
                if fd != control::child_fd_number() {
                    libc::dup2(fd, control::child_fd_number());
                } else {
                    let flags = libc::fcntl(fd, libc::F_GETFD);
                    libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
                }
            }
            // Channel fd: clear CLOEXEC so the workload inherits it.
            let flags = libc::fcntl(child_channel_fd, libc::F_GETFD);
            libc::fcntl(child_channel_fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
            // RLIMIT_MEMLOCK=∞ (mlock of published regions, API.md §1.5).
            let lim = libc::rlimit {
                rlim_cur: libc::RLIM_INFINITY,
                rlim_max: libc::RLIM_INFINITY,
            };
            libc::setrlimit(libc::RLIMIT_MEMLOCK, &lim);
            let mut argv: Vec<*const libc::c_char> =
                argv_owned.iter().map(|c| c.as_ptr()).collect();
            argv.push(std::ptr::null());
            let envp: [*const libc::c_char; 2] = [env.as_ptr(), std::ptr::null()];
            libc::execve(exec.as_ptr(), argv.as_ptr(), envp.as_ptr());
            libc::_exit(127);
        }
        if let Some(fd) = dup_channel_fd {
            libc::close(fd);
        }
        // Parent: close child ends; make read ends non-blocking.
        libc::close(out_pipe[1]);
        libc::close(err_pipe[1]);
        for fd in [out_pipe[0], err_pipe[0]] {
            let fl = libc::fcntl(fd, libc::F_GETFL);
            libc::fcntl(fd, libc::F_SETFL, fl | libc::O_NONBLOCK);
        }
        Ok(Workload {
            pid,
            unit: unit.id,
            stdout: out_pipe[0],
            stderr: err_pipe[0],
            outbuf: LineBuf::default(),
            errbuf: LineBuf::default(),
            stopped: false,
        })
    }
}

/// The supervisor: channel + boot manifest + workload + loop fds.
pub struct Supervisor {
    /// The agent channel.
    pub channel: AgentChannel,
    /// Parsed boot manifest.
    pub manifest: BootManifest,
    /// The (single, v1) supervised workload.
    pub workload: Option<Workload>,
    /// Current LogLine level mask (bit L gates level L).
    pub log_mask: u32,
    /// Graceful-shutdown countdown, when active.
    pub shutdown: Option<ShutdownState>,
    /// Region-registration IPC server + ledger (bound by `runtime::run`;
    /// `None` only in tests that don't exercise regions).
    pub(crate) region_ipc: Option<crate::region_ipc::RegionIpc>,
    /// The agent's end of the workload's fd-3 control socketpair, held for
    /// the workload's lifetime. The workload's frame loop polls its end at
    /// every frame boundary and treats EOF as agent death, so dropping this
    /// while the workload runs is a protocol violation (it killed the first
    /// real boot right after Ready). The agent sends nothing on it
    /// post-Start today; host-driven HashRequest/Shutdown relays are future
    /// work.
    pub(crate) workload_control: Option<crate::control::ControlSocket>,
    /// Outstanding FORCED-quiesce token (v1: at most one).
    forced_token: u64,
    epfd: RawFd,
    sigfd: RawFd,
    timerfd: RawFd,
}

const TOK_SIG: u64 = 1;
const TOK_TIMER: u64 = 2;
const TOK_OUT: u64 = 3;
const TOK_ERR: u64 = 4;
const TOK_REGION_LISTENER: u64 = 5;
const TOK_REGION_CONN: u64 = 6;

impl Supervisor {
    /// Build the loop fds: blocked-SIGCHLD signalfd + 10 ms periodic timerfd
    /// (virtual time — the tick cadence is deterministic).
    pub fn new(channel: AgentChannel, manifest: BootManifest) -> io::Result<Supervisor> {
        // SAFETY: standard epoll/signalfd/timerfd setup.
        unsafe {
            let mut mask: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut mask);
            libc::sigaddset(&mut mask, libc::SIGCHLD);
            libc::sigprocmask(libc::SIG_BLOCK, &mask, std::ptr::null_mut());
            let sigfd = libc::signalfd(-1, &mask, libc::SFD_NONBLOCK | libc::SFD_CLOEXEC);
            if sigfd < 0 {
                return Err(io::Error::last_os_error());
            }
            let timerfd = libc::timerfd_create(
                libc::CLOCK_MONOTONIC,
                libc::TFD_NONBLOCK | libc::TFD_CLOEXEC,
            );
            if timerfd < 0 {
                return Err(io::Error::last_os_error());
            }
            let spec = libc::itimerspec {
                it_interval: libc::timespec {
                    tv_sec: 0,
                    tv_nsec: 10_000_000,
                },
                it_value: libc::timespec {
                    tv_sec: 0,
                    tv_nsec: 10_000_000,
                },
            };
            libc::timerfd_settime(timerfd, 0, &spec, std::ptr::null_mut());
            let epfd = libc::epoll_create1(libc::EPOLL_CLOEXEC);
            if epfd < 0 {
                return Err(io::Error::last_os_error());
            }
            for (fd, tok) in [(sigfd, TOK_SIG), (timerfd, TOK_TIMER)] {
                let mut ev = libc::epoll_event {
                    events: libc::EPOLLIN as u32,
                    u64: tok,
                };
                if libc::epoll_ctl(epfd, libc::EPOLL_CTL_ADD, fd, &mut ev) != 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(Supervisor {
                channel,
                manifest,
                workload: None,
                log_mask: 0x1F,
                shutdown: None,
                region_ipc: None,
                workload_control: None,
                forced_token: 0,
                epfd,
                sigfd,
                timerfd,
            })
        }
    }

    /// Install the region IPC server and register its listener with the
    /// epoll loop (runtime calls this before the autostart unit spawns).
    pub(crate) fn install_region_ipc(&mut self, ipc: crate::region_ipc::RegionIpc) {
        let mut ev = libc::epoll_event {
            events: libc::EPOLLIN as u32,
            u64: TOK_REGION_LISTENER,
        };
        // SAFETY: registering the listener fd with our epoll instance.
        unsafe {
            libc::epoll_ctl(self.epfd, libc::EPOLL_CTL_ADD, ipc.listener_fd(), &mut ev);
        }
        self.region_ipc = Some(ipc);
    }

    /// Accept + drain pending region-IPC requests (non-blocking; the
    /// deadlock-avoidance primitive — called from the epoll loop, the
    /// expected-regions wait, and the control-recv idle loop).
    pub(crate) fn service_region_ipc(&mut self) -> io::Result<()> {
        let Some(mut ipc) = self.region_ipc.take() else {
            return Ok(());
        };
        let pid = self.workload.as_ref().map(|w| w.pid);
        let result = ipc.service(&mut self.channel, pid, Some((self.epfd, TOK_REGION_CONN)));
        self.region_ipc = Some(ipc);
        result
    }

    /// `ReverifyRegions` dispatch (ring C → [`crate::commands`]).
    pub(crate) fn reverify_regions(&mut self) {
        let Some(mut ipc) = self.region_ipc.take() else {
            return; // no server bound (tests): nothing registered, no-op
        };
        ipc.reverify(&mut self.channel);
        self.region_ipc = Some(ipc);
    }

    /// Start `unit` and emit `WorkloadStarted` (shared by ring-C
    /// StartWorkload and boot autostart — the autostart path involves NO
    /// ring-C record, ARCHITECTURE.md §4 step 7).
    pub fn start_unit(&mut self, unit_id: u32) -> io::Result<()> {
        self.start_unit_inner(unit_id, None)
    }

    pub fn start_unit_with_control(&mut self, unit_id: u32, control_fd: RawFd) -> io::Result<()> {
        self.start_unit_inner(unit_id, Some(control_fd))
    }

    fn start_unit_inner(&mut self, unit_id: u32, control_fd: Option<RawFd>) -> io::Result<()> {
        if self.workload.is_some() {
            // v1 supervises exactly one workload; silently replacing it
            // would leak the old process and emit a lying WorkloadStarted.
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "a workload is already running",
            ));
        }
        let unit = self
            .manifest
            .unit(unit_id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown unit id"))?
            .clone();
        self.log_mask = unit.log_mask;
        let w = spawn(&unit, self.channel.fd(), control_fd)?;
        // SAFETY: registering the pipe fds with our epoll instance.
        unsafe {
            for (fd, tok) in [(w.stdout, TOK_OUT), (w.stderr, TOK_ERR)] {
                let mut ev = libc::epoll_event {
                    events: libc::EPOLLIN as u32,
                    u64: tok,
                };
                libc::epoll_ctl(self.epfd, libc::EPOLL_CTL_ADD, fd, &mut ev);
            }
        }
        let (pid, uid) = (w.pid, w.unit);
        self.workload = Some(w);
        self.channel.emit(
            vnanos(),
            0,
            &EventPayload::WorkloadStarted {
                guest_pid: pid as u32,
                unit: uid,
            },
        );
        Ok(())
    }

    /// Drain a workload pipe into LogLine events (droppable; level gated by
    /// the mask). `which` is TOK_OUT or TOK_ERR.
    fn drain_pipe(&mut self, which: u64) {
        let (fd, stream, level) = match (&self.workload, which) {
            (Some(w), TOK_OUT) => (w.stdout, log_stream::STDOUT, STDOUT_LEVEL),
            (Some(w), _) => (w.stderr, log_stream::STDERR, STDERR_LEVEL),
            (None, _) => return,
        };
        let mut chunk = [0u8; 4096];
        loop {
            // SAFETY: read into a local buffer from a non-blocking fd.
            let n = unsafe { libc::read(fd, chunk.as_mut_ptr().cast(), chunk.len()) };
            if n <= 0 {
                return;
            }
            let mut lines: Vec<Vec<u8>> = Vec::new();
            {
                let w = self.workload.as_mut().expect("checked above");
                let buf = if which == TOK_OUT {
                    &mut w.outbuf
                } else {
                    &mut w.errbuf
                };
                buf.push(&chunk[..n as usize], |line| lines.push(line.to_vec()));
            }
            self.emit_lines(stream, level, &lines);
        }
    }

    fn emit_lines(&mut self, stream: u8, level: u8, lines: &[Vec<u8>]) {
        if self.log_mask & (1u32 << level) == 0 {
            return;
        }
        for line in lines {
            self.channel.emit(
                vnanos(),
                0,
                &EventPayload::LogLine {
                    stream,
                    level,
                    msg: line,
                },
            );
        }
    }

    /// Reap exited/stopped children (SIGCHLD or shutdown sweep).
    fn reap(&mut self) {
        loop {
            let mut status: i32 = 0;
            // SAFETY: waitpid into a local; WUNTRACED catches FORCED-quiesce
            // SIGSTOPs (ARCHITECTURE.md §6).
            let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG | libc::WUNTRACED) };
            if pid <= 0 {
                return;
            }
            if self.workload.as_ref().map(|w| w.pid) != Some(pid) {
                continue; // unrelated child (none expected in v1)
            }
            if libc::WIFSTOPPED(status) {
                // FORCED quiesce landed: acknowledge host-ward (§6).
                if let Some(w) = self.workload.as_mut() {
                    w.stopped = true;
                }
                let token = self.forced_token;
                self.channel
                    .emit_with_doorbell(vnanos(), 0, &EventPayload::QuiesceReady { token });
                continue;
            }
            // Exited or killed: final pipe drain, flush partial lines, emit
            // WorkloadExited (critical + doorbell).
            self.drain_pipe(TOK_OUT);
            self.drain_pipe(TOK_ERR);
            let w = self.workload.take().expect("checked above");
            // The dead workload's control socket must not linger across a
            // future restart of the (single, v1) unit slot.
            self.workload_control = None;
            let mut out_lines: Vec<Vec<u8>> = Vec::new();
            let mut err_lines: Vec<Vec<u8>> = Vec::new();
            let (mut ob, mut eb) = (w.outbuf, w.errbuf);
            ob.finish(|l| out_lines.push(l.to_vec()));
            eb.finish(|l| err_lines.push(l.to_vec()));
            self.emit_lines(log_stream::STDOUT, STDOUT_LEVEL, &out_lines);
            self.emit_lines(log_stream::STDERR, STDERR_LEVEL, &err_lines);
            // SAFETY: deregister + close the pipe fds.
            unsafe {
                libc::epoll_ctl(
                    self.epfd,
                    libc::EPOLL_CTL_DEL,
                    w.stdout,
                    std::ptr::null_mut(),
                );
                libc::epoll_ctl(
                    self.epfd,
                    libc::EPOLL_CTL_DEL,
                    w.stderr,
                    std::ptr::null_mut(),
                );
                libc::close(w.stdout);
                libc::close(w.stderr);
            }
            let (exit_code, term_signal) = if libc::WIFEXITED(status) {
                (libc::WEXITSTATUS(status), 0)
            } else {
                (-1, libc::WTERMSIG(status))
            };
            self.channel.emit_with_doorbell(
                vnanos(),
                0,
                &EventPayload::WorkloadExited {
                    guest_pid: pid as u32,
                    exit_code,
                    term_signal,
                },
            );
        }
    }

    /// Run until a shutdown completes (the caller then powers off). Each
    /// pass: wait (bounded), service fd events, sweep the shutdown deadline,
    /// poll ring C (deterministic cadence: every pass + SIGCHLD).
    pub fn run(&mut self) -> io::Result<()> {
        loop {
            let mut events = [libc::epoll_event { events: 0, u64: 0 }; 8];
            // SAFETY: epoll_wait into a local array.
            let n = unsafe {
                libc::epoll_wait(self.epfd, events.as_mut_ptr(), events.len() as i32, 100)
            };
            for ev in events.iter().take(n.max(0) as usize) {
                match ev.u64 {
                    TOK_SIG => {
                        let mut info = [0u8; 128]; // >= sizeof(signalfd_siginfo)
                                                   // SAFETY: drain the signalfd.
                        while unsafe {
                            libc::read(self.sigfd, info.as_mut_ptr().cast(), info.len())
                        } > 0
                        {}
                        self.reap();
                    }
                    TOK_TIMER => {
                        let mut ticks = [0u8; 8];
                        // SAFETY: drain the timerfd counter.
                        unsafe { libc::read(self.timerfd, ticks.as_mut_ptr().cast(), 8) };
                    }
                    TOK_OUT | TOK_ERR => {
                        self.drain_pipe(ev.u64);
                        if ev.events & (libc::EPOLLHUP as u32 | libc::EPOLLERR as u32) != 0 {
                            // Writer closed without exiting: deregister so a
                            // permanently-HUP fd cannot busy-spin the loop.
                            // The fd stays open for the final drain at reap.
                            if let Some(w) = &self.workload {
                                let fd = if ev.u64 == TOK_OUT {
                                    w.stdout
                                } else {
                                    w.stderr
                                };
                                // SAFETY: removing our own registration.
                                unsafe {
                                    libc::epoll_ctl(
                                        self.epfd,
                                        libc::EPOLL_CTL_DEL,
                                        fd,
                                        std::ptr::null_mut(),
                                    );
                                }
                            }
                        }
                    }
                    TOK_REGION_LISTENER | TOK_REGION_CONN => {
                        self.service_region_ipc()?;
                    }
                    _ => {}
                }
            }
            // Region IPC every pass (same deterministic cadence as ring C):
            // covers requests that raced the epoll registration.
            self.service_region_ipc()?;
            // Shutdown progress (virtual-time deadline).
            if let Some(s) = &self.shutdown {
                match &self.workload {
                    Some(w) if monotonic_raw_ns() >= s.kill_deadline => {
                        // SAFETY: SIGKILL the supervised child; reap follows
                        // via SIGCHLD.
                        unsafe { libc::kill(w.pid, libc::SIGKILL) };
                    }
                    Some(_) => {}
                    None => return Ok(()), // workload reaped; power off now
                }
            }
            // Ring C poll — every pass (ARCHITECTURE.md §4 step 8).
            while let Some(cmd) = self
                .channel
                .poll_command()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{e:?}")))?
            {
                if crate::commands::handle(self, cmd)? {
                    return Ok(()); // immediate shutdown
                }
            }
        }
    }

    /// Begin a graceful shutdown (SIGTERM now; SIGKILL after 2 s virtual).
    pub fn begin_graceful_shutdown(&mut self) {
        match &self.workload {
            Some(w) => {
                // SAFETY: SIGTERM the supervised child.
                unsafe { libc::kill(w.pid, libc::SIGTERM) };
                self.shutdown = Some(ShutdownState {
                    kill_deadline: monotonic_raw_ns() + 2_000_000_000,
                });
            }
            None => self.shutdown = Some(ShutdownState { kill_deadline: 0 }),
        }
    }

    /// Immediate shutdown: SIGKILL + synchronous reap + WorkloadExited.
    pub fn immediate_shutdown(&mut self) {
        self.workload_control = None;
        if let Some(w) = self.workload.take() {
            // SAFETY: SIGKILL + blocking waitpid on our child.
            unsafe {
                libc::kill(w.pid, libc::SIGKILL);
                let mut status = 0i32;
                libc::waitpid(w.pid, &mut status, 0);
            }
            self.channel.emit_with_doorbell(
                vnanos(),
                0,
                &EventPayload::WorkloadExited {
                    guest_pid: w.pid as u32,
                    exit_code: -1,
                    term_signal: libc::SIGKILL,
                },
            );
        }
    }

    /// FORCED quiesce: SIGSTOP; the WUNTRACED reap emits QuiesceReady (§6).
    pub fn forced_quiesce(&mut self, token: u64) {
        self.forced_token = token;
        match &self.workload {
            Some(w) => {
                // SAFETY: SIGSTOP the supervised child.
                unsafe { libc::kill(w.pid, libc::SIGSTOP) };
            }
            None => {
                // No workload: the agent itself acknowledges.
                self.channel
                    .emit_with_doorbell(vnanos(), 0, &EventPayload::QuiesceReady { token });
            }
        }
    }

    /// Resume a FORCED-quiesced workload (ring-C Resume).
    pub fn forced_resume(&mut self) {
        if let Some(w) = self.workload.as_mut() {
            if w.stopped {
                // SAFETY: SIGCONT the stopped child.
                unsafe { libc::kill(w.pid, libc::SIGCONT) };
                w.stopped = false;
            }
        }
    }

    /// COOP quiesce relay onto ring I (§6).
    pub fn relay_quiesce(&mut self, token: u64) {
        let _ = self
            .channel
            .relay_workload_ctrl(vnanos(), &WorkloadCtrl::QuiesceReq { token });
    }

    /// COOP Resume relay onto ring I.
    pub fn relay_resume(&mut self, token: u64) {
        let _ = self
            .channel
            .relay_workload_ctrl(vnanos(), &WorkloadCtrl::Resume { token });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn linebuf_frames_lines() {
        let mut lb = LineBuf::default();
        let mut got: Vec<Vec<u8>> = Vec::new();
        lb.push(b"hello\nwor", |l| got.push(l.to_vec()));
        assert_eq!(got, vec![b"hello".to_vec()]);
        lb.push(b"ld\n\n", |l| got.push(l.to_vec()));
        assert_eq!(
            got,
            vec![b"hello".to_vec(), b"world".to_vec(), b"".to_vec()]
        );
    }

    #[test]
    fn linebuf_caps_runaway_lines() {
        let mut lb = LineBuf::default();
        let mut got = 0usize;
        lb.push(&vec![b'x'; LineBuf::MAX_LINE * 2 + 10], |l| {
            assert!(l.len() <= LineBuf::MAX_LINE);
            got += 1;
        });
        assert_eq!(got, 2, "two full chunks emitted, remainder buffered");
        lb.finish(|l| {
            assert_eq!(l.len(), 10);
            got += 1;
        });
        assert_eq!(got, 3);
    }

    #[test]
    fn spawn_exports_sdk_channel_fd_and_preserves_log_pipes() {
        let channel_fd = memfd("detguest-agent-spawn-test");
        let script = "\
printf 'fd=%s\\n' \"$DETGUEST_CHANNEL_FD\"
if [ -e \"/proc/$$/fd/$DETGUEST_CHANNEL_FD\" ]; then
  printf 'fd-open\\n'
else
  printf 'fd-closed\\n'
  exit 41
fi
printf 'memlock=%s\\n' \"$(ulimit -l)\"
printf 'stderr-still-works\\n' >&2
";
        let unit = Unit {
            id: 7,
            exec: "/bin/sh".to_string(),
            args: vec!["-c".to_string(), script.to_string()],
            log_mask: 0x1F,
            control: None,
        };

        let workload = spawn(&unit, channel_fd, None).expect("spawn shell workload");
        let status = wait_for(workload.pid);
        let stdout = read_fd_to_string(workload.stdout);
        let stderr = read_fd_to_string(workload.stderr);

        unsafe {
            libc::close(workload.stdout);
            libc::close(workload.stderr);
            libc::close(channel_fd);
        }

        assert!(libc::WIFEXITED(status), "child status: {status}");
        assert_eq!(
            libc::WEXITSTATUS(status),
            0,
            "stdout:\n{stdout}\nstderr:\n{stderr}"
        );
        assert!(
            stdout.contains(&format!("fd={channel_fd}\nfd-open\n")),
            "stdout:\n{stdout}"
        );
        if memlock_hard_limit() == libc::RLIM_INFINITY {
            assert!(stdout.contains("memlock=unlimited\n"), "stdout:\n{stdout}");
        } else {
            assert!(stdout.contains("memlock="), "stdout:\n{stdout}");
        }
        assert_eq!(stderr, "stderr-still-works\n");
    }

    #[test]
    fn linebuf_finish_flushes_partial() {
        let mut lb = LineBuf::default();
        let mut got: Vec<Vec<u8>> = Vec::new();
        lb.push(b"no newline", |l| got.push(l.to_vec()));
        assert!(got.is_empty());
        lb.finish(|l| got.push(l.to_vec()));
        assert_eq!(got, vec![b"no newline".to_vec()]);
    }

    fn memfd(name: &str) -> RawFd {
        let name = CString::new(name).unwrap();
        let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
        assert!(fd >= 0, "memfd_create: {}", io::Error::last_os_error());
        fd
    }

    fn wait_for(pid: i32) -> i32 {
        loop {
            let mut status = 0i32;
            let got = unsafe { libc::waitpid(pid, &mut status, 0) };
            if got == pid {
                return status;
            }
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            panic!("waitpid({pid}): {err}");
        }
    }

    fn read_fd_to_string(fd: RawFd) -> String {
        let mut out = Vec::new();
        let mut buf = [0u8; 256];
        loop {
            let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
            if n > 0 {
                out.extend_from_slice(&buf[..n as usize]);
                continue;
            }
            if n == 0 {
                break;
            }
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::WouldBlock {
                break;
            }
            panic!("read({fd}): {err}");
        }
        String::from_utf8(out).expect("test workload emits UTF-8")
    }

    fn memlock_hard_limit() -> libc::rlim_t {
        let mut lim = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        let rc = unsafe { libc::getrlimit(libc::RLIMIT_MEMLOCK, &mut lim) };
        assert_eq!(rc, 0, "getrlimit: {}", io::Error::last_os_error());
        lim.rlim_max
    }
}
