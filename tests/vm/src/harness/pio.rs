//! Port-I/O + MMIO device stubs: the detcall handler (bead 11d), a minimal
//! 8250 serial sink, and the trivial pv-pad MMIO latch stub (bead d4w).
//!
//! Every detcall is handled synchronously with the vCPU paused inside the
//! exit — exactly the production discipline (ARCHITECTURE.md §2). The
//! detcall handler delegates channel work to `detguest-host` so the harness
//! exercises the real host crate, not a reimplementation.

use detguest_host::Channel;
use detguest_wire::header::CHANNEL_SIZE_PAGES;
use detguest_wire::ports;

use super::VmHarness;

/// pv-pad MMIO latch stub (determinism-hypervisor ARCHITECTURE.md §6.4 owns
/// the real device; this repo only cites the addresses). Base GPA
/// 0xD000_1000; PAD0..PAD3 at +0x08 + 4*port; FRAME_COUNTER at +0x18.
pub struct PvPad {
    /// Latched pad values returned by PAD0..PAD3 reads.
    pub pads: [u32; 4],
    /// Last FRAME_COUNTER value written by the guest.
    pub frame_counter: u32,
}

/// pv-pad MMIO base GPA (cited from the hypervisor's device map).
pub const PVPAD_BASE: u64 = 0xD000_1000;
const PVPAD_PAD0_OFF: u64 = 0x08;
const PVPAD_FRAME_COUNTER_OFF: u64 = 0x18;
/// One past the last register we decode.
const PVPAD_END_OFF: u64 = 0x1C;

impl PvPad {
    /// Schedule a pad value (the "PAD_SET landing" stand-in for M3 tests).
    pub fn set_pad(&mut self, port: usize, value: u32) {
        self.pads[port] = value;
    }
}

const SERIAL_BASE: u16 = 0x3F8;
const SERIAL_END: u16 = 0x400;

/// All mutable PIO/MMIO device state.
pub struct PioState {
    /// CHANNEL_INIT GPA latches (API.md §5).
    pub init_lo: u32,
    pub init_hi: u32,
    /// Last INIT_GO status (readable via IN 0xD37C).
    pub init_status: u32,
    /// Last INJECT answer (readable via IN 0xD384).
    pub inject_answer: u32,
    /// The pv-pad latch stub.
    pub pvpad: PvPad,
}

impl PioState {
    pub fn new() -> PioState {
        PioState {
            init_lo: 0,
            init_hi: 0,
            init_status: u32::MAX, // "never committed"
            inject_answer: 0,
            pvpad: PvPad {
                pads: [0; 4],
                frame_counter: 0,
            },
        }
    }
}

impl Default for PioState {
    fn default() -> Self {
        Self::new()
    }
}

/// Handle `IN eax, port`.
pub fn handle_in(h: &mut VmHarness, port: u16) -> u32 {
    match port {
        ports::PORT_IDENT => ports::IDENT_VALUE,
        ports::PORT_INIT_LO => h.pio_state().init_lo,
        ports::PORT_INIT_HI => h.pio_state().init_hi,
        ports::PORT_INIT_GO => h.pio_state().init_status,
        ports::PORT_INJECT => h.pio_state().inject_answer,
        p if (ports::PORT_RANGE_START..=ports::PORT_RANGE_END).contains(&p) => 0, // RAZ
        p if (SERIAL_BASE..SERIAL_END).contains(&p) => match p - SERIAL_BASE {
            // LSR: THR empty + transmitter idle — the kernel never blocks.
            5 => 0x60,
            // MSR: CTS+DSR asserted.
            6 => 0xB0,
            _ => 0,
        },
        _ => 0, // unknown ports read as zero
    }
}

