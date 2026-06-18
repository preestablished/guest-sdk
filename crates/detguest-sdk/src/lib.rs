//! detguest-sdk — in-guest instrumentation for deterministic workloads.
//!
//! This crate owns the workload-facing API described in
//! `prompts/docs/guest-sdk/API.md` section 1. The initial scaffold provides the
//! complete public surface and deterministic standalone behavior; platform
//! channel mapping and event production are implemented by the follow-on M3
//! beads.

use std::{fmt, io, sync::OnceLock};

pub use detguest_wire::FaultDecision;

mod beacons;
mod channel;
mod inject;
mod intern;
mod pio;
mod regions;

pub use regions::{RegionError, RegionFlags, RegionHandle};

/// Coverage counter slots in the auto-registered `detsdk.stats` region.
pub const BEACON_SLOTS: usize = beacons::BEACON_SLOTS;

/// Per-name violation limit before the full SDK switches to summary-only
/// accounting for a hot failing invariant.
pub const ASSERT_REPEAT_LIMIT: u32 = 16;

/// Opaque SDK handle returned by [`init`] once platform initialization succeeds.
#[derive(Debug)]
pub struct Sdk {
    _state: std::sync::Mutex<SdkState>,
}

#[derive(Debug)]
struct SdkState {
    _channel: channel::MappedChannel,
    _pio: pio::PioState,
    intern: intern::InternTable,
    beacons: beacons::BeaconCounters,
    stats: StatsState,
    frame_index: u32,
}

#[derive(Debug, Default, Clone, Copy)]
struct StatsState {
    asserts_passed_total: u64,
    asserts_failed_total: u64,
    reachable_names: u64,
    inject_queries_total: u64,
}

static SDK: OnceLock<Sdk> = OnceLock::new();

/// Errors from one-time SDK initialization.
#[derive(Debug)]
#[non_exhaustive]
pub enum InitError {
    /// The workload is not running under `detguest-agent`.
    NoChannel,
    /// The inherited channel header did not contain the expected magic.
    BadChannelHeader {
        /// Magic read from the mapped channel header.
        found_magic: u64,
    },
    /// The channel protocol version is not supported by this SDK.
    ProtocolVersionMismatch {
        /// Protocol version implemented by this crate.
        guest: u32,
        /// Protocol version found in the channel header.
        channel: u32,
    },
    /// Raising I/O privilege level for detcall ports failed.
    PioPermissionDenied,
    /// Agent IPC setup or another initialization syscall failed.
    AgentSocket(io::Error),
}

impl fmt::Display for InitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InitError::NoChannel => write!(f, "DETGUEST_CHANNEL_FD is not present"),
            InitError::BadChannelHeader { found_magic } => {
                write!(f, "bad detguest channel magic 0x{found_magic:016x}")
            }
            InitError::ProtocolVersionMismatch { guest, channel } => write!(
                f,
                "detguest protocol mismatch: sdk supports {guest}, channel is {channel}"
            ),
            InitError::PioPermissionDenied => write!(f, "iopl(3) permission denied"),
            InitError::AgentSocket(err) => write!(f, "agent initialization failed: {err}"),
        }
    }
}

impl std::error::Error for InitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            InitError::AgentSocket(err) => Some(err),
            _ => None,
        }
    }
}

fn with_sdk_state<R>(f: impl FnOnce(&mut SdkState) -> R) -> Option<R> {
    let sdk = SDK.get()?;
    let mut state = sdk._state.lock().ok()?;
    Some(f(&mut state))
}

/// One-time initialization.
///
/// Without `DETGUEST_CHANNEL_FD`, returns [`InitError::NoChannel`] and the rest
/// of the SDK API stays in deterministic standalone mode.
pub fn init() -> Result<&'static Sdk, InitError> {
    init_from_channel_fd(
        std::env::var_os(channel::DETGUEST_CHANNEL_FD_ENV).as_deref(),
        &SDK,
        pio::init,
    )
}

