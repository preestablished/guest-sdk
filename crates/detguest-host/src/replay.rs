//! Replay a recorded [`SinkOp`] trace into a channel — the proof that the
//! sink trace alone reconstructs every host-owned channel mutation (the
//! `m5-host-mutation-log-audit` acceptance: "a single ordered trace can
//! replay all host-owned channel mutations"). The Ms5 `determinism_replay`
//! scaffold folds its S1–S3 hash surfaces from the same trace shape.

use crate::channel::Channel;
use crate::guestmem::{GuestMem, GuestMemExt, MemError};
use crate::{ChannelWriteSink, SinkOp};

/// Apply one recorded host mutation to `ch`, re-reporting it through `sink`
/// — the crate's no-mutate-without-sink invariant holds on the replay path
/// too, so a replayed channel's own trace compares 1:1 with the recorded
/// one.
///
/// `PioAnswer` has no channel-memory effect; it is forwarded to the sink
/// only (the packed answer value is the comparison surface).
pub fn apply_sink_op<M: GuestMem>(
    ch: &mut Channel<M>,
    op: &SinkOp,
    sink: &mut dyn ChannelWriteSink,
) -> Result<(), MemError> {
    match op {
        SinkOp::RingPush {
            ring,
            bytes,
            new_prod,
        } => {
            let size = ch.ring_desc(*ring).size;
            let mask = size - 1;
            let data = ch.data_gpa(*ring);
            let old_prod = new_prod.wrapping_sub(bytes.len() as u32);
            let off = old_prod & mask;
            // A span with a tail pad occupies [off..size) then wraps to 0
            // (records themselves never wrap; only the pad+record span can).
            let tail = (size - off) as usize;
            if bytes.len() <= tail {
                ch.gm.write(data + off as u64, bytes)?;
            } else {
                ch.gm.write(data + off as u64, &bytes[..tail])?;
                ch.gm.write(data, &bytes[tail..])?;
            }
            ch.gm.write_u32(ch.prod_gpa(*ring), *new_prod)?;
            sink.ring_push(*ring, bytes, *new_prod);
        }
        SinkOp::ConsBump { ring, new_cons } => {
            ch.gm.write_u32(ch.cons_gpa(*ring), *new_cons)?;
            sink.cons_bump(*ring, *new_cons);
        }
        SinkOp::PioAnswer { port, value } => {
            sink.pio_answer(*port, *value);
        }
    }
    Ok(())
}

