use std::{ffi::OsStr, io, os::unix::ffi::OsStrExt, os::unix::io::RawFd};

pub(crate) const DETGUEST_CHANNEL_FD_ENV: &str = "DETGUEST_CHANNEL_FD";
const DETGUEST_STANDALONE_PANIC_ENV: &str = "DETGUEST_STANDALONE_PANIC";

pub(crate) fn parse_channel_fd(raw: &OsStr) -> io::Result<RawFd> {
    let raw = std::str::from_utf8(raw.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "channel fd is not UTF-8"))?;
    raw.parse::<RawFd>()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "channel fd is not an integer"))
}

pub(crate) fn standalone_panic_enabled() -> bool {
    std::env::var_os(DETGUEST_STANDALONE_PANIC_ENV)
        .as_deref()
        .is_some_and(|v| v == OsStr::new("1"))
}
