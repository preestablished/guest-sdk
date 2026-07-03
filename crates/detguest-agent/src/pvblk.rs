//! pv-blk game-image materialization (API.md §7.1 `game_source = "pv-blk"`).
//!
//! The agent is a read-only client of the hypervisor's pv-blk MMIO device
//! (determinism-hypervisor owns the device — `dh-devices` `blk.rs` register
//! set + `bus.rs` window convention; this repo only cites the addresses,
//! same as the SDK's pv-pad map). Before driving `LoadGame`, the agent reads
//! the whole game image into [`GAME_IMG_PATH`] on the RAM-backed rootfs so
//! the workload's harness can do an ordinary filesystem read
//! (ARCHITECTURE.md §4.2).
//!
//! Determinism (ARCHITECTURE.md §7): everything here runs single-threaded,
//! pre-Ready, as pure guest↔device MMIO — no entropy, no clocks, no host
//! detcalls. The device ABI exposes no capacity register; size discovery is
//! sequential reads from sector 0 treating the first `STATUS_BAD_REQUEST`
//! as the tail (the only past-the-end signal), narrowed to the exact end
//! with shrinking reads. The command sequence — and therefore the READY
//! icount — is a pure function of the image. Every failure is an
//! `Err(String)` for the §7.3 loud-fault path, never a panic (a PID 1 panic
//! is exit 101, not a boot fault), and never a retry (boot failure must be
//! loud and reproducible).
//!
//! The agent never issues `CMD_WRITE`/`CMD_FLUSH`: writes would dirty the
//! hypervisor's overlay (cluster-sized snapshot sections). After the last
//! read, SECTOR/BUF_GPA/COUNT/STATUS retain that command's values —
//! deterministic device snapshot state at READY.
//!
//! Permitted-unsafe module: /dev/mem mmap of the MMIO window, mlock of the
//! DMA page, and volatile register/DMA-page access.
#![allow(unsafe_code)]

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;

use crate::translate;

/// The materialized game image the agent passes as `LoadGame.dev_path`
/// (unlinked again after the control leg completes — the harness holds its
/// own copy by `GameLoaded`).
pub const GAME_IMG_PATH: &str = "/run/detguest/game.img";

/// pv-blk MMIO window GPA (hypervisor device map; one 4 KiB window).
const PV_BLK_BASE: u64 = 0xD000_4000;
const PV_BLK_WINDOW: usize = 0x1000;

/// Bus-served id register (`bus.rs`: `0x00 MAGIC` = device id; 4-byte read
/// only — an 8-byte read at 0x00 is a bus guest-fault).
const REG_MAGIC: usize = 0x00;
const REG_SECTOR: usize = 0x08; // u64 RW
const REG_BUF_GPA: usize = 0x10; // u64 RW (guest-physical DMA target)
const REG_COUNT: usize = 0x18; // u32 RW (sectors)
const REG_CMD: usize = 0x1C; // u32 WO: the write triggers synchronously
const REG_STATUS: usize = 0x20; // u32 RO

const DEVICE_ID_PV_BLK: u32 = 0x0005;

const CMD_READ: u32 = 1;

const STATUS_OK: u32 = 0;
/// Out-of-range sector/count, zero count, or unknown CMD — the ONLY status
/// that encodes "past the end" (`blk.rs` `request_range`). Anything else
/// mid-read is a real fault, never a size signal.
const STATUS_BAD_REQUEST: u32 = 1;

/// Device sector granularity. `capacity = len_bytes / 512` (truncating): a
/// partial tail in the staged image is unaddressable and therefore
/// **invisible to any guest-side probe** — alignment must be validated where
/// the image is staged.
pub(crate) const SECTOR_SIZE: usize = 512;
const DMA_PAGE_SIZE: usize = 4096;
/// One 4 KiB DMA page per command: a single page is always GPA-contiguous,
/// which the device's linear `BUF_GPA` walk requires.
const SECTORS_PER_PAGE: u32 = (DMA_PAGE_SIZE / SECTOR_SIZE) as u32;

