//! Game-materialization acceptance workload (plan
//! `phase3-game-device-materialization` package 04).
//!
//! A synthetic refwork-ctl unit that — unlike the m9 contract fixture, for
//! which `LoadGame.dev_path` was protocol theater — actually READS the game
//! file the agent materialized from pv-blk, and refuses to load anything
//! but the exact expected image:
//!
//! 1. `Hello` → `HelloAck`.
//! 2. `LoadGame{dev_path}`: assert the materialized path, `fs::read` it,
//!    compare length + checksum against the embedded expectation (the
//!    shared 32 KiB test pattern) — any divergence replies `Fault{detail}`
//!    (the agent turns that into a loud boot fault) instead of
//!    `GameLoaded`. On success print `game bytes=<len> checksum=0x<sum>`
//!    to stdout (host-visible LogLine via the supervise pipes).
//! 3. Register one region (`meta`, carrying checksum + length little-endian
//!    at offsets 0/8) so the boot exercises the production shape:
//!    materialize → control leg → region gate → Ready.
//! 4. Socket `Ready{frame: 0}` → await `Start` → park forever.
//!
//! The expected pattern (`((i*7) ^ (i>>8)) as u8`, 32 768 bytes) and the
//! checksum algorithm (seed `0x7062_6c6b_5f69_6f31`, rotate-left-5 fold at
//! whole-stream offsets — `detguest-agent`'s `pvblk::checksum_fold`; the
//! crates don't link) are the contract shared with
//! `tests/vm/tests/game_materialization.rs` and pinned there and in the
//! agent's unit tests as the same golden constant.

use core::ptr::{addr_of_mut, write_volatile};

use detguest_sdk::{self as sdk, RegionFlags};

const CONTROL_FD: i32 = 3;
const PROTO_VERSION: u64 = 1;

/// The path the agent materializes to under `game_source = "pv-blk"`
/// (guest-sdk API.md §7.1) — pinned here the way m9 pins `/dev/vdb`.
const GAME_IMG_PATH: &str = "/run/detguest/game.img";

/// The shared test pattern: exactly this image must be on the device.
const GAME_LEN: usize = 32 * 1024;

const CHECKSUM_SEED: u64 = 0x7062_6c6b_5f69_6f31;

const META_LEN: usize = 256;
static mut META: [u8; META_LEN] = [0; META_LEN];

fn pattern_byte(i: usize) -> u8 {
    ((i * 7) ^ (i >> 8)) as u8
}

fn checksum(bytes: &[u8]) -> u64 {
    let mut sum = CHECKSUM_SEED;
    for (i, byte) in bytes.iter().enumerate() {
        sum = sum.rotate_left(5) ^ u64::from(*byte).wrapping_add(i as u64);
    }
    sum
}

fn main() {
    let _ = sdk::init();
    let (len, sum) = drive_control();
    write_meta(len, sum);
    publish_meta();
    send_datagram(&[0x08, 0x00]); // Ready { frame: 0 }
    expect_start();
    // Park: the frame loop is out of scope for this fixture.
    loop {
        // SAFETY: plain pause(2); nothing ever wakes it.
        unsafe {
            libc::pause();
        }
    }
}

/// Hello/LoadGame leg. Returns (len, checksum) of the verified game image;
/// any divergence replies `Fault` and exits (the agent boot-faults on it).
fn drive_control() -> (u64, u64) {
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
    assert!(cur.is_empty(), "trailing LoadGame bytes");
    if dev_path != GAME_IMG_PATH {
        fault_and_exit(&format!(
            "unexpected game path {dev_path:?}, want {GAME_IMG_PATH:?}"
        ));
    }

    let bytes = match std::fs::read(&dev_path) {
        Ok(b) => b,
        Err(e) => fault_and_exit(&format!("cannot read game path `{dev_path}`: {e}")),
    };
    if bytes.len() != GAME_LEN {
        fault_and_exit(&format!(
            "game image is {} bytes, want {GAME_LEN}",
            bytes.len()
        ));
    }
    let sum = checksum(&bytes);
    let want: Vec<u8> = (0..GAME_LEN).map(pattern_byte).collect();
    let want_sum = checksum(&want);
    if bytes != want {
        fault_and_exit(&format!(
            "game image checksum {sum:#x} != expected {want_sum:#x}"
        ));
    }
    println!("game bytes={} checksum={sum:#018x}", bytes.len());
    send_game_loaded();
    (bytes.len() as u64, sum)
}

fn fault_and_exit(detail: &str) -> ! {
    let mut out = Vec::new();
    push_varint(&mut out, 10); // Fault
    push_varint(&mut out, 0); // frame
    push_varint(&mut out, 1); // code
    push_bytes(&mut out, detail.as_bytes());
    send_datagram(&out);
    std::process::exit(1);
}

fn write_meta(len: u64, sum: u64) {
    unsafe {
        let meta = addr_of_mut!(META).cast::<u8>();
        for (offset, byte) in sum.to_le_bytes().into_iter().enumerate() {
            write_volatile(meta.add(offset), byte);
        }
        for (offset, byte) in len.to_le_bytes().into_iter().enumerate() {
            write_volatile(meta.add(8 + offset), byte);
        }
    }
}

fn publish_meta() {
    // SAFETY: the static lives for the process lifetime and never moves,
    // satisfying the SDK region registration contract.
    unsafe {
        let meta = sdk::register_region(
            "meta",
            1,
            addr_of_mut!(META).cast::<u8>(),
            META_LEN,
            RegionFlags::empty(),
        )
        .expect("register meta");
        // Dropping the handle would unregister (DEAD) the region.
        std::mem::forget(meta);
    }
}

fn expect_start() {
    let start = recv_datagram();
    assert_eq!(start, [0x02], "expected Start");
}

fn send_hello_ack() {
    let mut out = Vec::new();
    push_varint(&mut out, 5);
    push_varint(&mut out, PROTO_VERSION);
    push_bytes(&mut out, b"game-load-check");
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
    // SAFETY: blocking recv on the inherited SEQPACKET control fd.
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
    // SAFETY: plain send on the control fd.
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