/// Handle `OUT port, eax`.
pub fn handle_out(h: &mut VmHarness, port: u16, value: u32) {
    match port {
        ports::PORT_INIT_LO => h.pio_state().init_lo = value,
        ports::PORT_INIT_HI => h.pio_state().init_hi = value,
        ports::PORT_INIT_GO => {
            let status = attach_channel(h, value);
            h.pio_state().init_status = status;
        }
        ports::PORT_DOORBELL => {
            // Mask bits select rings A/W; our drain covers both (the host
            // crate drains complete records only — the selective mask is a
            // production optimization the harness doesn't need).
            h.drain();
        }
        ports::PORT_INJECT => {
            // Sequencing rule (API.md §5): drain ring W first — the matching
            // InjectQuery was release-stored before this OUT.
            h.drain();
            let answer = {
                let Some(ch) = h.channel.as_mut() else { return };
                // Split-borrow dance: responder + sink both live on h.
                let mut sink = std::mem::take(&mut h.sink);
                let v = h.responder.answer(ch, value, &mut sink);
                h.sink = sink;
                v
            };
            h.pio_state().inject_answer = answer;
        }
        ports::PORT_QUIESCE_ACK => h.observed.quiesce_acks.push(value),
        p if (ports::PORT_RANGE_START..=ports::PORT_RANGE_END).contains(&p) => {} // WI
        p if (SERIAL_BASE..SERIAL_END).contains(&p) && p == SERIAL_BASE => {
            h.observed.serial.push(value as u8);
        }
        _ => {}
    }
}

/// CHANNEL_INIT commit (INIT_GO): validate the size, attach via
/// `detguest-host`, map errors to the API.md §5 status codes.
fn attach_channel(h: &mut VmHarness, size_pages: u32) -> u32 {
    if h.channel.is_some() {
        return 3; // already attached
    }
    if size_pages != CHANNEL_SIZE_PAGES {
        return 1; // bad GPA/size commit
    }
    let gpa = (h.pio_state().init_hi as u64) << 32 | h.pio_state().init_lo as u64;
    match Channel::attach(h.mem(), gpa) {
        Ok(ch) => {
            h.channel = Some(ch);
            0
        }
        Err(e) => e.init_status() as u32,
    }
}

/// pv-pad MMIO read.
pub fn pvpad_read(h: &mut VmHarness, addr: u64) -> u32 {
    let Some(off) = addr.checked_sub(PVPAD_BASE) else {
        return 0;
    };
    let pv = &h.pio_state().pvpad;
    match off {
        o if (PVPAD_PAD0_OFF..PVPAD_PAD0_OFF + 16).contains(&o) && o % 4 == 0 => {
            pv.pads[((o - PVPAD_PAD0_OFF) / 4) as usize]
        }
        PVPAD_FRAME_COUNTER_OFF => pv.frame_counter,
        _ => 0,
    }
}

/// pv-pad MMIO write (FRAME_COUNTER is the frame-boundary exit the host
/// records `frame → icount` at — the harness collects the sequence).
pub fn pvpad_write(h: &mut VmHarness, addr: u64, value: u32) {
    if let Some(frame) = apply_pvpad_write(&mut h.pio_state().pvpad, addr, value) {
        h.observed.frame_counter_writes.push(frame);
        // Drain inside the frame-boundary exit: the FrameMark record
        // preceding this write is guaranteed visible (ARCHITECTURE.md §2).
        h.drain();
    }
}

fn apply_pvpad_write(pv: &mut PvPad, addr: u64, value: u32) -> Option<u32> {
    let off = addr.checked_sub(PVPAD_BASE)?;
    if off == PVPAD_FRAME_COUNTER_OFF && off < PVPAD_END_OFF {
        pv.frame_counter = value;
        return Some(value);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_counter_write_updates_latch_and_requests_drain() {
        let mut pv = PvPad {
            pads: [0; 4],
            frame_counter: 0,
        };

        let drain_frame = apply_pvpad_write(&mut pv, PVPAD_BASE + PVPAD_FRAME_COUNTER_OFF, 42);

        assert_eq!(drain_frame, Some(42));
        assert_eq!(pv.frame_counter, 42);
    }

    #[test]
    fn non_frame_counter_pvpad_write_does_not_request_drain() {
        let mut pv = PvPad {
            pads: [0; 4],
            frame_counter: 7,
        };

        let drain_frame = apply_pvpad_write(&mut pv, PVPAD_BASE + PVPAD_PAD0_OFF, 99);

        assert_eq!(drain_frame, None);
        assert_eq!(pv.frame_counter, 7);
    }
}