/// Loud fault above this. Budget arithmetic: the game exists twice at peak —
/// the /run file plus the harness's own in-process copy (refwork's loader
/// `fs::read` keeps the Vec as `Cartridge.rom` for the process lifetime) —
/// against 128 MiB guest RAM shared with kernel/agent/channel. 32 MiB caps
/// the pair at 64 MiB; SNES-class carts are far below it. (The file is also
/// unlinked after the control leg, so steady state holds one copy.)
pub(crate) const MAX_GAME_BYTES: u64 = 32 << 20;

/// Streaming-checksum seed, shared with the M9 fixture's readback checksum
/// (`m9_refwork_contract.rs`), generalized from per-sector to whole-stream.
pub(crate) const CHECKSUM_SEED: u64 = 0x7062_6c6b_5f69_6f31;

/// Fold `bytes` (starting at absolute stream offset `stream_off`) into the
/// running checksum: per byte, `sum = sum.rotate_left(5) ^ (byte as
/// u64).wrapping_add(offset)`.
pub(crate) fn checksum_fold(mut sum: u64, bytes: &[u8], stream_off: u64) -> u64 {
    for (i, byte) in bytes.iter().enumerate() {
        sum = sum.rotate_left(5) ^ u64::from(*byte).wrapping_add(stream_off + i as u64);
    }
    sum
}

/// Register-level access to one pv-blk window. Two impls: [`MappedPvBlk`]
/// (the real /dev/mem mapping) and the in-module test fake — so size
/// discovery, the read loop, and checksumming are host-unit-testable (same
/// injectable pattern as `translate`).
pub(crate) trait PvBlkRegs {
    /// 4-byte register read (presence check via `REG_MAGIC`).
    fn read_u32(&mut self, off: usize) -> u32;
    /// Issue `CMD_READ { sector, count }` into the DMA page and return the
    /// device STATUS. On `STATUS_OK` the first `count * SECTOR_SIZE` bytes
    /// of [`Self::dma`] hold the sectors.
    fn read_sectors(&mut self, sector: u64, count: u32) -> u32;
    /// The DMA page contents after the last successful `read_sectors`.
    fn dma(&self) -> &[u8];
}

/// Materialize the game image from pv-blk to `dest`, verify the written
/// file against the device-stream checksum, and return the byte count.
pub fn materialize(dest: &str) -> Result<u64, String> {
    let mut regs = MappedPvBlk::new()?;
    materialize_with(&mut regs, dest)
}

/// [`materialize`] over any register impl (unit-tested with the fake).
fn materialize_with(regs: &mut impl PvBlkRegs, dest: &str) -> Result<u64, String> {
    let magic = regs.read_u32(REG_MAGIC);
    if magic != DEVICE_ID_PV_BLK {
        return Err(format!(
            "pv-blk: no device at GPA {PV_BLK_BASE:#x} (magic {magic:#x}, want {DEVICE_ID_PV_BLK:#x})"
        ));
    }
    if let Some(dir) = std::path::Path::new(dest).parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("pv-blk: create {}: {e}", dir.display()))?;
    }
    let mut file = File::create(dest).map_err(|e| format!("pv-blk: create {dest}: {e}"))?;
    let (bytes, checksum) = read_device(regs, &mut file)?;
    file.flush()
        .map_err(|e| format!("pv-blk: flush {dest}: {e}"))?;
    drop(file);
    verify_file(dest, bytes, checksum)?;
    Ok(bytes)
}

