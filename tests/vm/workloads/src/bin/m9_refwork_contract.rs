//! Synthetic M9 reference-workload contract binary.
//!
//! This is a deterministic stand-in for the full reference-workload harness
//! while the emulator-side control state machine is still landing. It publishes
//! the minimal fd-3 control handshake, publishes the canonical M9 regions
//! expected by the hypervisor fixture contract, and then enters a deterministic
//! post-Start frame loop so the hypervisor can exercise post-READY landing,
//! frame, and replay gates.

use core::ptr::{addr_of_mut, read_volatile, write_volatile};

use detguest_sdk::{self as sdk, RegionFlags};

const CONTROL_FD: i32 = 3;
const PROTO_VERSION: u64 = 1;
const WRAM_LEN: usize = 4096;
const FRAMEBUFFER_LEN: usize = 4096;
const META_LEN: usize = 256;
const WORK_UNITS_PER_FRAME: usize = 4096;

static mut WRAM: [u8; WRAM_LEN] = [0; WRAM_LEN];
static mut FRAMEBUFFER: [u8; FRAMEBUFFER_LEN] = [0; FRAMEBUFFER_LEN];
static mut META: [u8; META_LEN] = [0; META_LEN];

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
    loop {
        for step in 0..WORK_UNITS_PER_FRAME {
            acc = acc
                .rotate_left(7)
                .wrapping_add((frame as u64) << 32)
                .wrapping_add(step as u64)
                ^ 0xa5a5_5a5a_1020_3040;
            let wram_index = ((acc as usize) ^ step) & (WRAM_LEN - 1);
            let framebuffer_index =
                ((acc.rotate_right(17) as usize) ^ (step << 1)) & (FRAMEBUFFER_LEN - 1);
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
        write_frame_meta(frame, acc);
        sdk::coverage_beacon(frame & 0x3f);
        if frame & 0x0f == 0 {
            sdk::quiesce_check();
        }
        sdk::frame_mark();
        frame = frame.wrapping_add(1);
    }
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
        let _wram = sdk::register_region(
            "wram",
            1,
            addr_of_mut!(WRAM).cast::<u8>(),
            WRAM_LEN,
            RegionFlags::empty(),
        );
        let _framebuffer = sdk::register_region(
            "framebuffer",
            1,
            addr_of_mut!(FRAMEBUFFER).cast::<u8>(),
            FRAMEBUFFER_LEN,
            RegionFlags::FRAMEBUFFER,
        );
        let _meta = sdk::register_region(
            "meta",
            1,
            addr_of_mut!(META).cast::<u8>(),
            META_LEN,
            RegionFlags::empty(),
        );
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
