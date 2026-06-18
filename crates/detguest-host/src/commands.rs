//! `push_command` / `push_workload_ctrl`: host-side ring producers (API.md §2).
//!
//! NORMATIVE (ARCHITECTURE.md §2): ring I carries workload **control**
//! records only — quiesce relay in v1. It never carries pad input; pad input
//! travels exclusively via the hypervisor's pv-pad MMIO latch. The type
//! system enforces this here: [`WorkloadCtrl`] has no input-bearing variant.

use detguest_wire::events::{encode_command, encode_workload_ctrl, Command, WorkloadCtrl};
use detguest_wire::record::{encode_pad, record_len};
use detguest_wire::ring::{bytes_needed, contiguous_tail, free};
use detguest_wire::RingId;

use crate::channel::Channel;
use crate::guestmem::{GuestMem, GuestMemExt};
use crate::{ChannelWriteSink, PushError};

impl<M: GuestMem> Channel<M> {
    /// Push a command onto ring C (API.md §3.3). Host-produced: `vnanos` is
    /// 0; the input log carries the icount. Errors with
    /// [`PushError::RingFull`] if the ring lacks space — the host may simply
    /// retry at the next pause; it never spins the guest.
    pub fn push_command(
        &mut self,
        cmd: &Command,
        sink: &mut dyn ChannelWriteSink,
    ) -> Result<(), PushError> {
        let payload_len = match cmd {
            Command::Quiesce { .. } => 16,
            Command::ReverifyRegions => 0,
            _ => 8,
        };
        let total = record_len(payload_len);
        let mut seq = self.next_seq_c;
        let pushed = self.push_record(
            RingId::C,
            total,
            sink,
            |buf, s| encode_command(buf, s, cmd).map_err(PushError::Encode),
            &mut seq,
        )?;
        self.next_seq_c = seq;
        debug_assert!(pushed == total);
        Ok(())
    }

    /// Push a workload-control record onto ring I (quiesce relay only in
    /// v1 — NEVER pad input, which travels via the pv-pad latch).
    pub fn push_workload_ctrl(
        &mut self,
        rec: &WorkloadCtrl,
        sink: &mut dyn ChannelWriteSink,
    ) -> Result<(), PushError> {
        let total = record_len(8);
        let mut seq = self.next_seq_i;
        let pushed = self.push_record(
            RingId::I,
            total,
            sink,
            |buf, s| encode_workload_ctrl(buf, s, 0, rec).map_err(PushError::Encode),
            &mut seq,
        )?;
        self.next_seq_i = seq;
        debug_assert!(pushed == total);
        Ok(())
    }

