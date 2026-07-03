//! Read-only pv-blk MMIO device model (determinism-hypervisor ARCH §6.5
//! owns the real device; this repo only mirrors the guest-visible ABI).
//!
//! Semantics follow determinism-hypervisor's dh-devices:
//! `crates/dh-devices/src/blk.rs` (registers, request validation, status
//! codes) and `crates/dh-devices/src/bus.rs` (the bus-served MAGIC/VERSION
//! registers and the 4 KiB window convention). Deliberate divergences from
//! that reference are called out inline; the headline ones:
//!
//! - **Read-only**: `CMD_WRITE`/`CMD_FLUSH` return `STATUS_BAD_REQUEST`
//!   instead of executing. The real device supports writes, but the agent
//!   must never issue them (materialization is reads-only so the pv-blk
//!   overlay stays clean) — a write reaching this stub should be loud.
//! - **No bus-fault fidelity**: the real bus injects a guest fault for
//!   misaligned / non-4/8-byte accesses (`bus.rs` `check_access`). Here
//!   they read as zeros / are ignored — the agent never issues them, and
//!   fault injection is beyond this tier.
//!
//! The model's mutable state rides in [`super::pio::PioState`] (`Clone`, so
//! a `VmSnapshot` carries it verbatim); there is no restore fidelity beyond
//! that because all pv-blk traffic in this tier happens pre-READY.

use detguest_host::GuestMem;

use super::VmHarness;

/// pv-blk MMIO base GPA (cited from the hypervisor's device map).
pub const PVBLK_BASE: u64 = 0xD000_4000;
/// One past the device's 4 KiB window (`bus.rs` `WINDOW_LEN`).
pub const PVBLK_END: u64 = PVBLK_BASE + 0x1000;

/// Bus-served device id (`blk.rs` `DEVICE_ID_PV_BLK`, `bus.rs:89-97`).
pub const DEVICE_ID_PV_BLK: u32 = 0x0005;
/// Bus-served device section version.
pub const PVBLK_VERSION: u32 = 1;

/// Register offsets within the window (`blk.rs` REG_*; MAGIC/VERSION are
/// bus-owned in the reference — this model serves them itself).
pub const REG_MAGIC: u64 = 0x00; // 4B RO
pub const REG_VERSION: u64 = 0x04; // 4B RO
pub const REG_SECTOR: u64 = 0x08; // 8B RW
pub const REG_BUF_GPA: u64 = 0x10; // 8B RW
pub const REG_COUNT: u64 = 0x18; // 4B RW (sectors)
pub const REG_CMD: u64 = 0x1C; // 4B WO: write triggers synchronously
pub const REG_STATUS: u64 = 0x20; // 4B RO

pub const CMD_READ: u32 = 1;
pub const CMD_WRITE: u32 = 2;
pub const CMD_FLUSH: u32 = 3;

pub const STATUS_OK: u32 = 0;
/// Request outside the device (sector range, zero/overflowing count) or an
/// unknown CMD value — and, divergently, any write/flush (module docs).
pub const STATUS_BAD_REQUEST: u32 = 1;
/// Guest buffer access faulted (BUF_GPA range not mapped guest RAM).
pub const STATUS_MEM_FAULT: u32 = 2;

pub const SECTOR_SIZE: usize = 512;

/// The pv-blk device model: an immutable backing image plus the one-deep
/// request register set. Completion is synchronous inside the CMD write's
/// VM exit, exactly like the reference device.
#[derive(Clone)]
pub struct PvBlkModel {
    /// The backing image; capacity truncates to whole sectors
    /// (`blk.rs:114-117` — a trailing partial sector is not addressable).
    backing: Vec<u8>,
    /// Latched SECTOR register.
    pub sector: u64,
    /// Latched BUF_GPA register.
    pub buf_gpa: u64,
    /// Latched COUNT register (sectors).
    pub count: u32,
    /// STATUS register — valid when the CMD write's VM exit returns.
    pub status: u32,
}

