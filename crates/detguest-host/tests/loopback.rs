//! M1 acceptance (IMPLEMENTATION-PLAN): pure-host loopback test.
//!
//! A guest-side simulator — using `detguest-wire` *producer* code against the
//! same memory the host reads — produces 10^5 mixed events including ring
//! wrap, `Pad` records, droppable-event drops, and a region registration.
//! Assertions:
//! - `drain_events` recovers exactly the non-dropped sequence (per ring, in
//!   seq order, payload-equal);
//! - drop counters match the simulator's bookkeeping;
//! - every host mutation appeared exactly once in the recorded
//!   `ChannelWriteSink` trace (drains only bump consumer indices here, and
//!   each bump is logged once, in order, ending at the final index).
//!
//! The companion M1 requirement — `read_region` stitching a 3-extent region
//! across a discontiguous mock layout — is covered both in
//! `src/manifest.rs::tests` and at the end of this test against the live
//! channel manifest.

use std::collections::BTreeMap;

use detguest_host::{Channel, GuestEvent, GuestMem, MemError, OwnedPayload, RecordingSink, SinkOp};
use detguest_wire::events::{encode_event, encoded_event_len, EventPayload, RegionEvent};
use detguest_wire::header::{self, ChannelHeader, CHANNEL_SIZE, OFF_MANIFEST, OFF_RESERVED};
use detguest_wire::manifest::{
    init_manifest, writer_begin, writer_end, Extent, ManifestHeader, RegionEntry,
    MANIFEST_TOTAL_SIZE, REGION_FLAG_HOT,
};
use detguest_wire::record::{FLAG_REACHABLE_DECL, FLAG_TRUNCATED};
use detguest_wire::ring::{Producer, RingFull};
use detguest_wire::RingId;

const BASE: u64 = 0x1000_0000;

/// Raw-pointer guest memory over a leaked channel page. The guest-side
/// `wire::ring::Producer`s and this `GuestMem` share the allocation the same
/// way the real SDK/agent and hypervisor share the mapped hugepage: producers
/// exclusively own the free regions + their index cells, the host reads the
/// used regions and owns the consumer cells — the ring module's split-borrow
/// argument. Everything here is single-threaded; phases strictly alternate.
#[derive(Clone, Copy)]
struct RawChannelMem {
    ptr: *mut u8,
    len: usize,
}

impl RawChannelMem {
    fn leaked() -> RawChannelMem {
        let b: &'static mut [u8] = Box::leak(vec![0u8; CHANNEL_SIZE].into_boxed_slice());
        RawChannelMem {
            ptr: b.as_mut_ptr(),
            len: b.len(),
        }
    }

    fn range(&self, gpa: u64, len: usize) -> Result<usize, MemError> {
        let off = gpa
            .checked_sub(BASE)
            .ok_or(MemError::Unmapped { gpa, len })? as usize;
        if off.checked_add(len).map_or(true, |end| end > self.len) {
            return Err(MemError::Unmapped { gpa, len });
        }
        Ok(off)
    }
}

impl GuestMem for RawChannelMem {
    fn read(&self, gpa: u64, buf: &mut [u8]) -> Result<(), MemError> {
        let off = self.range(gpa, buf.len())?;
        unsafe { std::ptr::copy_nonoverlapping(self.ptr.add(off), buf.as_mut_ptr(), buf.len()) };
        Ok(())
    }
    fn write(&mut self, gpa: u64, buf: &[u8]) -> Result<(), MemError> {
        let off = self.range(gpa, buf.len())?;
        unsafe { std::ptr::copy_nonoverlapping(buf.as_ptr(), self.ptr.add(off), buf.len()) };
        Ok(())
    }
}

/// Deterministic LCG (no external dep; fixed seed — replayable by design).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        (self.next() >> 16) % n
    }
}

/// Guest-side simulator state for one ring.
struct SimRing<'a> {
    producer: Producer<'a>,
}

/// Simulator drop bookkeeping (mirrors the header counters it maintains).
#[derive(Default, Debug, PartialEq, Eq)]
struct DropBook {
    a_records: u64,
    a_bytes: u64,
    w_records: u64,
    w_bytes: u64,
    w_by_kind: [u64; 16],
}