    /// Shared host-producer push: space check (incl. tail pad), encode into a
    /// scratch span, write pad+record into ring memory, publish the producer
    /// index, and report the whole mutation through the sink as one
    /// `ring_push` (pad bytes first, then the record — the same span order
    /// they occupy in ring memory).
    fn push_record(
        &mut self,
        ring: RingId,
        total: usize,
        sink: &mut dyn ChannelWriteSink,
        encode: impl FnOnce(&mut [u8], u32) -> Result<usize, PushError>,
        seq: &mut u32,
    ) -> Result<usize, PushError> {
        let desc = self.ring_desc(ring);
        let size = desc.size;
        let mask = size - 1;
        let data = self.data_gpa(ring);
        let prod = self.gm.read_u32(self.prod_gpa(ring))?;
        let cons = self.gm.read_u32(self.cons_gpa(ring))?;
        let needed = bytes_needed(prod, size, total as u32);
        if free(prod, cons, size) < needed {
            return Err(PushError::RingFull);
        }

        let mut span = vec![0u8; needed as usize];
        let mut record_at = 0usize;
        let mut pos = prod;
        if needed > total as u32 {
            let tail = contiguous_tail(prod, size) as usize;
            encode_pad(&mut span[..tail], tail, *seq).map_err(PushError::Encode)?;
            *seq = seq.wrapping_add(1);
            record_at = tail;
        }
        let n = encode(&mut span[record_at..record_at + total], *seq)?;
        *seq = seq.wrapping_add(1);
        debug_assert_eq!(n, total);

        // Write the span into ring memory: pad at the old masked position,
        // record at offset 0 after a wrap (or contiguous when no pad).
        if record_at > 0 {
            self.gm
                .write(data + (pos & mask) as u64, &span[..record_at])?;
            pos = pos.wrapping_add(record_at as u32);
            debug_assert_eq!(pos & mask, 0);
        }
        self.gm
            .write(data + (pos & mask) as u64, &span[record_at..])?;
        let new_prod = prod.wrapping_add(needed);
        self.gm.write_u32(self.prod_gpa(ring), new_prod)?;
        sink.ring_push(ring, &span, new_prod);
        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::Channel;
    use crate::guestmem::{GuestMem, GuestMemExt, MockGuestMem};
    use crate::{RecordingSink, SinkOp};
    use detguest_wire::events::{
        decode_command, decode_workload_ctrl, encode_event, EventPayload, QuiesceMode,
    };
    use detguest_wire::header::{ChannelHeader, CHANNEL_SIZE, OFF_RESERVED};

    const BASE: u64 = 0x1000_0000;

    fn channel() -> Channel<MockGuestMem> {
        let mut gm = MockGuestMem::with_zeroed(BASE, CHANNEL_SIZE);
        let mut hdr = [0u8; OFF_RESERVED];
        ChannelHeader::canonical().write_to(&mut hdr).unwrap();
        gm.write(BASE, &hdr).unwrap();
        Channel::attach(gm, BASE).unwrap()
    }

    #[test]
    fn push_command_writes_record_and_logs_mutation() {
        let mut ch = channel();
        let mut sink = RecordingSink::default();
        let cmd = Command::StartWorkload {
            unit: 0,
            log_mask: 0x1F,
        };
        ch.push_command(&cmd, &mut sink).unwrap();

        // Record landed at ring C offset 0.
        let mut rec = [0u8; 24];
        ch.guest_mem()
            .read(ch.data_gpa(RingId::C), &mut rec)
            .unwrap();
        let (hdr, back) = decode_command(&rec).unwrap();
        assert_eq!(back, cmd);
        assert_eq!(hdr.seq, 0);
        assert_eq!(hdr.vnanos, 0);

        // Producer index published; exactly one mutation in the trace.
        assert_eq!(ch.guest_mem().read_u32(ch.prod_gpa(RingId::C)).unwrap(), 24);
        assert_eq!(
            sink.ops,
            vec![SinkOp::RingPush {
                ring: RingId::C,
                bytes: rec.to_vec(),
                new_prod: 24
            }]
        );
    }

    #[test]
    fn producer_seqs_checkpoint_and_restore() {
        // Restore semantics: after a snapshot restore the hypervisor
        // re-attaches (seqs reset to 0) and must feed the checkpoint back so
        // pushes continue the seq stream instead of re-emitting seq 0.
        let mut ch = channel();
        let mut sink = RecordingSink::default();
        let cmd = Command::SetLogMask { mask: 1 };
        ch.push_command(&cmd, &mut sink).unwrap();
        ch.push_command(&cmd, &mut sink).unwrap();
        let saved = ch.producer_seqs();
        assert_eq!(saved.ring_c, 2);

        // "Restore": re-attach over the same guest memory.
        let gm = std::mem::replace(&mut ch.gm, MockGuestMem::new());
        let mut ch2 = Channel::attach(gm, BASE).unwrap();
        assert_eq!(ch2.producer_seqs().ring_c, 0, "fresh attach starts at 0");
        ch2.restore_producer_seqs(saved);
        ch2.push_command(&cmd, &mut sink).unwrap();
        // The third record on the ring carries seq 2, not 0.
        let mut rec = [0u8; 24];
        ch2.guest_mem()
            .read(ch2.data_gpa(RingId::C) + 48, &mut rec)
            .unwrap();
        let (hdr, _) = decode_command(&rec).unwrap();
        assert_eq!(hdr.seq, 2);
    }

    #[test]
    fn push_workload_ctrl_uses_ring_i() {
        let mut ch = channel();
        let mut sink = RecordingSink::default();
        ch.push_workload_ctrl(&WorkloadCtrl::QuiesceReq { token: 9 }, &mut sink)
            .unwrap();
        let mut rec = [0u8; 24];
        ch.guest_mem()
            .read(ch.data_gpa(RingId::I), &mut rec)
            .unwrap();
        let (_, back) = decode_workload_ctrl(&rec).unwrap();
        assert_eq!(back, WorkloadCtrl::QuiesceReq { token: 9 });
    }

    #[test]
    fn recording_sink_captures_ring_push_and_consumer_bump() {
        let mut ch = channel();
        let mut sink = RecordingSink::default();

        ch.push_command(&Command::SetLogMask { mask: 0x3 }, &mut sink)
            .unwrap();

        let mut rec = [0u8; 64];
        let n = encode_event(
            &mut rec,
            0,
            11,
            0,
            &EventPayload::Hello {
                proto_version: 1,
                agent_version: 0x100,
                capabilities: 0,
            },
        )
        .unwrap();
        ch.gm.write(ch.data_gpa(RingId::A), &rec[..n]).unwrap();
        ch.gm.write_u32(ch.prod_gpa(RingId::A), n as u32).unwrap();

        let events = ch.drain_events(&mut sink).unwrap();

        assert_eq!(events.len(), 1);
        assert!(matches!(
            sink.ops.first(),
            Some(SinkOp::RingPush {
                ring: RingId::C,
                ..
            })
        ));
        assert!(
            sink.ops.iter().any(|op| matches!(
                op,
                SinkOp::ConsBump {
                    ring: RingId::A,
                    new_cons
                } if *new_cons == n as u32
            )),
            "draining ring A must log the consumer bump"
        );
    }

    #[test]
    fn full_ring_reports_ring_full_without_mutating() {
        let mut ch = channel();
        let mut sink = RecordingSink::default();
        // Fill ring C (16 KiB) with 32-byte Quiesce records (16-byte payload):
        // 512 fill the ring exactly.
        let cmd = Command::Quiesce {
            token: 1,
            mode: QuiesceMode::Coop,
        };
        let mut pushed = 0;
        loop {
            match ch.push_command(&cmd, &mut sink) {
                Ok(()) => pushed += 1,
                Err(PushError::RingFull) => break,
                Err(e) => panic!("{e:?}"),
            }
            assert!(pushed < 1000, "ring never filled");
        }
        let ops_at_full = sink.ops.len();
        assert!(matches!(
            ch.push_command(&cmd, &mut sink),
            Err(PushError::RingFull)
        ));
        assert_eq!(
            sink.ops.len(),
            ops_at_full,
            "failed push must not log a mutation"
        );
    }

    #[test]
    fn wrap_emits_pad_in_same_logged_span() {
        let mut ch = channel();
        let mut sink = RecordingSink::default();
        // 24-byte StartWorkload records: 682 fill to 16368, leaving a 16-byte
        // tail — the next push needs a 16 B pad + 24 B record. Drain
        // (simulate guest consumption) first so it fits.
        let cmd = Command::StartWorkload {
            unit: 0,
            log_mask: 0,
        };
        while ch.push_command(&cmd, &mut sink).is_ok() {}
        // Guest consumed everything: bump cons to prod.
        let prod = ch.guest_mem().read_u32(ch.prod_gpa(RingId::C)).unwrap();
        ch.gm.write_u32(ch.cons_gpa(RingId::C), prod).unwrap();
        sink.ops.clear();

        ch.push_command(&cmd, &mut sink).unwrap();
        match &sink.ops[..] {
            [SinkOp::RingPush {
                ring: RingId::C,
                bytes,
                new_prod,
            }] => {
                // 16-byte pad + 24-byte record in one span.
                assert_eq!(bytes.len(), 16 + 24);
                assert_eq!(*new_prod, prod.wrapping_add(40));
                // Pad header: len 16, kind 0; then the record at offset 16.
                assert_eq!(&bytes[0..3], &[16, 0, 0]);
                let (_, back) = decode_command(&bytes[16..]).unwrap();
                assert_eq!(back, cmd);
            }
            other => panic!("unexpected trace {other:?}"),
        }
    }
}