/// Read the whole device forward from sector 0, appending to `out` and
/// folding the stream checksum. Returns `(bytes, checksum)`.
///
/// Size discovery: `SECTORS_PER_PAGE` chunks until the first
/// `STATUS_BAD_REQUEST`, then narrow the tail with counts 4 → 2 → 1 (≤ 3
/// extra commands).
fn read_device(regs: &mut impl PvBlkRegs, out: &mut impl Write) -> Result<(u64, u64), String> {
    fn append(
        dma: &[u8],
        out: &mut dyn Write,
        sector: u64,
        count: u32,
        checksum: &mut u64,
    ) -> Result<(), String> {
        let bytes = &dma[..count as usize * SECTOR_SIZE];
        out.write_all(bytes)
            .map_err(|e| format!("pv-blk: write game image at sector {sector}: {e}"))?;
        *checksum = checksum_fold(*checksum, bytes, sector * SECTOR_SIZE as u64);
        Ok(())
    }

    let mut sector: u64 = 0;
    let mut checksum = CHECKSUM_SEED;

    // Full pages until the tail.
    loop {
        match regs.read_sectors(sector, SECTORS_PER_PAGE) {
            STATUS_OK => {
                append(regs.dma(), out, sector, SECTORS_PER_PAGE, &mut checksum)?;
                sector += u64::from(SECTORS_PER_PAGE);
                if sector * SECTOR_SIZE as u64 > MAX_GAME_BYTES {
                    return Err(format!(
                        "pv-blk: game image exceeds {MAX_GAME_BYTES}-byte cap"
                    ));
                }
            }
            STATUS_BAD_REQUEST => break,
            status => {
                return Err(format!(
                    "pv-blk: read status {status} at sector {sector} (count {SECTORS_PER_PAGE})"
                ))
            }
        }
    }
    // Tail narrowing: binary bits of the remaining [0, 7] sectors.
    for count in [4u32, 2, 1] {
        match regs.read_sectors(sector, count) {
            STATUS_OK => {
                append(regs.dma(), out, sector, count, &mut checksum)?;
                sector += u64::from(count);
            }
            STATUS_BAD_REQUEST => {}
            status => {
                return Err(format!(
                    "pv-blk: read status {status} at sector {sector} (count {count})"
                ))
            }
        }
    }
    if sector == 0 {
        return Err("pv-blk: game device is empty (0 sectors)".to_string());
    }
    Ok((sector * SECTOR_SIZE as u64, checksum))
}

/// Verify pass: re-read the **materialized file** — the artifact the
/// harness will actually consume — and compare length + checksum against
/// the device stream. Catches short/garbled file writes and
/// checksum-offset bookkeeping bugs (a device re-read would only
/// re-observe a deterministic device).
fn verify_file(dest: &str, bytes: u64, checksum: u64) -> Result<(), String> {
    let mut file = File::open(dest).map_err(|e| format!("pv-blk: reopen {dest}: {e}"))?;
    let mut buf = [0u8; DMA_PAGE_SIZE];
    let mut len: u64 = 0;
    let mut sum = CHECKSUM_SEED;
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("pv-blk: re-read {dest}: {e}"))?;
        if n == 0 {
            break;
        }
        sum = checksum_fold(sum, &buf[..n], len);
        len += n as u64;
    }
    if len != bytes {
        return Err(format!(
            "pv-blk: materialized file is {len} bytes, device stream was {bytes}"
        ));
    }
    if sum != checksum {
        return Err(format!(
            "pv-blk: materialized file checksum drift ({sum:#x} != {checksum:#x})"
        ));
    }
    Ok(())
}

/// One page-aligned DMA target. mlocked and translated once; a single page
/// is always GPA-contiguous. Static (guest .bss): with `norandmaps` its GPA
/// is deterministic across boots.
#[repr(align(4096))]
struct DmaPage {
    /// Accessed only through raw pointers (the device DMAs into it).
    _bytes: [u8; DMA_PAGE_SIZE],
}

static mut DMA_PAGE: DmaPage = DmaPage {
    _bytes: [0; DMA_PAGE_SIZE],
};

