//! Synthetic M9 reference-workload contract binary.
//!
//! This is a deterministic stand-in for the full reference-workload harness
//! while the emulator-side control state machine is still landing. It publishes
//! the minimal fd-3 control handshake, publishes the canonical M9 regions
//! expected by the hypervisor fixture contract, and then enters a deterministic
//! post-Start frame loop so the hypervisor can exercise post-READY landing,
//! frame, and replay gates.

use core::ptr::{addr_of_mut, read_volatile, write_volatile, NonNull};
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileExt, OpenOptionsExt};

use detguest_sdk::{self as sdk, RegionFlags};

const CONTROL_FD: i32 = 3;
const PROTO_VERSION: u64 = 1;
const WRAM_LEN: usize = 4096;
// D7 layout_version 1 contract (determinism-hypervisor 5698d7e): geometry
// derives from layout_version — XRGB8888, 256x224, stride 1024, EXACTLY
// 229,376 bytes. Anything else is rejected with FailedPrecondition. NOT a
// power of two: framebuffer indices use `%`, not a mask.
const FRAMEBUFFER_LEN: usize = 229_376;
const META_LEN: usize = 256;
const SECTOR_SIZE: usize = 512;
const WORK_UNITS_PER_FRAME: usize = 4096;
const PVBLK_TEST_SECTOR: u64 = 8;
const META_IO_MAGIC_OFF: usize = 32;
const META_IO_FRAME_OFF: usize = 40;
const META_IO_CHECKSUM_OFF: usize = 48;

const PV_BLK_BASE: libc::off_t = 0xD000_4000;
const PV_BLK_SIZE: usize = 0x1000;
const PV_BLK_REG_SECTOR: usize = 0x08;
const PV_BLK_REG_BUF_GPA: usize = 0x10;
const PV_BLK_REG_COUNT: usize = 0x18;
const PV_BLK_REG_CMD: usize = 0x1C;
const PV_BLK_REG_STATUS: usize = 0x20;
const PV_BLK_CMD_READ: u32 = 1;
const PV_BLK_CMD_WRITE: u32 = 2;
const PV_BLK_CMD_FLUSH: u32 = 3;
const PV_BLK_STATUS_OK: u32 = 0;

const PM_PRESENT: u64 = 1 << 63;
const PM_SWAPPED: u64 = 1 << 62;
const PM_PFN_MASK: u64 = (1 << 55) - 1;

static mut WRAM: [u8; WRAM_LEN] = [0; WRAM_LEN];
static mut FRAMEBUFFER: [u8; FRAMEBUFFER_LEN] = [0; FRAMEBUFFER_LEN];
static mut META: [u8; META_LEN] = [0; META_LEN];

#[repr(align(4096))]
struct DiskBuffer {
    _bytes: [u8; SECTOR_SIZE],
}

static mut DISK_BUF: DiskBuffer = DiskBuffer {
    _bytes: [0; SECTOR_SIZE],
};

fn main() {
    let _ = sdk::init();
    drive_control();
    publish_regions();
    send_datagram(&[0x08, 0x00]); // Ready { frame: 0 }
    expect_start();
    run_frame_loop();
}

