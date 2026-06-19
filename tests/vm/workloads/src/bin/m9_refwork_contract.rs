//! Synthetic M9 reference-workload contract binary.
//!
//! This is a deterministic stand-in for the full reference-workload harness
//! while the emulator-side control state machine is still landing. It publishes
//! the minimal fd-3 control handshake, publishes the canonical M9 regions
//! expected by the hypervisor fixture contract, and then parks forever so the
//! agent can gate Ready on those registrations.

use std::thread;

use detguest_sdk::{self as sdk, RegionFlags};

const CONTROL_FD: i32 = 3;
const PROTO_VERSION: u64 = 1;

static WRAM: [u8; 4096] = [0; 4096];
static FRAMEBUFFER: [u8; 4096] = [0; 4096];
static META: [u8; 256] = [0; 256];

fn main() {
    let _ = sdk::init();
    drive_control();
    publish_regions();
    send_datagram(&[0x08, 0x00]); // Ready { frame: 0 }
    expect_start();
    loop {
        thread::park();
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
        let _wram =
            sdk::register_region("wram", 1, WRAM.as_ptr(), WRAM.len(), RegionFlags::empty());
        let _framebuffer = sdk::register_region(
            "framebuffer",
            1,
            FRAMEBUFFER.as_ptr(),
            FRAMEBUFFER.len(),
            RegionFlags::FRAMEBUFFER,
        );
        let _meta =
            sdk::register_region("meta", 1, META.as_ptr(), META.len(), RegionFlags::empty());
    }
}