fn init_from_channel_fd<'a>(
    raw: Option<&std::ffi::OsStr>,
    cell: &'a OnceLock<Sdk>,
    init_pio: fn() -> Result<pio::PioState, InitError>,
) -> Result<&'a Sdk, InitError> {
    if let Some(sdk) = cell.get() {
        return Ok(sdk);
    }
    let Some(raw) = raw else {
        return Err(InitError::NoChannel);
    };
    let fd = channel::parse_channel_fd(raw).map_err(InitError::AgentSocket)?;
    let channel = channel::MappedChannel::map(fd)?;
    let pio = init_pio()?;
    channel.mark_workload_attached();
    let sdk = Sdk {
        _state: std::sync::Mutex::new(SdkState {
            _channel: channel,
            _pio: pio,
            intern: intern::InternTable::default(),
            beacons: beacons::BeaconCounters::default(),
            stats: StatsState::default(),
            frame_index: 0,
        }),
    };
    match cell.set(sdk) {
        Ok(()) => Ok(cell.get().expect("SDK set succeeded")),
        Err(_) => Ok(cell.get().expect("SDK initialized concurrently")),
    }
}

/// Record a finding if `cond` is false.
///
/// Standalone mode emits no ring traffic. If `DETGUEST_STANDALONE_PANIC=1` is
/// set, a failed assertion panics after evaluating and formatting `details`.
pub fn assert_always(cond: bool, name: &'static str, details: fmt::Arguments<'_>) {
    if let Some(mut state) = sdk_state() {
        state.assert_always(cond, name, details);
        return;
    }
    if cond {
        return;
    }
    if channel::standalone_panic_enabled() {
        panic!("detguest assertion `{name}` failed: {details}");
    }
}

/// Convenience wrapper for [`assert_always`].
#[macro_export]
macro_rules! det_assert_always {
    ($cond:expr, $name:expr $(,)?) => {
        $crate::assert_always($cond, $name, format_args!(""))
    };
    ($cond:expr, $name:expr, $($arg:tt)+) => {
        $crate::assert_always($cond, $name, format_args!($($arg)+))
    };
}

/// Declare that a location should be reachable and record that it was reached.
pub fn expect_reachable(name: &'static str) {
    let _ = with_sdk_state(|state| state.expect_reachable(name));
}

/// Pre-declare a reachability target without recording a hit.
pub fn declare_reachable(name: &'static str) {
    let _ = with_sdk_state(|state| state.declare_reachable(name));
}

/// Cheap coverage counter for the scorer.
pub fn coverage_beacon(id: u32) {
    let _ = with_sdk_state(|state| state.coverage_beacon(id));
}

/// Ask the host whether to inject a fault here.
pub fn inject_point(name: &'static str) -> FaultDecision {
    let _ = with_sdk_state(|state| {
        state.stats.inject_queries_total = state.stats.inject_queries_total.saturating_add(1);
    });
    inject::inject_point(name)
}

/// Per-frame controller read from the pv-pad latch.
pub fn poll_input(port: u8) -> u32 {
    sdk_state()
        .map(|state| state._pio.poll_input(port))
        .unwrap_or_else(|| pio::poll_input(port))
}

/// Mark a completed emulated frame.
pub fn frame_mark() {
    if let Some(mut state) = sdk_state() {
        state.frame_mark();
    } else {
        pio::frame_mark();
    }
}

/// Cooperative quiesce point.
pub fn quiesce_check() {}

/// Structured SDK user log level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LogLevel {
    /// Error.
    Error = 0,
    /// Warning.
    Warn = 1,
    /// Informational.
    Info = 2,
    /// Debug.
    Debug = 3,
    /// Trace.
    Trace = 4,
}

/// Structured log line host-ward.
pub fn log_line(level: LogLevel, msg: &str) {
    let _ = with_sdk_state(|state| {
        state.log_line(level, msg);
    });
}