fn run_frame_loop() -> ! {
    let mut frame = 0u32;
    let mut acc = 0x4d39_0000_0000_0001u64;
    let mut pvblk = PvBlkClient::new();
    let mut io_checksum = None;
    loop {
        for step in 0..WORK_UNITS_PER_FRAME {
            acc = acc
                .rotate_left(7)
                .wrapping_add((frame as u64) << 32)
                .wrapping_add(step as u64)
                ^ 0xa5a5_5a5a_1020_3040;
            let wram_index = ((acc as usize) ^ step) & (WRAM_LEN - 1);
            let framebuffer_index =
                ((acc.rotate_right(17) as usize) ^ (step << 1)) % FRAMEBUFFER_LEN;
            let meta_index = (step ^ frame as usize) & (META_LEN - 1);
            unsafe {
                bump_byte(addr_of_mut!(WRAM).cast::<u8>(), wram_index, acc as u8);
                bump_byte(
                    addr_of_mut!(FRAMEBUFFER).cast::<u8>(),
                    framebuffer_index,
                    acc.rotate_right(8) as u8,
                );
                bump_byte(
                    addr_of_mut!(META).cast::<u8>(),
                    meta_index,
                    acc.rotate_right(16) as u8,
                );
            }
        }
        if frame == 0 {
            let checksum = pvblk.write_read_once(frame, acc);
            io_checksum = Some(checksum);
        }
        write_frame_meta(frame, acc);
        if let Some(checksum) = io_checksum {
            write_io_meta(0, checksum);
        }
        sdk::coverage_beacon(frame & 0x3f);
        if frame & 0x0f == 0 {
            sdk::quiesce_check();
        }
        sdk::frame_mark();
        frame = frame.wrapping_add(1);
    }
}

struct PvBlkClient {
    mmio: NonNull<u8>,
    buf_gpa: u64,
}

impl PvBlkClient {
    fn new() -> PvBlkClient {
        let buf = addr_of_mut!(DISK_BUF).cast::<u8>();
        unsafe {
            for i in 0..SECTOR_SIZE {
                write_volatile(buf.add(i), 0);
            }
            if libc::mlock(buf.cast(), SECTOR_SIZE) != 0 {
                panic!(
                    "mlock pv-blk buffer failed: {}",
                    std::io::Error::last_os_error()
                );
            }
        }
        let buf_gpa = gva_to_gpa(buf as u64);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_SYNC)
            .open("/dev/mem")
            .expect("open /dev/mem for pv-blk");
        let ptr = unsafe {
            libc::mmap(
                core::ptr::null_mut(),
                PV_BLK_SIZE,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                PV_BLK_BASE,
            )
        };
        if ptr == libc::MAP_FAILED {
            panic!("mmap pv-blk failed: {}", std::io::Error::last_os_error());
        }
        PvBlkClient {
            mmio: NonNull::new(ptr.cast::<u8>()).expect("mmap never returns null on success"),
            buf_gpa,
        }
    }

    fn write_read_once(&mut self, frame: u32, acc: u64) -> u64 {
        let expected = fill_disk_pattern(frame, acc);
        self.command(PV_BLK_CMD_WRITE, PVBLK_TEST_SECTOR, 1);
        zero_disk_buffer();
        self.command(PV_BLK_CMD_READ, PVBLK_TEST_SECTOR, 1);
        let actual = checksum_disk_buffer();
        assert_eq!(actual, expected, "pv-blk readback checksum drift");
        self.command(PV_BLK_CMD_FLUSH, PVBLK_TEST_SECTOR, 1);
        actual
    }

    fn command(&mut self, cmd: u32, sector: u64, count: u32) {
        self.write_u64(PV_BLK_REG_SECTOR, sector);
        self.write_u64(PV_BLK_REG_BUF_GPA, self.buf_gpa);
        self.write_u32(PV_BLK_REG_COUNT, count);
        self.write_u32(PV_BLK_REG_CMD, cmd);
        let status = self.read_u32(PV_BLK_REG_STATUS);
        assert_eq!(status, PV_BLK_STATUS_OK, "pv-blk command {cmd} failed");
    }

    fn read_u32(&self, offset: usize) -> u32 {
        unsafe { read_volatile(self.mmio.as_ptr().add(offset).cast::<u32>()) }
    }

    fn write_u32(&mut self, offset: usize, value: u32) {
        unsafe {
            write_volatile(self.mmio.as_ptr().add(offset).cast::<u32>(), value);
        }
    }

    fn write_u64(&mut self, offset: usize, value: u64) {
        unsafe {
            write_volatile(self.mmio.as_ptr().add(offset).cast::<u64>(), value);
        }
    }
}

impl Drop for PvBlkClient {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.mmio.as_ptr().cast(), PV_BLK_SIZE);
        }
    }
}