struct Sim<'a> {
    mem: RawChannelMem,
    a: SimRing<'a>,
    w: SimRing<'a>,
    vnanos: u64,
    drops: DropBook,
    /// Critical-event doorbell drains triggered by a full ring.
    doorbells: u64,
    /// Expected non-dropped events, per ring, in production order.
    expected_a: Vec<GuestEvent>,
    expected_w: Vec<GuestEvent>,
}

impl<'a> Sim<'a> {
    fn ring_mut(&mut self, ring: RingId) -> &mut SimRing<'a> {
        match ring {
            RingId::A => &mut self.a,
            RingId::W => &mut self.w,
            _ => unreachable!(),
        }
    }
}

fn header_counter_add(mem: &mut RawChannelMem, off: usize, add: u64) {
    let mut b = [0u8; 8];
    mem.read(BASE + off as u64, &mut b).unwrap();
    let v = u64::from_le_bytes(b).wrapping_add(add);
    mem.write(BASE + off as u64, &v.to_le_bytes()).unwrap();
}

/// Push one event on `ring`, applying the ARCHITECTURE.md §3 flow-control
/// policy. Returns true if the event landed (false = dropped droppable).
/// Critical events on a full ring "doorbell": the host drains inside the
/// exit (events appended to `drained`), then the push retries.
#[allow(clippy::too_many_arguments)]
fn produce(
    sim: &mut Sim<'_>,
    ch: &mut Channel<RawChannelMem>,
    sink: &mut RecordingSink,
    drained: &mut Vec<GuestEvent>,
    ring: RingId,
    ev: EventPayload<'_>,
    extra_flags: u8,
    critical: bool,
) -> bool {
    sim.vnanos += 7;
    let vnanos = sim.vnanos;
    let len = encoded_event_len(&ev);
    loop {
        let sr = sim.ring_mut(ring);
        match sr.producer.try_push(len, |buf, seq| {
            encode_event(buf, seq, vnanos, extra_flags, &ev)
        }) {
            Ok(seq) => {
                let expected = expected_event(ring, seq, vnanos, extra_flags, &ev);
                match ring {
                    RingId::A => sim.expected_a.push(expected),
                    RingId::W => sim.expected_w.push(expected),
                    _ => unreachable!(),
                }
                return true;
            }
            Err(RingFull) => {
                if critical {
                    // Doorbell: host drains inside the exit, freeing space.
                    sim.doorbells += 1;
                    drained.extend(drain_and_collect(ch, sink));
                    continue;
                }
                // Droppable: bump header drop counters, skip (no doorbell).
                let kind_idx = kind_of(&ev) as usize;
                match ring {
                    RingId::A => {
                        header_counter_add(&mut sim.mem, header::OFF_RING_A_DROPPED_RECORDS, 1);
                        header_counter_add(
                            &mut sim.mem,
                            header::OFF_RING_A_DROPPED_BYTES,
                            len as u64,
                        );
                        sim.drops.a_records += 1;
                        sim.drops.a_bytes += len as u64;
                    }
                    RingId::W => {
                        header_counter_add(&mut sim.mem, header::OFF_RING_W_DROPPED_RECORDS, 1);
                        header_counter_add(
                            &mut sim.mem,
                            header::OFF_RING_W_DROPPED_BYTES,
                            len as u64,
                        );
                        header_counter_add(
                            &mut sim.mem,
                            header::OFF_RING_W_DROPPED_BY_KIND + kind_idx * 8,
                            1,
                        );
                        sim.drops.w_records += 1;
                        sim.drops.w_bytes += len as u64;
                        sim.drops.w_by_kind[kind_idx] += 1;
                    }
                    _ => unreachable!(),
                }
                return false;
            }
        }
    }
}