/// Snapshot of SDK statistics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdkStats {
    /// Stats layout version.
    pub stats_version: u32,
    /// Total successful assertion checks observed by the SDK.
    pub asserts_passed_total: u64,
    /// Total failed assertion checks observed by the SDK.
    pub asserts_failed_total: u64,
    /// Count of distinct reachability names hit.
    pub reachable_names: u64,
    /// Total inject queries issued.
    pub inject_queries_total: u64,
}

impl Default for SdkStats {
    fn default() -> Self {
        SdkStats {
            stats_version: 1,
            asserts_passed_total: 0,
            asserts_failed_total: 0,
            reachable_names: 0,
            inject_queries_total: 0,
        }
    }
}

/// Snapshot of local SDK statistics.
pub fn stats() -> SdkStats {
    with_sdk_state(|state| state.snapshot()).unwrap_or_default()
}

/// Publish `[ptr, ptr+len)` to the host under `name`.
///
/// In standalone mode this validates the stable public inputs and returns a
/// no-op handle so workloads can run unmodified outside the platform.
///
/// # Safety
/// `ptr..ptr+len` must remain valid, mapped, and non-relocating for the life
/// of the returned handle once platform registration is implemented.
pub unsafe fn register_region(
    name: &'static str,
    layout_version: u32,
    ptr: *const u8,
    len: usize,
    flags: RegionFlags,
) -> Result<RegionHandle, RegionError> {
    regions::register_region(name, layout_version, ptr, len, flags)
}

fn sdk_state() -> Option<std::sync::MutexGuard<'static, SdkState>> {
    SDK.get()?._state.lock().ok()
}

impl SdkState {
    fn snapshot(&self) -> SdkStats {
        SdkStats {
            stats_version: 1,
            asserts_passed_total: self.stats.asserts_passed_total,
            asserts_failed_total: self.stats.asserts_failed_total,
            reachable_names: self.stats.reachable_names,
            inject_queries_total: self.stats.inject_queries_total,
        }
    }

    fn intern_name(&mut self, name: &'static str, extra_flags: u8) -> Option<u32> {
        let interned = self.intern.intern(name).ok()?;
        if interned.is_new {
            let ev = detguest_wire::events::EventPayload::NameIntern {
                name_id: interned.id,
                name: name.as_bytes(),
            };
            let _ = self
                ._channel
                .emit_w_event(0, extra_flags, &ev, channel::EventClass::Critical);
        }
        Some(interned.id)
    }

    fn assert_always(&mut self, cond: bool, name: &'static str, details: fmt::Arguments<'_>) {
        let Some(name_id) = self.intern_name(name, 0) else {
            return;
        };
        if cond {
            self.stats.asserts_passed_total = self.stats.asserts_passed_total.saturating_add(1);
            let _ = self.intern.record_assert(name_id, true);
            return;
        }

        self.stats.asserts_failed_total = self.stats.asserts_failed_total.saturating_add(1);
        let Ok(counts) = self.intern.record_assert(name_id, false) else {
            return;
        };
        if counts.fail_count > ASSERT_REPEAT_LIMIT {
            return;
        }

        let details = fmt::format(details);
        let ev = detguest_wire::events::EventPayload::AssertViolation {
            name_id,
            violation_count: counts.fail_count,
            details: details.as_bytes(),
        };
        let _ = self
            ._channel
            .emit_w_event(0, 0, &ev, channel::EventClass::Critical);
    }

    fn expect_reachable(&mut self, name: &'static str) {
        let Some(name_id) = self.intern_name(name, 0) else {
            return;
        };
        let Ok(hits) = self.intern.record_reachable(name_id) else {
            return;
        };
        if hits == 1 {
            self.stats.reachable_names = self.stats.reachable_names.saturating_add(1);
            let ev = detguest_wire::events::EventPayload::Reachable { name_id };
            let _ = self
                ._channel
                .emit_w_event(0, 0, &ev, channel::EventClass::Critical);
        }
    }