/// The real register impl: /dev/mem mapping of the MMIO window
/// (CONFIG_DEVMEM=y, STRICT_DEVMEM off — pinned in image/kernel.config) +
/// the mlocked static DMA page. Same shape as the M9 fixture's
/// `PvBlkClient` and the SDK's `map_pv_pad`, with `Err` instead of panics.
struct MappedPvBlk {
    mmio: *mut u8,
    /// Keeps the /dev/mem fd alive for the mapping's lifetime (not strictly
    /// required by mmap semantics, but explicit).
    _file: File,
    buf_gpa: u64,
    /// Host-side copy of the DMA page after each successful read (volatile
    /// copy out of the static, so callers never alias the DMA target).
    page: [u8; DMA_PAGE_SIZE],
}

impl MappedPvBlk {
    fn new() -> Result<MappedPvBlk, String> {
        let base = core::ptr::addr_of_mut!(DMA_PAGE).cast::<u8>();
        // SAFETY: zeroing + mlocking + touching the module-owned static DMA
        // page; single-threaded agent, sole user of this static.
        unsafe {
            for i in 0..DMA_PAGE_SIZE {
                core::ptr::write_volatile(base.add(i), 0);
            }
            if libc::mlock(base.cast(), DMA_PAGE_SIZE) != 0 {
                return Err(format!(
                    "pv-blk: mlock DMA page: {}",
                    std::io::Error::last_os_error()
                ));
            }
        }
        let pagemap =
            translate::open_pagemap().map_err(|e| format!("pv-blk: open pagemap: {e}"))?;
        let buf_gpa = translate::gva_to_gpa(&pagemap, base as u64)
            .map_err(|e| format!("pv-blk: DMA page GPA translation: {e:?}"))?;

        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_SYNC)
            .open("/dev/mem")
            .map_err(|e| format!("pv-blk: open /dev/mem: {e}"))?;
        // SAFETY: mapping one 4 KiB device window from /dev/mem; unmapped in
        // Drop. The window GPA is the hypervisor's device map constant.
        let ptr = unsafe {
            libc::mmap(
                core::ptr::null_mut(),
                PV_BLK_WINDOW,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                PV_BLK_BASE as libc::off_t,
            )
        };
        if ptr == libc::MAP_FAILED {
            return Err(format!(
                "pv-blk: mmap window at {PV_BLK_BASE:#x}: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(MappedPvBlk {
            mmio: ptr.cast::<u8>(),
            _file: file,
            buf_gpa,
            page: [0; DMA_PAGE_SIZE],
        })
    }

    fn write_u32(&mut self, off: usize, value: u32) {
        // SAFETY: naturally aligned volatile store inside the mapped window.
        unsafe { core::ptr::write_volatile(self.mmio.add(off).cast::<u32>(), value) }
    }

    fn write_u64(&mut self, off: usize, value: u64) {
        // SAFETY: naturally aligned volatile store inside the mapped window.
        unsafe { core::ptr::write_volatile(self.mmio.add(off).cast::<u64>(), value) }
    }
}

impl PvBlkRegs for MappedPvBlk {
    fn read_u32(&mut self, off: usize) -> u32 {
        // SAFETY: naturally aligned volatile load inside the mapped window.
        unsafe { core::ptr::read_volatile(self.mmio.add(off).cast::<u32>()) }
    }

    fn read_sectors(&mut self, sector: u64, count: u32) -> u32 {
        self.write_u64(REG_SECTOR, sector);
        self.write_u64(REG_BUF_GPA, self.buf_gpa);
        self.write_u32(REG_COUNT, count);
        self.write_u32(REG_CMD, CMD_READ);
        let status = self.read_u32(REG_STATUS);
        if status == STATUS_OK {
            let base = core::ptr::addr_of!(DMA_PAGE).cast::<u8>();
            for (i, slot) in self
                .page
                .iter_mut()
                .take(count as usize * SECTOR_SIZE)
                .enumerate()
            {
                // SAFETY: volatile read of the DMA target the device just
                // wrote (host-side write is not visible to the compiler).
                *slot = unsafe { core::ptr::read_volatile(base.add(i)) };
            }
        }
        status
    }

    fn dma(&self) -> &[u8] {
        &self.page
    }
}

