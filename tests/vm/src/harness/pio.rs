//! Port-I/O + MMIO device stubs: the detcall handler (bead 11d), a minimal
//! 8250 serial sink, and the trivial pv-pad MMIO latch stub (bead d4w).
//!
//! Every detcall is handled synchronously with the vCPU paused inside the
//! exit — exactly the production discipline (ARCHITECTURE.md §2). The
//! detcall handler delegates channel work to `detguest-host` so the harness
//! exercises the real host crate, not a reimplementation.

use std::collections::BTreeMap;

use detguest_host::Channel;
use detguest_wire::header::CHANNEL_SIZE_PAGES;
use detguest_wire::ports;

use super::VmHarness;

/// pv-pad MMIO latch stub (determinism-hypervisor ARCHITECTURE.md §6.4 owns
/// the real device; this repo only cites the addresses). Base GPA
/// 0xD000_1000; PAD0..PAD3 at +0x08 + 4*port; FRAME_COUNTER at +0x1C.
#[derive(Clone)]
pub struct PvPad {
    /// Latched pad values returned by PAD0..PAD3 reads.
    pub pads: [u32; 4],
    /// Last FRAME_COUNTER value written by the guest.
    pub frame_counter: u32,
    /// Per-frame input schedule: frame → latch operations, applied on the
    /// guest's FRAME_COUNTER writes (see [`PvPad::schedule`]).
    schedule: BTreeMap<u32, Vec<(u8, u32)>>,
}

/// pv-pad MMIO base GPA (cited from the hypervisor's device map).
pub const PVPAD_BASE: u64 = 0xD000_1000;
const PVPAD_PAD0_OFF: u64 = 0x08;
const PVPAD_FRAME_COUNTER_OFF: u64 = 0x1C;
/// One past the last register we decode.
const PVPAD_END_OFF: u64 = 0x20;

impl PvPad {
    /// Schedule a pad value (the "PAD_SET landing" stand-in for M3 tests).
    pub fn set_pad(&mut self, port: usize, value: u32) {
        self.pads[port] = value;
    }

    /// Schedule `value` on `port` to become visible to `poll_input` during
    /// guest frame `frame`.
    ///
    /// Latch timing contract: the SDK's `frame_mark` at the end of workload
    /// frame `F-1` writes FRAME_COUNTER value `F`, and the work period that
    /// follows (all `poll_input` calls included) is workload frame `F`. The
    /// harness latches the values scheduled for frame `F` inside that write's
    /// exit — so exactly the polls of frame `F` observe them. Values
    /// scheduled for frames the guest already passed are applied (in frame
    /// order, latest wins) on the next FRAME_COUNTER write. Frame 0 is not
    /// schedulable (no FRAME_COUNTER write precedes it); use `set_pad`.
    pub fn schedule(&mut self, frame: u32, port: u8, value: u32) {
        self.schedule.entry(frame).or_default().push((port, value));
    }

    /// Apply every schedule entry due at (or before) FRAME_COUNTER value
    /// `written`, in ascending frame order (latest wins per port).
    fn latch_due(&mut self, written: u32) {
        // split_off(written + 1) keeps frames > written scheduled; everything
        // else is due now. `written == u32::MAX` cannot leave a remainder.
        let later = match written.checked_add(1) {
            Some(next) => self.schedule.split_off(&next),
            None => BTreeMap::new(),
        };
        let due = std::mem::replace(&mut self.schedule, later);
        for (_frame, ops) in due {
            for (port, value) in ops {
                if let Some(pad) = self.pads.get_mut(port as usize) {
                    *pad = value;
                }
            }
        }
    }
}

const SERIAL_BASE: u16 = 0x3F8;
const SERIAL_END: u16 = 0x400;

/// All mutable PIO/MMIO device state. `Clone` so a `VmSnapshot` can carry
/// the detcall latches + pv-pad (including the input schedule) verbatim.
#[derive(Clone)]
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
                schedule: BTreeMap::new(),
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
        // Latch the input schedule for the frame this write opens (see
        // `PvPad::schedule` for the exact timing contract).
        pv.latch_due(value);
        return Some(value);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_pvpad() -> PvPad {
        PioState::new().pvpad
    }

    /// Simulate the guest's end-of-frame FRAME_COUNTER write.
    fn write_frame_counter(pv: &mut PvPad, value: u32) -> Option<u32> {
        apply_pvpad_write(pv, PVPAD_BASE + PVPAD_FRAME_COUNTER_OFF, value)
    }

    #[test]
    fn frame_counter_write_updates_latch_and_requests_drain() {
        let mut pv = fresh_pvpad();

        let drain_frame = write_frame_counter(&mut pv, 42);

        assert_eq!(drain_frame, Some(42));
        assert_eq!(pv.frame_counter, 42);
    }

    #[test]
    fn non_frame_counter_pvpad_write_does_not_request_drain() {
        let mut pv = fresh_pvpad();
        pv.frame_counter = 7;

        let drain_frame = apply_pvpad_write(&mut pv, PVPAD_BASE + PVPAD_PAD0_OFF, 99);

        assert_eq!(drain_frame, None);
        assert_eq!(pv.frame_counter, 7);
    }

    /// Scripted FRAME_COUNTER sequence: the value scheduled for frame K is
    /// latched exactly by the write of value K (the write that opens frame
    /// K's work period), so frame K's `poll_input` sees it.
    #[test]
    fn schedule_latches_on_the_frame_counter_write_that_opens_the_frame() {
        let mut pv = fresh_pvpad();
        pv.schedule(2, 0, 0xAA);
        pv.schedule(3, 0, 0xBB);
        pv.schedule(3, 1, 0x11);

        // Guest ends frame 0: write 1 opens frame 1 — nothing due yet.
        write_frame_counter(&mut pv, 1);
        assert_eq!(pv.pads, [0, 0, 0, 0], "frame 1 polls see nothing");

        // Write 2 opens frame 2: frame 2's value is now visible.
        write_frame_counter(&mut pv, 2);
        assert_eq!(
            pv.pads,
            [0xAA, 0, 0, 0],
            "frame 2 polls see frame 2's value"
        );

        // Write 3 opens frame 3: both ports latch.
        write_frame_counter(&mut pv, 3);
        assert_eq!(
            pv.pads,
            [0xBB, 0x11, 0, 0],
            "frame 3 polls see frame 3's values"
        );

        // Write 4: nothing scheduled — the latch is sticky.
        write_frame_counter(&mut pv, 4);
        assert_eq!(pv.pads, [0xBB, 0x11, 0, 0]);
    }

    #[test]
    fn schedule_skipped_frames_apply_in_order_latest_wins() {
        let mut pv = fresh_pvpad();
        pv.schedule(2, 0, 0x22);
        pv.schedule(4, 0, 0x44);
        pv.schedule(6, 0, 0x66);

        // The guest jumps straight to write 5 (e.g. a child restored past
        // frames 2 and 4): both due entries apply, in frame order.
        write_frame_counter(&mut pv, 5);
        assert_eq!(pv.pads[0], 0x44, "latest due frame wins");

        write_frame_counter(&mut pv, 6);
        assert_eq!(pv.pads[0], 0x66);
    }

    #[test]
    fn schedule_out_of_range_port_is_ignored() {
        let mut pv = fresh_pvpad();
        pv.schedule(1, 7, 0xDEAD);
        write_frame_counter(&mut pv, 1);
        assert_eq!(pv.pads, [0, 0, 0, 0]);
    }
}
