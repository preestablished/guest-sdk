use std::{fmt, ops};

use detguest_wire::manifest::{
    MAX_REGION_NAME, REGION_FLAG_FRAMEBUFFER, REGION_FLAG_HOST_WRITABLE, REGION_FLAG_HOT,
};

/// Region publication flags.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct RegionFlags(u32);

impl RegionFlags {
    /// Host treats contents as a framebuffer.
    pub const FRAMEBUFFER: RegionFlags = RegionFlags(REGION_FLAG_FRAMEBUFFER);
    /// Contents change every step.
    pub const HOT: RegionFlags = RegionFlags(REGION_FLAG_HOT);
    /// Host may write this region; reserved in v1.
    pub const HOST_WRITABLE: RegionFlags = RegionFlags(REGION_FLAG_HOST_WRITABLE);

    /// Empty flag set.
    pub const fn empty() -> RegionFlags {
        RegionFlags(0)
    }

    /// Raw flag bits.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Build from raw bits, preserving unknown future bits.
    pub const fn from_bits_retain(bits: u32) -> RegionFlags {
        RegionFlags(bits)
    }

    /// True when all `other` bits are present.
    pub const fn contains(self, other: RegionFlags) -> bool {
        (self.0 & other.0) == other.0
    }
}

impl fmt::Debug for RegionFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("RegionFlags")
            .field(&format_args!("0x{:08x}", self.0))
            .finish()
    }
}

impl ops::BitOr for RegionFlags {
    type Output = RegionFlags;

    fn bitor(self, rhs: RegionFlags) -> Self::Output {
        RegionFlags(self.0 | rhs.0)
    }
}

impl ops::BitOrAssign for RegionFlags {
    fn bitor_assign(&mut self, rhs: RegionFlags) {
        self.0 |= rhs.0;
    }
}

impl ops::BitAnd for RegionFlags {
    type Output = RegionFlags;

    fn bitand(self, rhs: RegionFlags) -> Self::Output {
        RegionFlags(self.0 & rhs.0)
    }
}

impl ops::BitAndAssign for RegionFlags {
    fn bitand_assign(&mut self, rhs: RegionFlags) {
        self.0 &= rhs.0;
    }
}

/// Errors from region publication.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RegionError {
    /// The manifest has no free region slot.
    ManifestFull,
    /// The region would exceed the manifest extent pool.
    TooManyExtents,
    /// The requested bytes are not present and pinned.
    NotPinned,
    /// Region name exceeds the manifest field.
    NameTooLong,
    /// The agent IPC path is unavailable.
    AgentUnavailable,
}

impl fmt::Display for RegionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegionError::ManifestFull => write!(f, "region manifest is full"),
            RegionError::TooManyExtents => write!(f, "region has too many extents"),
            RegionError::NotPinned => write!(f, "region is not pinned"),
            RegionError::NameTooLong => write!(f, "region name is too long"),
            RegionError::AgentUnavailable => write!(f, "agent is unavailable"),
        }
    }
}

impl std::error::Error for RegionError {}

/// Published region handle.
#[derive(Debug)]
pub struct RegionHandle {
    region_id: u32,
}

impl RegionHandle {
    /// Manifest region slot id.
    pub fn region_id(&self) -> u32 {
        self.region_id
    }

    /// Explicitly unregister this region.
    pub fn unregister(self) {}
}

impl Drop for RegionHandle {
    fn drop(&mut self) {}
}

pub(crate) unsafe fn register_region(
    name: &'static str,
    layout_version: u32,
    ptr: *const u8,
    len: usize,
    flags: RegionFlags,
) -> Result<RegionHandle, RegionError> {
    if name.len() > MAX_REGION_NAME {
        return Err(RegionError::NameTooLong);
    }
    if len != 0 && ptr.is_null() {
        return Err(RegionError::NotPinned);
    }
    let _ = (layout_version, ptr, len, flags);
    Ok(RegionHandle { region_id: 0 })
}