impl Drop for MappedPvBlk {
    fn drop(&mut self) {
        // SAFETY: unmapping exactly the window mapped in `new`.
        unsafe {
            libc::munmap(self.mmio.cast(), PV_BLK_WINDOW);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `game_materialization` VM test's 32 KiB pattern
    /// (`tests/vm/tests/game_materialization.rs` and the `game-load-check`
    /// workload regenerate it independently — the formula is the contract).
    fn test_pattern(len: usize) -> Vec<u8> {
        (0..len).map(|i| ((i * 7) ^ (i >> 8)) as u8).collect()
    }

    /// Pinned golden: checksum of the 32 KiB test pattern. Asserted again in
    /// the VM tier so a drifted reimplementation of the checksum (the
    /// crates don't link) fails here, at the cheap tier, first.
    const GAME_MAT_PATTERN_CHECKSUM: u64 = 0x59ac_17a5_2dff_da9c;

    struct FakePvBlk {
        image: Vec<u8>,
        magic: u32,
        page: [u8; DMA_PAGE_SIZE],
        /// Injected per-sector statuses (sector → status) — a request whose
        /// range covers the sector returns that status.
        overrides: Vec<(u64, u32)>,
    }

    impl FakePvBlk {
        fn new(image: Vec<u8>) -> FakePvBlk {
            FakePvBlk {
                image,
                magic: DEVICE_ID_PV_BLK,
                page: [0; DMA_PAGE_SIZE],
                overrides: Vec::new(),
            }
        }

        fn capacity(&self) -> u64 {
            (self.image.len() / SECTOR_SIZE) as u64
        }
    }

    impl PvBlkRegs for FakePvBlk {
        fn read_u32(&mut self, off: usize) -> u32 {
            match off {
                REG_MAGIC => self.magic,
                _ => 0,
            }
        }

        fn read_sectors(&mut self, sector: u64, count: u32) -> u32 {
            if count == 0 {
                return STATUS_BAD_REQUEST;
            }
            let Some(end) = sector.checked_add(u64::from(count)) else {
                return STATUS_BAD_REQUEST;
            };
            if end > self.capacity() {
                return STATUS_BAD_REQUEST;
            }
            if let Some(&(_, status)) = self
                .overrides
                .iter()
                .find(|(s, _)| (sector..end).contains(s))
            {
                return status;
            }
            let at = sector as usize * SECTOR_SIZE;
            let len = count as usize * SECTOR_SIZE;
            self.page[..len].copy_from_slice(&self.image[at..at + len]);
            STATUS_OK
        }

        fn dma(&self) -> &[u8] {
            &self.page
        }
    }

    fn temp_path(tag: &str) -> String {
        let dir = std::env::temp_dir().join(format!("pvblk-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(tag).to_str().unwrap().to_string()
    }

    #[test]
    fn checksum_matches_pinned_golden() {
        let sum = checksum_fold(CHECKSUM_SEED, &test_pattern(32768), 0);
        assert_eq!(sum, GAME_MAT_PATTERN_CHECKSUM);
        // Folding in chunks with stream offsets must equal one-shot folding.
        let pattern = test_pattern(32768);
        let mut chunked = CHECKSUM_SEED;
        for (i, chunk) in pattern.chunks(4096).enumerate() {
            chunked = checksum_fold(chunked, chunk, (i * 4096) as u64);
        }
        assert_eq!(chunked, GAME_MAT_PATTERN_CHECKSUM);
    }

    #[test]
    fn size_discovery_exact_at_chunk_boundaries() {
        // Off-by-one country for the 8-sector chunks + 4/2/1 tail narrowing.
        for sectors in [1usize, 7, 8, 9, 63, 64, 65, 4096] {
            let image: Vec<u8> = (0..sectors * SECTOR_SIZE)
                .map(|i| ((i * 31) ^ (i >> 7)) as u8)
                .collect();
            let mut fake = FakePvBlk::new(image.clone());
            let mut out = Vec::new();
            let (bytes, checksum) = read_device(&mut fake, &mut out).unwrap();
            assert_eq!(bytes, (sectors * SECTOR_SIZE) as u64, "{sectors} sectors");
            assert_eq!(out, image, "{sectors} sectors: byte-exact");
            assert_eq!(checksum, checksum_fold(CHECKSUM_SEED, &image, 0));
        }
    }

    #[test]
    fn empty_device_is_a_loud_fault() {
        // 0 bytes and a sub-sector tail both present 0 addressable sectors.
        for len in [0usize, 100] {
            let mut fake = FakePvBlk::new(vec![0xAB; len]);
            let err = read_device(&mut fake, &mut Vec::new()).unwrap_err();
            assert!(err.contains("empty"), "{err}");
        }
    }

    #[test]
    fn over_cap_image_is_a_loud_fault() {
        let mut fake = FakePvBlk::new(vec![0u8; MAX_GAME_BYTES as usize + DMA_PAGE_SIZE]);
        let err = read_device(&mut fake, &mut Vec::new()).unwrap_err();
        assert!(err.contains("cap"), "{err}");
    }

    #[test]
    fn exactly_cap_image_is_allowed() {
        let mut fake = FakePvBlk::new(vec![0x5A; MAX_GAME_BYTES as usize]);
        let (bytes, _) = read_device(&mut fake, &mut Vec::new()).unwrap();
        assert_eq!(bytes, MAX_GAME_BYTES);
    }

    #[test]
    fn wrong_magic_names_the_device_and_both_magics() {
        let mut fake = FakePvBlk::new(test_pattern(SECTOR_SIZE));
        fake.magic = 0;
        let err = materialize_with(&mut fake, &temp_path("wrong-magic")).unwrap_err();
        assert!(err.contains("pv-blk"), "{err}");
        assert!(err.contains("0x0") && err.contains("0x5"), "{err}");
    }

    #[test]
    fn mid_read_host_io_names_status_and_sector_not_size() {
        // HOST_IO (0xFE) at sector 3 — must be a hard fault naming the
        // status, never treated as the past-the-end size signal.
        let mut fake = FakePvBlk::new(test_pattern(16 * SECTOR_SIZE));
        fake.overrides.push((3, 0xFE));
        let err = read_device(&mut fake, &mut Vec::new()).unwrap_err();
        assert!(err.contains("status 254"), "{err}");
        assert!(err.contains("sector 0"), "{err}");
    }

    #[test]
    fn materialize_is_byte_exact_and_verified() {
        let image = test_pattern(32768);
        let mut fake = FakePvBlk::new(image.clone());
        let dest = temp_path("byte-exact");
        let bytes = materialize_with(&mut fake, &dest).unwrap();
        assert_eq!(bytes, 32768);
        assert_eq!(std::fs::read(&dest).unwrap(), image);
    }

    #[test]
    fn verify_pass_catches_a_corrupted_file() {
        // Negative control (a materializer that skips verification could
        // not fail this): corrupt the written file, then run the verify
        // pass the way materialize_with would.
        let image = test_pattern(4 * SECTOR_SIZE);
        let mut fake = FakePvBlk::new(image);
        let dest = temp_path("drift");
        let mut file = File::create(&dest).unwrap();
        let (bytes, checksum) = read_device(&mut fake, &mut file).unwrap();
        drop(file);
        verify_file(&dest, bytes, checksum).unwrap();

        let mut corrupted = std::fs::read(&dest).unwrap();
        corrupted[513] ^= 0xFF;
        std::fs::write(&dest, &corrupted).unwrap();
        let err = verify_file(&dest, bytes, checksum).unwrap_err();
        assert!(err.contains("checksum drift"), "{err}");

        // A short file is named as a length mismatch, not a checksum drift.
        std::fs::write(&dest, &corrupted[..SECTOR_SIZE]).unwrap();
        let err = verify_file(&dest, bytes, checksum).unwrap_err();
        assert!(err.contains("bytes"), "{err}");
    }
}