    fn declare_reachable(&mut self, name: &'static str) {
        let _ = self.intern_name(name, detguest_wire::record::FLAG_REACHABLE_DECL);
    }

    fn coverage_beacon(&mut self, id: u32) {
        let hit = self.beacons.hit(id);
        if hit.first_hit {
            let ev = detguest_wire::events::EventPayload::Beacon { beacon_id: hit.id };
            let _ = self
                ._channel
                .emit_w_event(0, 0, &ev, channel::EventClass::Droppable);
        }
    }

    fn log_line(&mut self, level: LogLevel, msg: &str) {
        let ev = detguest_wire::events::EventPayload::LogLine {
            stream: detguest_wire::events::log_stream::SDK_USER,
            level: level as u8,
            msg: msg.as_bytes(),
        };
        let _ = self
            ._channel
            .emit_w_event(0, 0, &ev, channel::EventClass::Droppable);
    }

    fn frame_mark(&mut self) {
        self.frame_index = self.frame_index.wrapping_add(1);
        let frame_index = self.frame_index;
        let ev = detguest_wire::events::EventPayload::FrameMark { frame_index };
        let _ = self
            ._channel
            .emit_w_event(0, 0, &ev, channel::EventClass::Critical);
        self._pio.write_frame_counter(frame_index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use detguest_wire::events::{decode_event, encoded_event_len, EventPayload, MAX_LOG_MSG};
    use detguest_wire::header::{
        ChannelHeader, FLAG_WORKLOAD_ATTACHED, OFF_HEADER_FLAGS, OFF_RING_W_DATA, OFF_RING_W_PROD,
        PROTO_VERSION,
    };
    use detguest_wire::record::{RecordHeader, FLAG_REACHABLE_DECL, FLAG_TRUNCATED};
    use std::{
        fs::File,
        io,
        os::fd::{AsRawFd, FromRawFd},
        os::unix::fs::FileExt,
        sync::OnceLock,
    };

    fn fake_pio() -> Result<pio::PioState, InitError> {
        Ok(pio::PioState::for_test())
    }

    fn test_pvpad_words() -> &'static mut [u32; pio::PV_PAD_WORDS] {
        Box::leak(Box::new([0; pio::PV_PAD_WORDS]))
    }

    fn test_channel_file(header: ChannelHeader) -> File {
        let name = std::ffi::CString::new("detguest-sdk-test").unwrap();
        let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
        assert!(
            fd >= 0,
            "memfd_create failed: {}",
            io::Error::last_os_error()
        );
        let file = unsafe { File::from_raw_fd(fd) };
        file.set_len(detguest_wire::header::CHANNEL_SIZE as u64)
            .unwrap();
        let mut bytes = [0u8; detguest_wire::header::OFF_RESERVED];
        header.write_to(&mut bytes).unwrap();
        file.write_all_at(&bytes, 0).unwrap();
        file
    }

    fn test_state(file: &File) -> SdkState {
        SdkState {
            _channel: channel::MappedChannel::map(file.as_raw_fd()).unwrap(),
            _pio: pio::PioState::for_test(),
            intern: intern::InternTable::default(),
            beacons: beacons::BeaconCounters::default(),
            stats: StatsState::default(),
            frame_index: 0,
        }
    }

    fn for_each_ring_w_event(
        file: &File,
        mut f: impl FnMut(usize, RecordHeader, EventPayload<'_>),
    ) {
        let mut prod = [0u8; 4];
        file.read_exact_at(&mut prod, OFF_RING_W_PROD as u64)
            .unwrap();
        let prod = u32::from_le_bytes(prod) as usize;
        let mut bytes = vec![0u8; prod];
        file.read_exact_at(&mut bytes, OFF_RING_W_DATA as u64)
            .unwrap();
        let mut at = 0;
        let mut index = 0;
        while at < bytes.len() {
            let hdr = RecordHeader::read_from(&bytes[at..]).unwrap();
            let len = hdr.len as usize;
            let (_, payload) = decode_event(&bytes[at..at + len]).unwrap();
            f(index, hdr, payload);
            at += len;
            index += 1;
        }
    }

    fn init_for_test<'a>(file: &File, cell: &'a OnceLock<Sdk>) -> Result<&'a Sdk, InitError> {
        let raw = file.as_raw_fd().to_string();
        init_from_channel_fd(Some(std::ffi::OsStr::new(&raw)), cell, fake_pio)
    }