impl PvBlkModel {
    /// A fresh device over `backing` (registers zeroed, STATUS_OK).
    pub fn new(backing: Vec<u8>) -> PvBlkModel {
        PvBlkModel {
            backing,
            sector: 0,
            buf_gpa: 0,
            count: 0,
            status: STATUS_OK,
        }
    }

    /// Device capacity in sectors (whole sectors of the backing image).
    pub fn capacity_sectors(&self) -> u64 {
        (self.backing.len() / SECTOR_SIZE) as u64
    }

    /// MMIO read at window offset `off`, filling `data` (raw exit slice —
    /// 4- and 8-byte naturally aligned accesses decode; anything else reads
    /// as zeros, see the module docs on bus-fault fidelity).
    pub fn mmio_read(&self, off: u64, data: &mut [u8]) {
        match (off, data.len()) {
            (REG_MAGIC, 4) => data.copy_from_slice(&DEVICE_ID_PV_BLK.to_le_bytes()),
            (REG_VERSION, 4) => data.copy_from_slice(&PVBLK_VERSION.to_le_bytes()),
            (REG_SECTOR, 8) => data.copy_from_slice(&self.sector.to_le_bytes()),
            (REG_BUF_GPA, 8) => data.copy_from_slice(&self.buf_gpa.to_le_bytes()),
            (REG_COUNT, 4) => data.copy_from_slice(&self.count.to_le_bytes()),
            (REG_STATUS, 4) => data.copy_from_slice(&self.status.to_le_bytes()),
            // CMD is write-only; an 8-byte read at 0x00 (spanning
            // MAGIC+VERSION) is a GuestFault on the real bus (`bus.rs:93`)
            // but reads as zeros here; everything else (unknown offsets,
            // size-mismatched access to known registers) also zeros.
            _ => data.fill(0),
        }
    }

    /// MMIO write at window offset `off` (raw exit slice). A CMD write
    /// executes synchronously against `mem` before this returns.
    /// MAGIC/VERSION/STATUS are read-only: writes to them are ignored, as
    /// are size-mismatched or unknown-offset writes.
    pub fn mmio_write(&mut self, off: u64, data: &[u8], mem: &mut dyn GuestMem) {
        match (off, data.len()) {
            (REG_SECTOR, 8) => self.sector = u64::from_le_bytes(data.try_into().expect("len 8")),
            (REG_BUF_GPA, 8) => self.buf_gpa = u64::from_le_bytes(data.try_into().expect("len 8")),
            (REG_COUNT, 4) => self.count = u32::from_le_bytes(data.try_into().expect("len 4")),
            (REG_CMD, 4) => {
                let cmd = u32::from_le_bytes(data.try_into().expect("len 4"));
                self.status = self.execute(cmd, mem);
            }
            _ => {}
        }
    }

    /// Execute a latched request. DIVERGENCE from `blk.rs`: this model is
    /// read-only, so `CMD_WRITE` and `CMD_FLUSH` fail loudly with
    /// `STATUS_BAD_REQUEST` (the agent must never write the game device —
    /// see the module docs) instead of mutating an overlay.
    fn execute(&mut self, cmd: u32, mem: &mut dyn GuestMem) -> u32 {
        match cmd {
            CMD_READ => self.do_read(mem),
            _ => STATUS_BAD_REQUEST,
        }
    }

    /// Validate the latched request exactly like `blk.rs` `request_range`
    /// (`blk.rs:137-152`); returns (byte offset, byte length) on success.
    fn request_range(&self) -> Result<(usize, usize), u32> {
        if self.count == 0 {
            return Err(STATUS_BAD_REQUEST);
        }
        let end_sector = self
            .sector
            .checked_add(u64::from(self.count))
            .ok_or(STATUS_BAD_REQUEST)?;
        if end_sector > self.capacity_sectors() {
            return Err(STATUS_BAD_REQUEST);
        }
        // In-range per the check above, so both products fit in usize
        // (bounded by backing.len()).
        Ok((
            self.sector as usize * SECTOR_SIZE,
            self.count as usize * SECTOR_SIZE,
        ))
    }