fn fill_disk_pattern(frame: u32, acc: u64) -> u64 {
    let mut checksum = 0x7062_6c6b_5f69_6f31u64;
    let buf = addr_of_mut!(DISK_BUF).cast::<u8>();
    for i in 0..SECTOR_SIZE {
        let byte = (acc.rotate_left((i & 63) as u32) as u8)
            .wrapping_add(frame as u8)
            .wrapping_add((i as u8).wrapping_mul(17));
        unsafe {
            write_volatile(buf.add(i), byte);
        }
        checksum = checksum.rotate_left(5) ^ u64::from(byte).wrapping_add(i as u64);
    }
    checksum
}

fn zero_disk_buffer() {
    let buf = addr_of_mut!(DISK_BUF).cast::<u8>();
    for i in 0..SECTOR_SIZE {
        unsafe {
            write_volatile(buf.add(i), 0);
        }
    }
}

fn checksum_disk_buffer() -> u64 {
    let mut checksum = 0x7062_6c6b_5f69_6f31u64;
    let buf = addr_of_mut!(DISK_BUF).cast::<u8>();
    for i in 0..SECTOR_SIZE {
        let byte = unsafe { read_volatile(buf.add(i)) };
        checksum = checksum.rotate_left(5) ^ u64::from(byte).wrapping_add(i as u64);
    }
    checksum
}

fn gva_to_gpa(vaddr: u64) -> u64 {
    let pagemap = File::open("/proc/self/pagemap").expect("open /proc/self/pagemap");
    let mut entry = [0u8; 8];
    pagemap
        .read_exact_at(&mut entry, (vaddr / 4096) * 8)
        .expect("read pagemap entry");
    let raw = u64::from_le_bytes(entry);
    assert_eq!(raw & PM_SWAPPED, 0, "pv-blk buffer is swapped");
    assert_ne!(raw & PM_PRESENT, 0, "pv-blk buffer is not present");
    let pfn = raw & PM_PFN_MASK;
    assert_ne!(pfn, 0, "pagemap hid pv-blk buffer PFN");
    (pfn << 12) + (vaddr & 0xFFF)
}

fn drive_control() {
    let hello = recv_datagram();
    let mut cur = Cursor::new(&hello);
    assert_eq!(cur.varint(), 0, "expected Hello");
    assert_eq!(cur.varint(), PROTO_VERSION, "unexpected proto_version");
    assert!(cur.is_empty(), "trailing Hello bytes");
    send_hello_ack();

    let load = recv_datagram();
    let mut cur = Cursor::new(&load);
    assert_eq!(cur.varint(), 1, "expected LoadGame");
    let dev_path = cur.string();
    assert_eq!(dev_path, "/dev/vdb", "unexpected game device path");
    assert!(cur.is_empty(), "trailing LoadGame bytes");
    sdk::log_line(sdk::LogLevel::Info, "LoadGame /dev/vdb accepted");
    send_game_loaded();
}

fn expect_start() {
    let start = recv_datagram();
    assert_eq!(start, [0x02], "expected Start");
}

fn send_hello_ack() {
    let mut out = Vec::new();
    push_varint(&mut out, 5);
    push_varint(&mut out, PROTO_VERSION);
    push_bytes(&mut out, b"m9-contract");
    push_bytes(&mut out, b"0.1.0");
    send_datagram(&out);
}

fn send_game_loaded() {
    let mut out = vec![0x06];
    out.extend_from_slice(&[0u8; 32]);
    push_bytes(&mut out, b"synthetic");
    push_varint(&mut out, 0);
    send_datagram(&out);
}

fn recv_datagram() -> Vec<u8> {
    let mut buf = [0u8; 4096];
    let n = unsafe { libc::recv(CONTROL_FD, buf.as_mut_ptr().cast(), buf.len(), 0) };
    if n <= 0 {
        panic!(
            "recv control datagram failed: {}",
            std::io::Error::last_os_error()
        );
    }
    buf[..n as usize].to_vec()
}