    #[test]
    fn init_without_channel_fd_reports_no_channel() {
        let cell = OnceLock::new();
        let err = init_from_channel_fd(None, &cell, fake_pio).unwrap_err();
        assert!(matches!(err, InitError::NoChannel));
    }

    #[test]
    fn init_with_bad_fd_is_deterministic_error() {
        let cell = OnceLock::new();
        let err = init_from_channel_fd(Some(std::ffi::OsStr::new("not-an-fd")), &cell, fake_pio)
            .unwrap_err();
        assert!(matches!(err, InitError::AgentSocket(_)));
    }

    #[test]
    fn bad_header_magic_is_reported_before_pio_setup() {
        let mut header = ChannelHeader::canonical();
        header.magic ^= 1;
        let file = test_channel_file(header);
        let cell = OnceLock::new();
        let err = init_for_test(&file, &cell).unwrap_err();
        assert!(matches!(
            err,
            InitError::BadChannelHeader { found_magic } if found_magic == header.magic
        ));
    }

    #[test]
    fn protocol_version_mismatch_is_reported() {
        let mut header = ChannelHeader::canonical();
        header.proto_version = PROTO_VERSION + 1;
        let file = test_channel_file(header);
        let cell = OnceLock::new();
        let err = init_for_test(&file, &cell).unwrap_err();
        assert!(matches!(
            err,
            InitError::ProtocolVersionMismatch { guest, channel }
                if guest == PROTO_VERSION && channel == header.proto_version
        ));
    }

    #[test]
    fn valid_init_sets_workload_attached_and_is_idempotent() {
        let file = test_channel_file(ChannelHeader::canonical());
        let cell = OnceLock::new();
        let first = init_for_test(&file, &cell).unwrap() as *const Sdk;
        let second = init_from_channel_fd(None, &cell, fake_pio).unwrap() as *const Sdk;
        assert_eq!(first, second);

        let mut flags = [0u8; 4];
        file.read_exact_at(&mut flags, OFF_HEADER_FLAGS as u64)
            .unwrap();
        let flags = u32::from_le_bytes(flags);
        assert_eq!(flags & FLAG_WORKLOAD_ATTACHED, FLAG_WORKLOAD_ATTACHED);
    }

    #[test]
    fn standalone_calls_are_noops() {
        det_assert_always!(true, "ok", "value={}", 1);
        assert_eq!(inject_point("fault.site"), FaultDecision::Proceed);
        assert_eq!(poll_input(0), 0);
        frame_mark();
        quiesce_check();
        log_line(LogLevel::Info, "hello");
        assert_eq!(stats(), SdkStats::default());
    }

    #[test]
    fn standalone_region_registration_returns_noop_handle() {
        let byte = 7u8;
        let handle = unsafe {
            register_region(
                "region",
                1,
                &byte as *const u8,
                1,
                RegionFlags::HOT | RegionFlags::FRAMEBUFFER,
            )
        }
        .unwrap();
        assert_eq!(handle.region_id(), 0);
        handle.unregister();
    }