fn kind_of(ev: &EventPayload<'_>) -> u8 {
    use detguest_wire::record::EventKind as K;
    match ev {
        EventPayload::Pad => K::Pad as u8,
        EventPayload::Hello { .. } => K::Hello as u8,
        EventPayload::NameIntern { .. } => K::NameIntern as u8,
        EventPayload::AssertViolation { .. } => K::AssertViolation as u8,
        EventPayload::Reachable { .. } => K::Reachable as u8,
        EventPayload::Beacon { .. } => K::Beacon as u8,
        EventPayload::InjectQuery { .. } => K::InjectQuery as u8,
        EventPayload::RegionRegister(_) => K::RegionRegister as u8,
        EventPayload::RegionUpdate(_) => K::RegionUpdate as u8,
        EventPayload::WorkloadStarted { .. } => K::WorkloadStarted as u8,
        EventPayload::WorkloadExited { .. } => K::WorkloadExited as u8,
        EventPayload::LogLine { .. } => K::LogLine as u8,
        EventPayload::QuiesceReady { .. } => K::QuiesceReady as u8,
        EventPayload::FrameMark { .. } => K::FrameMark as u8,
        EventPayload::Ready { .. } => K::Ready as u8,
    }
}

fn expected_event(
    ring: RingId,
    seq: u32,
    vnanos: u64,
    extra_flags: u8,
    ev: &EventPayload<'_>,
) -> GuestEvent {
    let payload = match *ev {
        EventPayload::Hello {
            proto_version,
            agent_version,
            capabilities,
        } => OwnedPayload::Hello {
            proto_version,
            agent_version,
            capabilities,
        },
        EventPayload::NameIntern { name_id, name } => OwnedPayload::NameIntern {
            name_id,
            name: name.to_vec(),
            reachable_decl: extra_flags & FLAG_REACHABLE_DECL != 0,
        },
        EventPayload::AssertViolation {
            name_id,
            violation_count,
            details,
        } => OwnedPayload::AssertViolation {
            name_id,
            violation_count,
            details: details.to_vec(),
        },
        EventPayload::Reachable { name_id } => OwnedPayload::Reachable { name_id },
        EventPayload::Beacon { beacon_id } => OwnedPayload::Beacon { beacon_id },
        EventPayload::InjectQuery { iseq, name_id } => OwnedPayload::InjectQuery { iseq, name_id },
        EventPayload::RegionRegister(r) => OwnedPayload::RegionRegister(r),
        EventPayload::WorkloadStarted { guest_pid, unit } => {
            OwnedPayload::WorkloadStarted { guest_pid, unit }
        }
        EventPayload::WorkloadExited {
            guest_pid,
            exit_code,
            term_signal,
        } => OwnedPayload::WorkloadExited {
            guest_pid,
            exit_code,
            term_signal,
        },
        EventPayload::LogLine { stream, level, msg } => OwnedPayload::LogLine {
            stream,
            level,
            msg: msg.to_vec(),
        },
        EventPayload::QuiesceReady { token } => OwnedPayload::QuiesceReady { token },
        EventPayload::FrameMark { frame_index } => OwnedPayload::FrameMark { frame_index },
        EventPayload::Ready {
            unit,
            region_count,
            manifest_generation,
        } => OwnedPayload::Ready {
            unit,
            region_count,
            manifest_generation,
        },
        EventPayload::Pad | EventPayload::RegionUpdate(_) => unreachable!("not produced here"),
    };
    GuestEvent {
        ring,
        seq,
        vnanos,
        truncated: extra_flags & FLAG_TRUNCATED != 0,
        payload,
    }
}

fn drain_and_collect(ch: &mut Channel<RawChannelMem>, sink: &mut RecordingSink) -> Vec<GuestEvent> {
    ch.drain_events(sink)
        .expect("drain must succeed on a well-formed channel")
}