/// Apply a whole recorded trace, in order.
pub fn apply_trace<M: GuestMem>(
    ch: &mut Channel<M>,
    ops: &[SinkOp],
    sink: &mut dyn ChannelWriteSink,
) -> Result<(), MemError> {
    for op in ops {
        apply_sink_op(ch, op, sink)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guestmem::MockGuestMem;
    use crate::inject::{FaultRule, InjectResponder, TableFaultPlan};
    use crate::RecordingSink;
    use detguest_wire::events::{
        encode_event, encoded_event_len, Command, EventPayload, WorkloadCtrl,
    };
    use detguest_wire::header::{ChannelHeader, CHANNEL_SIZE, OFF_RESERVED};
    use detguest_wire::ports::PORT_INJECT;
    use detguest_wire::{FaultDecision, RingId};

    const BASE: u64 = 0x1000_0000;

    fn fresh_channel() -> Channel<MockGuestMem> {
        let mut gm = MockGuestMem::with_zeroed(BASE, CHANNEL_SIZE);
        let mut hdr = [0u8; OFF_RESERVED];
        ChannelHeader::canonical().write_to(&mut hdr).unwrap();
        gm.write(BASE, &hdr).unwrap();
        Channel::attach(gm, BASE).unwrap()
    }

    /// Write guest-produced records onto a ring (the guest half of the
    /// fixture — identical on the record and replay channels, since host
    /// mutations don't include guest writes).
    fn write_guest_events(ch: &mut Channel<MockGuestMem>, ring: RingId, events: &[EventPayload]) {
        let desc = ch.ring_desc(ring);
        let mut off = 0u32;
        for (seq, ev) in events.iter().enumerate() {
            let mut buf = [0u8; 4096];
            let n = encode_event(&mut buf, seq as u32, 1, 0, ev).unwrap();
            assert_eq!(n, encoded_event_len(ev));
            ch.gm
                .write(BASE + desc.offset as u64 + off as u64, &buf[..n])
                .unwrap();
            off += n as u32;
        }
        ch.gm
            .write(BASE + ring.prod_offset() as u64, &off.to_le_bytes())
            .unwrap();
    }

    fn guest_fixture(ch: &mut Channel<MockGuestMem>) {
        write_guest_events(
            ch,
            RingId::A,
            &[EventPayload::Hello {
                proto_version: 1,
                agent_version: 0x100,
                capabilities: 0,
            }],
        );
        write_guest_events(
            ch,
            RingId::W,
            &[
                EventPayload::NameIntern {
                    name_id: 1,
                    name: b"io_read",
                },
                EventPayload::InjectQuery {
                    iseq: 1,
                    name_id: 1,
                },
            ],
        );
    }

    fn read_page(ch: &Channel<MockGuestMem>) -> Vec<u8> {
        let mut page = vec![0u8; CHANNEL_SIZE];
        ch.guest_mem().read(BASE, &mut page).unwrap();
        page
    }

    /// Drive a mixed workload (C and I pushes, A and W drains, a matched and
    /// an unmatched inject answer) and return (final page, trace).
    fn mixed_workload(ch: &mut Channel<MockGuestMem>, sink: &mut RecordingSink) {
        ch.push_command(&Command::SetLogMask { mask: 0x1F }, sink)
            .unwrap();
        ch.push_workload_ctrl(&WorkloadCtrl::QuiesceReq { token: 7 }, sink)
            .unwrap();
        ch.push_command(
            &Command::StartWorkload {
                unit: 0,
                log_mask: 3,
            },
            sink,
        )
        .unwrap();
        let events = ch.drain_events(sink).unwrap();
        assert_eq!(events.len(), 3, "hello + intern + inject query");
        let mut responder = InjectResponder::new(TableFaultPlan::new(vec![FaultRule {
            name_glob: "io_*".into(),
            occurrence: None,
            decision: FaultDecision::Platform { kind: 2, arg: 64 },
        }]));
        responder.answer(ch, 1, sink); // matched
        responder.answer(ch, 99, sink); // unmatched → Proceed, still one op
    }

    /// Every host mutation class appears in the trace exactly as many times
    /// as its mutation happened — ring C and I pushes, ring A and W consumer
    /// bumps (each ring id distinct), and one pio_answer per IN (matched or
    /// not). Wrap-pad spans and failed-push-logs-nothing are pinned by
    /// `commands::tests::{wrap_emits_pad_in_same_logged_span,
    /// full_ring_reports_ring_full_without_mutating}`.
    #[test]
    fn every_host_mutation_is_reported_exactly_once() {
        let mut ch = fresh_channel();
        guest_fixture(&mut ch);
        let mut sink = RecordingSink::default();
        mixed_workload(&mut ch, &mut sink);

        let pushes_c = sink
            .ops
            .iter()
            .filter(|op| {
                matches!(
                    op,
                    SinkOp::RingPush {
                        ring: RingId::C,
                        ..
                    }
                )
            })
            .count();
        let pushes_i = sink
            .ops
            .iter()
            .filter(|op| {
                matches!(
                    op,
                    SinkOp::RingPush {
                        ring: RingId::I,
                        ..
                    }
                )
            })
            .count();
        let bumps: Vec<(RingId, u32)> = sink
            .ops
            .iter()
            .filter_map(|op| match op {
                SinkOp::ConsBump { ring, new_cons } => Some((*ring, *new_cons)),
                _ => None,
            })
            .collect();
        let answers: Vec<u32> = sink
            .ops
            .iter()
            .filter_map(|op| match op {
                SinkOp::PioAnswer { port, value } => {
                    assert_eq!(*port, PORT_INJECT);
                    Some(*value)
                }
                _ => None,
            })
            .collect();

        assert_eq!(
            pushes_c, 2,
            "two push_command calls, one RingPush{{C}} each"
        );
        assert_eq!(pushes_i, 1, "one push_workload_ctrl, one RingPush{{I}}");
        // One bump per drained ring, ring ids distinct, at the drained prod.
        let cons_a = ch
            .guest_mem()
            .read_u32(BASE + RingId::A.prod_offset() as u64);
        let cons_w = ch
            .guest_mem()
            .read_u32(BASE + RingId::W.prod_offset() as u64);
        assert_eq!(
            bumps,
            vec![(RingId::A, cons_a.unwrap()), (RingId::W, cons_w.unwrap())]
        );
        assert_eq!(
            answers,
            vec![FaultDecision::Platform { kind: 2, arg: 64 }.pack(), 0],
            "exactly one pio_answer per IN — matched then unmatched"
        );
        assert_eq!(sink.ops.len(), 3 + 2 + 2, "no unaccounted mutations");
    }

    /// The bead's acceptance line, literally: replay the single ordered
    /// trace against a second channel (same guest-produced bytes, fresh
    /// host state) and get byte-identical channel memory plus an identical
    /// re-reported trace.
    #[test]
    fn single_ordered_trace_replays_all_host_mutations() {
        let mut ch1 = fresh_channel();
        guest_fixture(&mut ch1);
        let mut sink1 = RecordingSink::default();
        mixed_workload(&mut ch1, &mut sink1);

        let mut ch2 = fresh_channel();
        guest_fixture(&mut ch2);
        let mut sink2 = RecordingSink::default();
        apply_trace(&mut ch2, &sink1.ops, &mut sink2).unwrap();

        assert_eq!(sink2.ops, sink1.ops, "replay re-reports the same trace");
        assert_eq!(
            read_page(&ch2),
            read_page(&ch1),
            "byte-identical channel page after replaying host mutations"
        );
    }

    /// A replayed wrap-pad span reconstructs the pad and the wrapped record.
    #[test]
    fn replay_reconstructs_wrap_pad_spans() {
        let cmd = Command::StartWorkload {
            unit: 0,
            log_mask: 0,
        };
        let mut ch1 = fresh_channel();
        let mut sink1 = RecordingSink::default();
        while ch1.push_command(&cmd, &mut sink1).is_ok() {}
        // Guest consumed everything; the next push wraps with a tail pad.
        let prod = ch1.guest_mem().read_u32(ch1.prod_gpa(RingId::C)).unwrap();
        ch1.gm.write_u32(ch1.cons_gpa(RingId::C), prod).unwrap();
        ch1.push_command(&cmd, &mut sink1).unwrap();

        let mut ch2 = fresh_channel();
        // Replay needs the same starting cons (a guest-side mutation).
        ch2.gm.write_u32(ch2.cons_gpa(RingId::C), prod).unwrap();
        let mut sink2 = RecordingSink::default();
        apply_trace(&mut ch2, &sink1.ops, &mut sink2).unwrap();
        assert_eq!(read_page(&ch2), read_page(&ch1));
    }
}