    #[test]
    fn user_event_apis_emit_exact_first_hit_sequence() {
        let file = test_channel_file(ChannelHeader::canonical());
        let mut state = test_state(&file);

        state.assert_always(false, "hp.limit", format_args!("hp={} max={}", 12, 10));
        state.expect_reachable("room.ready");
        state.expect_reachable("room.ready");
        state.coverage_beacon(9);
        state.coverage_beacon(9);
        state.log_line(LogLevel::Info, "hello");

        let mut seen = 0;
        for_each_ring_w_event(&file, |index, hdr, payload| {
            seen += 1;
            match (index, payload) {
                (
                    0,
                    EventPayload::NameIntern {
                        name_id,
                        name: b"hp.limit",
                    },
                ) => {
                    assert_eq!(name_id, 1);
                    assert_eq!(hdr.flags, 0);
                }
                (
                    1,
                    EventPayload::AssertViolation {
                        name_id,
                        violation_count,
                        details: b"hp=12 max=10",
                    },
                ) => {
                    assert_eq!(name_id, 1);
                    assert_eq!(violation_count, 1);
                }
                (
                    2,
                    EventPayload::NameIntern {
                        name_id,
                        name: b"room.ready",
                    },
                ) => {
                    assert_eq!(name_id, 2);
                    assert_eq!(hdr.flags, 0);
                }
                (3, EventPayload::Reachable { name_id }) => assert_eq!(name_id, 2),
                (4, EventPayload::Beacon { beacon_id }) => assert_eq!(beacon_id, 9),
                (
                    5,
                    EventPayload::LogLine {
                        stream,
                        level,
                        msg: b"hello",
                    },
                ) => {
                    assert_eq!(stream, detguest_wire::events::log_stream::SDK_USER);
                    assert_eq!(level, LogLevel::Info as u8);
                }
                other => panic!("unexpected event at {index}: {other:?}"),
            }
        });
        assert_eq!(seen, 6);
    }

    #[test]
    fn declare_reachable_marks_name_intern_with_reachable_decl() {
        let file = test_channel_file(ChannelHeader::canonical());
        let mut state = test_state(&file);

        state.declare_reachable("declared");

        let mut seen = 0;
        for_each_ring_w_event(&file, |index, hdr, payload| {
            seen += 1;
            match (index, payload) {
                (
                    0,
                    EventPayload::NameIntern {
                        name_id,
                        name: b"declared",
                    },
                ) => {
                    assert_eq!(name_id, 1);
                    assert_eq!(hdr.flags & FLAG_REACHABLE_DECL, FLAG_REACHABLE_DECL);
                }
                other => panic!("unexpected event at {index}: {other:?}"),
            }
        });
        assert_eq!(seen, 1);
    }

    #[test]
    fn log_line_truncates_at_wire_cap() {
        let file = test_channel_file(ChannelHeader::canonical());
        let mut state = test_state(&file);
        let msg = "x".repeat(MAX_LOG_MSG + 1);

        state.log_line(LogLevel::Info, &msg);

        let mut seen = 0;
        for_each_ring_w_event(&file, |index, hdr, payload| {
            seen += 1;
            match (index, payload) {
                (0, EventPayload::LogLine { msg, .. }) => {
                    assert_eq!(msg.len(), MAX_LOG_MSG);
                    assert_eq!(hdr.flags & FLAG_TRUNCATED, FLAG_TRUNCATED);
                }
                other => panic!("unexpected event at {index}: {other:?}"),
            }
        });
        assert_eq!(seen, 1);
    }

    #[test]
    fn frame_mark_publishes_record_before_frame_counter_write() {
        let file = test_channel_file(ChannelHeader::canonical());
        let words = test_pvpad_words();
        let words_ptr = words.as_mut_ptr();
        let pio = pio::PioState::for_test_with_pvpad(words);
        let mut state = test_state(&file);
        state._pio = pio;

        state.frame_mark();

        let ev = EventPayload::FrameMark { frame_index: 1 };
        let len = encoded_event_len(&ev);
        let mut record = vec![0u8; len];
        file.read_exact_at(&mut record, OFF_RING_W_DATA as u64)
            .unwrap();
        let (hdr, payload) = decode_event(&record).unwrap();
        assert_eq!(hdr.seq, 0);
        assert_eq!(payload, ev);
        let frame = unsafe { words_ptr.add(pio::PVPAD_FRAME_COUNTER_WORD).read() };
        assert_eq!(frame, 1);
    }
}