    fn do_read(&self, mem: &mut dyn GuestMem) -> u32 {
        let (off, len) = match self.request_range() {
            Ok(r) => r,
            Err(s) => return s,
        };
        match mem.write(self.buf_gpa, &self.backing[off..off + len]) {
            Ok(()) => STATUS_OK,
            Err(_) => STATUS_MEM_FAULT,
        }
    }
}

/// Whether `addr` falls in the pv-blk window (dispatch predicate for the
/// harness run loop; the loop additionally requires an attached device).
pub fn in_window(addr: u64) -> bool {
    (PVBLK_BASE..PVBLK_END).contains(&addr)
}

/// pv-blk MMIO read (harness run-loop entry; `addr` pre-checked by
/// [`in_window`] and a device is attached, or this fills zeros).
pub fn pvblk_read(h: &mut VmHarness, addr: u64, data: &mut [u8]) {
    match h.pio_state().pvblk.as_ref() {
        Some(blk) => blk.mmio_read(addr - PVBLK_BASE, data),
        None => data.fill(0),
    }
}

/// pv-blk MMIO write (harness run-loop entry). The guest-memory copy for a
/// CMD_READ goes through the harness's [`super::memslot::MemSlot`] — the
/// same guest-RAM access path `detguest-host`'s `Channel` uses.
pub fn pvblk_write(h: &mut VmHarness, addr: u64, data: &[u8]) {
    let mut mem = h.mem();
    if let Some(blk) = h.pio_state().pvblk.as_mut() {
        blk.mmio_write(addr - PVBLK_BASE, data, &mut mem);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use detguest_host::MemError;

    /// Vec-backed guest memory (host-side stand-in for the KVM memslot).
    struct VecMem(Vec<u8>);

    impl GuestMem for VecMem {
        fn read(&self, gpa: u64, buf: &mut [u8]) -> Result<(), MemError> {
            let off = usize::try_from(gpa).map_err(|_| MemError::Overflow)?;
            if off.checked_add(buf.len()).is_none() || off + buf.len() > self.0.len() {
                return Err(MemError::Unmapped {
                    gpa,
                    len: buf.len(),
                });
            }
            buf.copy_from_slice(&self.0[off..off + buf.len()]);
            Ok(())
        }

        fn write(&mut self, gpa: u64, buf: &[u8]) -> Result<(), MemError> {
            let off = usize::try_from(gpa).map_err(|_| MemError::Overflow)?;
            if off.checked_add(buf.len()).is_none() || off + buf.len() > self.0.len() {
                return Err(MemError::Unmapped {
                    gpa,
                    len: buf.len(),
                });
            }
            self.0[off..off + buf.len()].copy_from_slice(buf);
            Ok(())
        }
    }

    /// Patterned backing: byte = (absolute_offset % 251), like the
    /// dh-devices tests — misplacement of any sector is visible.
    fn patterned(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    /// Drive one request through the register protocol, like the agent
    /// would (8-byte SECTOR/BUF_GPA stores, 4-byte COUNT/CMD), and return
    /// the STATUS read back after the CMD write.
    fn request(
        blk: &mut PvBlkModel,
        mem: &mut VecMem,
        cmd: u32,
        sector: u64,
        buf_gpa: u64,
        count: u32,
    ) -> u32 {
        blk.mmio_write(REG_SECTOR, &sector.to_le_bytes(), mem);
        blk.mmio_write(REG_BUF_GPA, &buf_gpa.to_le_bytes(), mem);
        blk.mmio_write(REG_COUNT, &count.to_le_bytes(), mem);
        blk.mmio_write(REG_CMD, &cmd.to_le_bytes(), mem);
        let mut status = [0u8; 4];
        blk.mmio_read(REG_STATUS, &mut status);
        u32::from_le_bytes(status)
    }

    #[test]
    fn magic_and_version_read_as_4_byte_u32s() {
        let blk = PvBlkModel::new(patterned(4 * SECTOR_SIZE));
        let mut v4 = [0u8; 4];
        blk.mmio_read(REG_MAGIC, &mut v4);
        assert_eq!(u32::from_le_bytes(v4), DEVICE_ID_PV_BLK);
        blk.mmio_read(REG_VERSION, &mut v4);
        assert_eq!(u32::from_le_bytes(v4), PVBLK_VERSION);

        // An 8-byte read at 0x00 spans MAGIC+VERSION: GuestFault on the
        // real bus, zeros here (documented divergence).
        let mut v8 = [0xFFu8; 8];
        blk.mmio_read(REG_MAGIC, &mut v8);
        assert_eq!(v8, [0; 8]);
    }

    #[test]
    fn read_at_last_valid_sector_ok_one_past_bad_request() {
        let backing = patterned(4 * SECTOR_SIZE);
        let mut blk = PvBlkModel::new(backing.clone());
        let mut mem = VecMem(vec![0u8; 0x4000]);
        let cap = blk.capacity_sectors();
        assert_eq!(cap, 4);

        // Last valid sector reads OK with the exact tail bytes.
        assert_eq!(
            request(&mut blk, &mut mem, CMD_READ, cap - 1, 0x1000, 1),
            STATUS_OK
        );
        assert_eq!(
            &mem.0[0x1000..0x1000 + SECTOR_SIZE],
            &backing[3 * SECTOR_SIZE..]
        );

        // One past the end: the agent's size discovery keys on exactly
        // this BAD_REQUEST semantic.
        assert_eq!(
            request(&mut blk, &mut mem, CMD_READ, cap, 0x1000, 1),
            STATUS_BAD_REQUEST
        );
        // A count that crosses the end from a valid start also fails.
        assert_eq!(
            request(&mut blk, &mut mem, CMD_READ, cap - 1, 0x1000, 2),
            STATUS_BAD_REQUEST
        );
        // sector + count overflow.
        assert_eq!(
            request(&mut blk, &mut mem, CMD_READ, u64::MAX, 0x1000, 2),
            STATUS_BAD_REQUEST
        );
    }

    #[test]
    fn count_zero_is_bad_request() {
        let mut blk = PvBlkModel::new(patterned(4 * SECTOR_SIZE));
        let mut mem = VecMem(vec![0u8; 0x1000]);
        assert_eq!(
            request(&mut blk, &mut mem, CMD_READ, 0, 0, 0),
            STATUS_BAD_REQUEST
        );
    }

    #[test]
    fn multi_sector_read_returns_exact_backing_bytes() {
        let backing = patterned(8 * SECTOR_SIZE);
        let mut blk = PvBlkModel::new(backing.clone());
        let mut mem = VecMem(vec![0u8; 0x4000]);

        assert_eq!(
            request(&mut blk, &mut mem, CMD_READ, 2, 0x800, 5),
            STATUS_OK
        );
        assert_eq!(
            &mem.0[0x800..0x800 + 5 * SECTOR_SIZE],
            &backing[2 * SECTOR_SIZE..7 * SECTOR_SIZE]
        );
    }

    #[test]
    fn non_512_multiple_backing_truncates_partial_tail() {
        // 2 whole sectors + 100 stray bytes: capacity truncates to 2 and
        // the tail is unaddressable (blk.rs:114-117 semantics).
        let backing = patterned(2 * SECTOR_SIZE + 100);
        let mut blk = PvBlkModel::new(backing.clone());
        let mut mem = VecMem(vec![0u8; 0x2000]);
        assert_eq!(blk.capacity_sectors(), 2);

        assert_eq!(
            request(&mut blk, &mut mem, CMD_READ, 1, 0x1000, 1),
            STATUS_OK
        );
        assert_eq!(
            &mem.0[0x1000..0x1000 + SECTOR_SIZE],
            &backing[SECTOR_SIZE..2 * SECTOR_SIZE]
        );
        assert_eq!(
            request(&mut blk, &mut mem, CMD_READ, 2, 0x1000, 1),
            STATUS_BAD_REQUEST
        );
    }

    #[test]
    fn write_flush_and_unknown_cmds_are_bad_request() {
        let mut blk = PvBlkModel::new(patterned(4 * SECTOR_SIZE));
        let mut mem = VecMem(vec![0u8; 0x1000]);
        // Valid ranges — only the command itself is rejected (read-only
        // divergence; the real device would execute these).
        assert_eq!(
            request(&mut blk, &mut mem, CMD_WRITE, 0, 0x0, 1),
            STATUS_BAD_REQUEST
        );
        assert_eq!(
            request(&mut blk, &mut mem, CMD_FLUSH, 0, 0x0, 1),
            STATUS_BAD_REQUEST
        );
        assert_eq!(
            request(&mut blk, &mut mem, 0xBEEF, 0, 0x0, 1),
            STATUS_BAD_REQUEST
        );
        // The rejected write did not touch guest RAM.
        assert!(mem.0.iter().all(|&b| b == 0));
    }

    #[test]
    fn eight_byte_writes_to_sector_and_buf_gpa_preserve_high_half() {
        let mut blk = PvBlkModel::new(patterned(4 * SECTOR_SIZE));
        let mut mem = VecMem(vec![0u8; 0x1000]);

        blk.mmio_write(
            REG_SECTOR,
            &0xDEAD_BEEF_0000_0002u64.to_le_bytes(),
            &mut mem,
        );
        blk.mmio_write(
            REG_BUF_GPA,
            &0x0000_0001_0000_0800u64.to_le_bytes(),
            &mut mem,
        );
        assert_eq!(blk.sector, 0xDEAD_BEEF_0000_0002);
        assert_eq!(blk.buf_gpa, 0x0000_0001_0000_0800);

        // And they echo back whole on 8-byte reads.
        let mut v8 = [0u8; 8];
        blk.mmio_read(REG_SECTOR, &mut v8);
        assert_eq!(u64::from_le_bytes(v8), 0xDEAD_BEEF_0000_0002);
        blk.mmio_read(REG_BUF_GPA, &mut v8);
        assert_eq!(u64::from_le_bytes(v8), 0x0000_0001_0000_0800);
    }

    #[test]
    fn unmapped_buf_gpa_is_mem_fault() {
        let mut blk = PvBlkModel::new(patterned(4 * SECTOR_SIZE));
        let mut mem = VecMem(vec![0u8; 0x1000]);
        // Buffer entirely outside guest RAM.
        assert_eq!(
            request(&mut blk, &mut mem, CMD_READ, 0, 0x10_0000, 1),
            STATUS_MEM_FAULT
        );
        // Buffer straddling the end of guest RAM.
        assert_eq!(
            request(&mut blk, &mut mem, CMD_READ, 0, 0x0F00, 1),
            STATUS_MEM_FAULT
        );
    }

    #[test]
    fn status_is_read_only_and_odd_accesses_are_inert() {
        let mut blk = PvBlkModel::new(patterned(4 * SECTOR_SIZE));
        let mut mem = VecMem(vec![0u8; 0x1000]);
        // Force a nonzero status, then try to overwrite it.
        assert_eq!(
            request(&mut blk, &mut mem, CMD_READ, 0, 0, 0),
            STATUS_BAD_REQUEST
        );
        blk.mmio_write(REG_STATUS, &STATUS_OK.to_le_bytes(), &mut mem);
        let mut v4 = [0u8; 4];
        blk.mmio_read(REG_STATUS, &mut v4);
        assert_eq!(u32::from_le_bytes(v4), STATUS_BAD_REQUEST);

        // MAGIC/VERSION writes ignored; size-mismatched register access
        // reads zeros / writes are dropped.
        blk.mmio_write(REG_MAGIC, &0u32.to_le_bytes(), &mut mem);
        blk.mmio_write(REG_VERSION, &9u32.to_le_bytes(), &mut mem);
        blk.mmio_write(REG_SECTOR, &7u32.to_le_bytes(), &mut mem); // 4B to an 8B reg
        assert_eq!(blk.sector, 0);
        let mut half = [0xFFu8; 4];
        blk.mmio_read(REG_SECTOR, &mut half);
        assert_eq!(half, [0; 4]);
        // CMD is write-only.
        blk.mmio_read(REG_CMD, &mut v4);
        assert_eq!(u32::from_le_bytes(v4), 0);
    }
}
