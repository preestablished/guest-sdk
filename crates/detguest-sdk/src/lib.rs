//! detguest-sdk — in-guest instrumentation for deterministic workloads.
//!
//! This crate owns the workload-facing API described in
//! `prompts/docs/guest-sdk/API.md` section 1. The initial scaffold provides the
//! complete public surface and deterministic standalone behavior; platform
//! channel mapping and event production are implemented by the follow-on M3
//! beads.

use std::{fmt, io};

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
    _private: (),
}

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

/// One-time initialization.
///
/// Without `DETGUEST_CHANNEL_FD`, returns [`InitError::NoChannel`] and the rest
/// of the SDK API stays in deterministic standalone mode. The channel mapping
/// implementation is intentionally left for `guest-sdk-m3-sdk-channel-init`.
pub fn init() -> Result<&'static Sdk, InitError> {
    init_from_channel_fd(std::env::var_os(channel::DETGUEST_CHANNEL_FD_ENV).as_deref())
}

fn init_from_channel_fd(raw: Option<&std::ffi::OsStr>) -> Result<&'static Sdk, InitError> {
    let Some(raw) = raw else {
        return Err(InitError::NoChannel);
    };
    let fd = channel::parse_channel_fd(raw).map_err(InitError::AgentSocket)?;
    let _ = fd;
    Err(InitError::AgentSocket(io::Error::new(
        io::ErrorKind::Unsupported,
        "detguest-sdk channel mapping is not implemented in this scaffold",
    )))
}

/// Record a finding if `cond` is false.
///
/// Standalone mode emits no ring traffic. If `DETGUEST_STANDALONE_PANIC=1` is
/// set, a failed assertion panics after evaluating and formatting `details`.
pub fn assert_always(cond: bool, name: &'static str, details: fmt::Arguments<'_>) {
    let _ = intern::valid_name(name);
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
    let _ = intern::valid_name(name);
}

/// Pre-declare a reachability target without recording a hit.
pub fn declare_reachable(name: &'static str) {
    let _ = intern::valid_name(name);
}

/// Cheap coverage counter for the scorer.
pub fn coverage_beacon(id: u32) {
    beacons::coverage_beacon(id);
}

/// Ask the host whether to inject a fault here.
pub fn inject_point(name: &'static str) -> FaultDecision {
    inject::inject_point(name)
}

/// Per-frame controller read from the pv-pad latch.
pub fn poll_input(port: u8) -> u32 {
    pio::poll_input(port)
}

/// Mark a completed emulated frame.
pub fn frame_mark() {
    pio::frame_mark();
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
    let _ = (level, msg);
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
    SdkStats::default()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_without_channel_fd_reports_no_channel() {
        let err = init_from_channel_fd(None).unwrap_err();
        assert!(matches!(err, InitError::NoChannel));
    }

    #[test]
    fn init_with_bad_fd_is_deterministic_error() {
        let err = init_from_channel_fd(Some(std::ffi::OsStr::new("not-an-fd"))).unwrap_err();
        assert!(matches!(err, InitError::AgentSocket(_)));
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
}