fn send_datagram(bytes: &[u8]) {
    let n = unsafe {
        libc::send(
            CONTROL_FD,
            bytes.as_ptr().cast(),
            bytes.len(),
            libc::MSG_NOSIGNAL,
        )
    };
    if n < 0 {
        panic!(
            "send control datagram failed: {}",
            std::io::Error::last_os_error()
        );
    }
    assert_eq!(n as usize, bytes.len(), "short control datagram write");
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

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Cursor<'a> {
        Cursor { bytes, at: 0 }
    }

    fn is_empty(&self) -> bool {
        self.at == self.bytes.len()
    }

    fn string(&mut self) -> String {
        let len = self.varint() as usize;
        let bytes = self.take(len);
        std::str::from_utf8(bytes)
            .expect("control string utf8")
            .to_string()
    }

    fn varint(&mut self) -> u64 {
        let mut value = 0u64;
        let mut shift = 0;
        loop {
            let byte = self.take(1)[0];
            value |= ((byte & 0x7F) as u64) << shift;
            if byte & 0x80 == 0 {
                return value;
            }
            shift += 7;
            assert!(shift < 64, "control varint overflow");
        }
    }

    fn take(&mut self, n: usize) -> &'a [u8] {
        let end = self.at.checked_add(n).expect("control cursor overflow");
        assert!(end <= self.bytes.len(), "truncated control datagram");
        let out = &self.bytes[self.at..end];
        self.at = end;
        out
    }
}

fn publish_regions() {
    // SAFETY: these static byte arrays are mapped for the process lifetime and
    // never move, satisfying the SDK region registration contract.
    unsafe {
        let wram = sdk::register_region(
            "wram",
            1,
            addr_of_mut!(WRAM).cast::<u8>(),
            WRAM_LEN,
            RegionFlags::empty(),
        )
        .expect("register wram");
        let framebuffer = sdk::register_region(
            "framebuffer",
            1,
            addr_of_mut!(FRAMEBUFFER).cast::<u8>(),
            FRAMEBUFFER_LEN,
            RegionFlags::FRAMEBUFFER,
        )
        .expect("register framebuffer");
        let meta = sdk::register_region(
            "meta",
            1,
            addr_of_mut!(META).cast::<u8>(),
            META_LEN,
            RegionFlags::empty(),
        )
        .expect("register meta");
        // Dropping a handle unregisters (DEADs) its region; these live until
        // power-off by design, so leak all three deliberately.
        std::mem::forget(wram);
        std::mem::forget(framebuffer);
        std::mem::forget(meta);
    }
}

unsafe fn bump_byte(base: *mut u8, index: usize, value: u8) {
    let cell = base.add(index);
    let prev = read_volatile(cell);
    write_volatile(cell, prev.wrapping_add(value).wrapping_add(1));
}

fn write_frame_meta(frame: u32, acc: u64) {
    unsafe {
        let meta = addr_of_mut!(META).cast::<u8>();
        for (offset, byte) in frame.to_le_bytes().into_iter().enumerate() {
            write_volatile(meta.add(offset), byte);
        }
        for (offset, byte) in acc.to_le_bytes().into_iter().enumerate() {
            write_volatile(meta.add(8 + offset), byte);
        }
    }
}

fn write_io_meta(frame: u32, checksum: u64) {
    unsafe {
        let meta = addr_of_mut!(META).cast::<u8>();
        for (offset, byte) in b"PVBLKIO1".iter().copied().enumerate() {
            write_volatile(meta.add(META_IO_MAGIC_OFF + offset), byte);
        }
        for (offset, byte) in frame.to_le_bytes().into_iter().enumerate() {
            write_volatile(meta.add(META_IO_FRAME_OFF + offset), byte);
        }
        for (offset, byte) in checksum.to_le_bytes().into_iter().enumerate() {
            write_volatile(meta.add(META_IO_CHECKSUM_OFF + offset), byte);
        }
    }
}