#[test]
fn loopback_100k_mixed_events() {
    // ---- channel setup (what the agent does at boot) ----
    let mut mem = RawChannelMem::leaked();
    let mut hdr = [0u8; OFF_RESERVED];
    ChannelHeader::canonical().write_to(&mut hdr).unwrap();
    mem.write(BASE, &hdr).unwrap();
    let mut area = vec![0u8; MANIFEST_TOTAL_SIZE];
    init_manifest(&mut area).unwrap();
    mem.write(BASE + OFF_MANIFEST as u64, &area).unwrap();

    let a_desc = RingId::A.canonical_desc();
    let w_desc = RingId::W.canonical_desc();
    let (a_prod, a_cons) = (RingId::A.prod_offset(), RingId::A.cons_offset());
    let (w_prod, w_cons) = (RingId::W.prod_offset(), RingId::W.cons_offset());
    // SAFETY: pointers into the leaked page; exactly one producer per ring;
    // the host side only ever touches the consumer halves.
    let (prod_a, prod_w) = unsafe {
        (
            Producer::from_raw(
                mem.ptr.add(a_desc.offset as usize),
                a_desc.size,
                mem.ptr.add(a_prod) as *mut u32,
                mem.ptr.add(a_cons) as *mut u32,
                0,
            ),
            Producer::from_raw(
                mem.ptr.add(w_desc.offset as usize),
                w_desc.size,
                mem.ptr.add(w_prod) as *mut u32,
                mem.ptr.add(w_cons) as *mut u32,
                0,
            ),
        )
    };

    let mut ch = Channel::attach(mem, BASE).unwrap();
    let mut sink = RecordingSink::default();
    let mut sim = Sim {
        mem,
        a: SimRing { producer: prod_a },
        w: SimRing { producer: prod_w },
        vnanos: 0,
        drops: DropBook::default(),
        doorbells: 0,
        expected_a: Vec::new(),
        expected_w: Vec::new(),
    };

    let mut drained: Vec<GuestEvent> = Vec::new();
    let mut rng = Lcg(0xDE7_6E57);

    // ---- the agent's boot Hello ----
    produce(
        &mut sim,
        &mut ch,
        &mut sink,
        &mut drained,
        RingId::A,
        EventPayload::Hello {
            proto_version: 1,
            agent_version: 0x100,
            capabilities: 3,
        },
        0,
        true,
    );

    // ---- 10^5 mixed events ----
    const TOTAL: u64 = 100_000;
    let details = vec![b'd'; 80];
    let mut interned_next: u32 = 1;
    let mut frame: u32 = 0;
    let mut iseq: u32 = 0;
    for i in 0..TOTAL {
        // Periodic pause-boundary drain (the hypervisor's exploration step).
        if i % 4096 == 0 {
            drained.extend(drain_and_collect(&mut ch, &mut sink));
        }
        // A registration mid-stream: manifest seqlock write + critical event.
        if i == 50_000 {
            let mut area = vec![0u8; MANIFEST_TOTAL_SIZE];
            sim.mem.read(BASE + OFF_MANIFEST as u64, &mut area).unwrap();
            writer_begin(&mut area).unwrap();
            RegionEntry {
                region_id: 0,
                name_id: 0xFFFF_0001,
                layout_version: 1,
                flags: REGION_FLAG_HOT,
                gva: 0x7000_0000,
                len: 48,
                extent_off: 0,
                extent_n: 3,
                name: RegionEntry::pack_name(b"wram").unwrap(),
            }
            .write_to(&mut area, 0)
            .unwrap();
            Extent {
                gpa: 0x4000_0000,
                len: 16,
            }
            .write_to(&mut area, 0)
            .unwrap();
            Extent {
                gpa: 0x5000_0000,
                len: 24,
            }
            .write_to(&mut area, 1)
            .unwrap();
            Extent {
                gpa: 0x6000_0000,
                len: 8,
            }
            .write_to(&mut area, 2)
            .unwrap();
            let mut h = ManifestHeader::read_from(&area).unwrap();
            h.region_count = 1;
            h.extent_count = 3;
            h.write_to(&mut area).unwrap();
            writer_end(&mut area).unwrap();
            sim.mem.write(BASE + OFF_MANIFEST as u64, &area).unwrap();

            produce(
                &mut sim,
                &mut ch,
                &mut sink,
                &mut drained,
                RingId::W,
                EventPayload::NameIntern {
                    name_id: 0xFFFF_0001,
                    name: b"wram",
                },
                0,
                true,
            );
            produce(
                &mut sim,
                &mut ch,
                &mut sink,
                &mut drained,
                RingId::W,
                EventPayload::RegionRegister(RegionEvent {
                    region_id: 0,
                    name_id: 0xFFFF_0001,
                    layout_version: 1,
                    manifest_generation: 2,
                }),
                0,
                true,
            );
        }
        // A drop burst: spam droppable LogLines with NO intervening drain so
        // ring W genuinely fills and the droppable path drops.
        if i == 70_000 {
            let spam = vec![b's'; 992];
            for _ in 0..1200 {
                produce(
                    &mut sim,
                    &mut ch,
                    &mut sink,
                    &mut drained,
                    RingId::W,
                    EventPayload::LogLine {
                        stream: 4,
                        level: 4,
                        msg: &spam,
                    },
                    0,
                    false,
                );
            }
            assert!(sim.drops.w_records > 0, "the burst must overflow ring W");
        }

        // A critical burst: more critical events than ring A can hold with
        // no intervening pause drain — the doorbell-retry path must carry it.
        if i == 80_000 {
            for _ in 0..3000 {
                produce(
                    &mut sim,
                    &mut ch,
                    &mut sink,
                    &mut drained,
                    RingId::A,
                    EventPayload::WorkloadStarted {
                        guest_pid: 3,
                        unit: 1,
                    },
                    0,
                    true,
                );
            }
            assert!(sim.doorbells > 0, "the burst must trigger doorbell drains");
        }

        match rng.below(100) {
            // Ring W traffic (the workload side).
            0..=24 => {
                let id = interned_next;
                interned_next += 1;
                let declared = rng.below(4) == 0;
                produce(
                    &mut sim,
                    &mut ch,
                    &mut sink,
                    &mut drained,
                    RingId::W,
                    EventPayload::NameIntern {
                        name_id: id,
                        name: b"sim_name",
                    },
                    if declared { FLAG_REACHABLE_DECL } else { 0 },
                    true,
                );
            }
            25..=39 => {
                produce(
                    &mut sim,
                    &mut ch,
                    &mut sink,
                    &mut drained,
                    RingId::W,
                    EventPayload::AssertViolation {
                        name_id: 1 + (rng.below(50) as u32),
                        violation_count: 1 + (rng.below(5) as u32),
                        details: &details,
                    },
                    0,
                    true,
                );
            }
            40..=49 => {
                produce(
                    &mut sim,
                    &mut ch,
                    &mut sink,
                    &mut drained,
                    RingId::W,
                    EventPayload::Reachable {
                        name_id: 1 + (rng.below(50) as u32),
                    },
                    0,
                    true,
                );
            }
            50..=64 => {
                produce(
                    &mut sim,
                    &mut ch,
                    &mut sink,
                    &mut drained,
                    RingId::W,
                    EventPayload::Beacon {
                        beacon_id: rng.below(65536) as u32,
                    },
                    0,
                    false,
                );
            }
            65..=74 => {
                let q = iseq;
                iseq += 1;
                produce(
                    &mut sim,
                    &mut ch,
                    &mut sink,
                    &mut drained,
                    RingId::W,
                    EventPayload::InjectQuery {
                        iseq: q,
                        name_id: 1,
                    },
                    0,
                    true,
                );
            }
            75..=84 => {
                frame += 1;
                produce(
                    &mut sim,
                    &mut ch,
                    &mut sink,
                    &mut drained,
                    RingId::W,
                    EventPayload::FrameMark { frame_index: frame },
                    0,
                    true,
                );
            }
            // Ring A traffic (the agent side).
            85..=94 => {
                let msg = vec![b'a'; 1 + (rng.below(64) as usize)];
                produce(
                    &mut sim,
                    &mut ch,
                    &mut sink,
                    &mut drained,
                    RingId::A,
                    EventPayload::LogLine {
                        stream: 3,
                        level: 2,
                        msg: &msg,
                    },
                    0,
                    false,
                );
            }
            _ => {
                produce(
                    &mut sim,
                    &mut ch,
                    &mut sink,
                    &mut drained,
                    RingId::A,
                    EventPayload::WorkloadStarted {
                        guest_pid: 2,
                        unit: 0,
                    },
                    0,
                    true,
                );
            }
        }
    }
    // Final drain picks up the tail.
    drained.extend(drain_and_collect(&mut ch, &mut sink));
    drained.extend(drain_and_collect(&mut ch, &mut sink)); // idempotent when empty

    // ---- assertion 1: exactly the non-dropped sequence, per ring ----
    let got_a: Vec<&GuestEvent> = drained.iter().filter(|e| e.ring == RingId::A).collect();
    let got_w: Vec<&GuestEvent> = drained.iter().filter(|e| e.ring == RingId::W).collect();
    assert_eq!(got_a.len(), sim.expected_a.len(), "ring A event count");
    assert_eq!(got_w.len(), sim.expected_w.len(), "ring W event count");
    for (g, e) in got_a.iter().zip(sim.expected_a.iter()) {
        assert_eq!(*g, e, "ring A event mismatch");
    }
    for (g, e) in got_w.iter().zip(sim.expected_w.iter()) {
        assert_eq!(*g, e, "ring W event mismatch");
    }
    assert!(
        sim.expected_w.len() as u64 > TOTAL / 2,
        "sanity: most attempts landed (got {})",
        sim.expected_w.len()
    );

    // ---- assertion 2: drop counters match simulator bookkeeping ----
    let counters = ch.drop_counters().unwrap();
    assert_eq!(counters.ring_a_records, sim.drops.a_records);
    assert_eq!(counters.ring_a_bytes, sim.drops.a_bytes);
    assert_eq!(counters.ring_w_records, sim.drops.w_records);
    assert_eq!(counters.ring_w_bytes, sim.drops.w_bytes);
    assert_eq!(counters.ring_w_by_kind, sim.drops.w_by_kind);
    assert!(
        sim.drops.w_records > 0,
        "the scenario must actually exercise drops"
    );

    // ---- assertion 3: every host mutation exactly once in the sink ----
    // Drains are the only host mutations here: the trace must be ConsBump
    // ops only, strictly advancing per ring, ending at the final consumer
    // index, which equals the producer index (everything consumed).
    let mut last: BTreeMap<RingId, u32> = BTreeMap::new();
    for op in &sink.ops {
        match op {
            SinkOp::ConsBump { ring, new_cons } => {
                if let Some(prev) = last.get(ring) {
                    assert!(
                        new_cons.wrapping_sub(*prev) > 0,
                        "consumer index must strictly advance"
                    );
                }
                last.insert(*ring, *new_cons);
            }
            other => panic!("unexpected host mutation in loopback trace: {other:?}"),
        }
    }
    let gm = *ch.guest_mem();
    let read_u32 = |off: usize| {
        let mut b = [0u8; 4];
        gm.read(BASE + off as u64, &mut b).unwrap();
        u32::from_le_bytes(b)
    };
    for ring in [RingId::A, RingId::W] {
        let prod = read_u32(ring.prod_offset());
        let cons = read_u32(ring.cons_offset());
        assert_eq!(prod, cons, "{ring:?}: fully drained");
        assert_eq!(
            last.get(&ring),
            Some(&cons),
            "{ring:?}: last logged bump == final index"
        );
    }

    // ---- wraps actually happened: pads consumed extra seqs ----
    assert!(
        sim.a.producer.next_seq() as usize > sim.expected_a.len(),
        "ring A must have wrapped (pads consume seqs)"
    );
    assert!(
        sim.w.producer.next_seq() as usize > sim.expected_w.len(),
        "ring W must have wrapped (pads consume seqs)"
    );
    assert!(sim.doorbells > 0, "doorbell-retry path exercised");

    // ---- intern folding + declared reachables surfaced ----
    assert_eq!(ch.intern_name(0xFFFF_0001), Some("wram"));
    assert!(ch.declared_reachables().count() > 0);

    // ---- the registered region resolves via the live manifest ----
    let m = ch.read_manifest().unwrap();
    let r = m.resolve("wram").expect("registered region resolves");
    assert_eq!(r.extents.len(), 3);
    assert_eq!(r.len, 48);
}
