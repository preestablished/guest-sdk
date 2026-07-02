//! M4 region-publication acceptance workload (Ms4 plan package 06 §B).
//!
//! A dedicated deterministic workload for the platform-readability
//! acceptance (the m9 fixture is contract-frozen against the hypervisor):
//! publishes `wram` (8 KiB) + `framebuffer` (exactly 229,376 bytes — D7
//! layout_version 1: XRGB8888, 256x224, stride 1024) + `meta` (256 B), then
//! runs a deterministic frame loop that mixes pv-pad input into an
//! accumulator, scatters writes into wram + framebuffer, and reports
//! per-frame state in `meta`:
//!
//! - `meta[0..4]`  = frame index (LE, 0-based index of the frame just done)
//! - `meta[8..16]` = accumulator (LE)
//! - `meta[16..24]` = FNV-1a hash of every input consumed so far (LE)
//!
//! Plain autostart: no `[unit.control]` handshake, no pv-blk — the minimal
//! surface for snapshot/restore readability tests.

use core::ptr::{addr_of_mut, read_volatile, write_volatile};

use detguest_sdk::{self as sdk, RegionFlags};

const WRAM_LEN: usize = 8192;
/// D7 layout_version 1 framebuffer: exactly 229,376 bytes (NOT a power of
/// two — indices use `%`, not a mask).
const FRAMEBUFFER_LEN: usize = 229_376;
const META_LEN: usize = 256;
const WORK_UNITS_PER_FRAME: usize = 4096;

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

static mut WRAM: [u8; WRAM_LEN] = [0; WRAM_LEN];
static mut FRAMEBUFFER: [u8; FRAMEBUFFER_LEN] = [0; FRAMEBUFFER_LEN];
static mut META: [u8; META_LEN] = [0; META_LEN];

fn main() {
    let _ = sdk::init();
    publish_regions();
    run_frame_loop();
}

fn publish_regions() {
    // SAFETY: static byte arrays — mapped for the process lifetime, never
    // moving, satisfying the SDK region registration contract.
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
        // The regions live until power-off by design; dropping a handle
        // unregisters (DEADs) its region, so leak all three deliberately.
        std::mem::forget(wram);
        std::mem::forget(framebuffer);
        std::mem::forget(meta);
    }
}

fn run_frame_loop() -> ! {
    let mut frame = 0u32;
    let mut acc = 0x4d34_0000_0000_0001u64;
    let mut input_hash = FNV_OFFSET_BASIS;
    loop {
        // One pad poll per frame; every consumed input feeds both the
        // accumulator and the input-history hash (hosts recompute the hash
        // from the schedule, warm-up zeros included).
        let input = sdk::poll_input(0);
        for byte in input.to_le_bytes() {
            input_hash = (input_hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME);
        }
        acc = acc.rotate_left(9) ^ (u64::from(input) << 16) ^ 0x00d7_00d7_00d7_00d7;

        for step in 0..WORK_UNITS_PER_FRAME {
            acc = acc
                .rotate_left(7)
                .wrapping_add(u64::from(frame) << 32)
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
        write_frame_meta(frame, acc, input_hash);
        sdk::frame_mark();
        frame = frame.wrapping_add(1);
    }
}

unsafe fn bump_byte(base: *mut u8, index: usize, value: u8) {
    let cell = base.add(index);
    let prev = read_volatile(cell);
    write_volatile(cell, prev.wrapping_add(value).wrapping_add(1));
}

fn write_frame_meta(frame: u32, acc: u64, input_hash: u64) {
    unsafe {
        let meta = addr_of_mut!(META).cast::<u8>();
        for (offset, byte) in frame.to_le_bytes().into_iter().enumerate() {
            write_volatile(meta.add(offset), byte);
        }
        for (offset, byte) in acc.to_le_bytes().into_iter().enumerate() {
            write_volatile(meta.add(8 + offset), byte);
        }
        for (offset, byte) in input_hash.to_le_bytes().into_iter().enumerate() {
            write_volatile(meta.add(16 + offset), byte);
        }
    }
}
