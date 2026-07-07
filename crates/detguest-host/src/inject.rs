//! `InjectResponder` + `FaultPlan`: answering `inject_point` detcalls
//! (API.md §1.4, §2, §5).

use detguest_wire::ports::PORT_INJECT;
use detguest_wire::FaultDecision;

use crate::channel::Channel;
use crate::guestmem::GuestMem;
use crate::ChannelWriteSink;

/// Decides the answer to an inject query.
///
/// Recording mode: implemented over the input-synthesizer-provided fault
/// plan for the burst; the hypervisor records `(iseq, decision)` into the
/// input log. Replay mode: implemented over the input log itself.
pub trait FaultPlan {
    /// Decide for inject point `iseq` with interned name `name_id`
    /// (`name` resolved when the intern table has it).
    fn decide(&mut self, iseq: u32, name_id: u32, name: Option<&str>) -> FaultDecision;
}

/// Answers `inject_point` detcalls. The hypervisor's PIO handler drains ring
/// W inside the INJECT exit (the SDK wrote the `InjectQuery` record and
/// release-stored the producer index *before* `OUT 0xD384` — API.md §5
/// sequencing rule), then calls [`InjectResponder::answer`].
pub struct InjectResponder<P: FaultPlan> {
    plan: P,
}

impl<P: FaultPlan> InjectResponder<P> {
    /// Wrap a fault plan.
    pub fn new(plan: P) -> InjectResponder<P> {
        InjectResponder { plan }
    }

    /// Borrow the plan (test assertions).
    pub fn plan_mut(&mut self) -> &mut P {
        &mut self.plan
    }

    /// Answer the INJECT detcall for `iseq`: look up the matching
    /// `InjectQuery` (drained just before, inside the same exit), ask the
    /// plan, pack the decision, and report it via
    /// [`ChannelWriteSink::pio_answer`]. An unmatched `iseq` answers `0`
    /// (Proceed) and bumps [`Channel::unmatched_injects`] (API.md §5).
    pub fn answer<M: GuestMem>(
        &mut self,
        channel: &mut Channel<M>,
        iseq: u32,
        sink: &mut dyn ChannelWriteSink,
    ) -> u32 {
        let value = match channel.take_pending_inject(iseq) {
            Some(name_id) => {
                let name = channel.intern_name(name_id).map(str::to_owned);
                self.plan.decide(iseq, name_id, name.as_deref()).pack()
            }
            None => {
                channel.unmatched_injects += 1;
                FaultDecision::Proceed.pack()
            }
        };
        sink.pio_answer(PORT_INJECT, value);
        value
    }
}

/// One rule in a [`TableFaultPlan`].
#[derive(Debug, Clone)]
pub struct FaultRule {
    /// Glob over the inject point name (`*` matches any run of characters).
    /// Unresolvable names (intern not yet seen) never match a non-`*` glob.
    pub name_glob: String,
    /// Match only the n-th occurrence (0-based) of this rule's glob;
    /// `None` matches every occurrence.
    pub occurrence: Option<u32>,
    /// The decision to return on match.
    pub decision: FaultDecision,
}

/// Test-oriented plan: a rule table matched by name glob + occurrence index
/// (first matching rule wins; no match ⇒ Proceed).
#[derive(Debug, Default)]
pub struct TableFaultPlan {
    rules: Vec<FaultRule>,
    /// Per-rule match counters (occurrence indexing).
    hits: Vec<u32>,
    /// Every decision made, in order (test assertions / golden streams).
    pub decisions: Vec<(u32, FaultDecision)>,
}

impl TableFaultPlan {
    /// Build from rules.
    pub fn new(rules: Vec<FaultRule>) -> TableFaultPlan {
        let hits = vec![0; rules.len()];
        TableFaultPlan {
            rules,
            hits,
            decisions: Vec::new(),
        }
    }
}

impl FaultPlan for TableFaultPlan {
    fn decide(&mut self, iseq: u32, _name_id: u32, name: Option<&str>) -> FaultDecision {
        let mut out = FaultDecision::Proceed;
        for (i, rule) in self.rules.iter().enumerate() {
            let matches_name = match name {
                Some(n) => glob_match(&rule.name_glob, n),
                None => rule.name_glob == "*",
            };
            if !matches_name {
                continue;
            }
            let occ = self.hits[i];
            self.hits[i] += 1;
            if rule.occurrence.is_none() || rule.occurrence == Some(occ) {
                out = rule.decision;
                break;
            }
        }
        self.decisions.push((iseq, out));
        out
    }
}

/// One recorded replay decision — the input to [`LogFaultPlan`].
///
/// This is the "supplied replay decisions" seam: DHILOG serialization stays
/// hypervisor-owned, so the caller feeds *decoded* decisions (in round 2,
/// parsed from real DHILOG `pio_answer` records by determinism-hypervisor
/// code; in tests, synthetic fixtures or a record leg's
/// [`TableFaultPlan::decisions`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoggedDecision {
    /// The inject sequence number the decision was recorded at.
    pub iseq: u32,
    /// The interned point name the query carried.
    pub name_id: u32,
    /// The decision to replay verbatim.
    pub decision: FaultDecision,
}

/// One replay divergence a [`LogFaultPlan`] detected (the replay-divergence
/// surface — a nonzero count fails the gate).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogDivergence {
    /// The query's iseq did not match the log entry at the cursor.
    IseqMismatch {
        /// iseq the log expected at this position.
        expected: u32,
        /// iseq the replay run actually asked.
        got: u32,
    },
    /// The iseq matched but the query carried a different name_id.
    NameIdMismatch {
        /// The (matching) iseq.
        iseq: u32,
        /// name_id the log recorded.
        expected: u32,
        /// name_id the replay run actually asked.
        got: u32,
    },
    /// A query arrived after the log was exhausted.
    PastEnd {
        /// iseq of the unanswerable query.
        got: u32,
    },
}

/// Replay-mode plan: a cursor over recorded decisions, returned verbatim so
/// the same call site gets the same answer bit-for-bit with the synthesizer
/// absent.
///
/// "Fail loudly" semantics: `decide` runs inside the PIO exit path and must
/// not panic, and the [`FaultPlan`] trait cannot fail — so every divergence
/// (iseq mismatch, name_id mismatch, past-end query) answers `Proceed`,
/// consumes the cursor slot (keeping queries and log entries 1:1 and every
/// later divergence attributable), and is recorded for the harness/gate to
/// assert on via [`LogFaultPlan::divergences`]. A `Default`-constructed plan
/// has an empty log: every query is a `PastEnd` divergence answered
/// `Proceed`.
#[derive(Debug, Default)]
pub struct LogFaultPlan {
    log: Vec<LoggedDecision>,
    cursor: usize,
    divergences: Vec<LogDivergence>,
}

impl LogFaultPlan {
    /// Build over supplied replay decisions, in recorded order.
    pub fn new(decisions: Vec<LoggedDecision>) -> LogFaultPlan {
        LogFaultPlan {
            log: decisions,
            cursor: 0,
            divergences: Vec::new(),
        }
    }

    /// Every divergence detected so far, in query order. The replay gate
    /// fails on a nonzero count.
    pub fn divergences(&self) -> &[LogDivergence] {
        &self.divergences
    }

    /// Log entries not yet consumed by queries (a nonzero count at end of
    /// run means the replay asked fewer questions than the record leg).
    pub fn remaining(&self) -> usize {
        self.log.len().saturating_sub(self.cursor)
    }
}

impl FaultPlan for LogFaultPlan {
    fn decide(&mut self, iseq: u32, name_id: u32, _name: Option<&str>) -> FaultDecision {
        let Some(entry) = self.log.get(self.cursor) else {
            self.divergences.push(LogDivergence::PastEnd { got: iseq });
            return FaultDecision::Proceed;
        };
        self.cursor += 1;
        if entry.iseq != iseq {
            self.divergences.push(LogDivergence::IseqMismatch {
                expected: entry.iseq,
                got: iseq,
            });
            return FaultDecision::Proceed;
        }
        if entry.name_id != name_id {
            self.divergences.push(LogDivergence::NameIdMismatch {
                iseq,
                expected: entry.name_id,
                got: name_id,
            });
            return FaultDecision::Proceed;
        }
        entry.decision
    }
}

/// Minimal `*`-only glob (no deps): `*` matches any (possibly empty) run.
fn glob_match(pattern: &str, text: &str) -> bool {
    fn inner(p: &[u8], t: &[u8]) -> bool {
        match p.split_first() {
            None => t.is_empty(),
            Some((b'*', rest)) => (0..=t.len()).any(|k| inner(rest, &t[k..])),
            Some((c, rest)) => t
                .split_first()
                .is_some_and(|(tc, tr)| tc == c && inner(rest, tr)),
        }
    }
    inner(pattern.as_bytes(), text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guestmem::{GuestMem, MockGuestMem};
    use crate::{RecordingSink, SinkOp};
    use detguest_wire::events::{encode_event, encoded_event_len, EventPayload};
    use detguest_wire::header::{ChannelHeader, CHANNEL_SIZE, OFF_RESERVED};
    use detguest_wire::RingId;

    const BASE: u64 = 0x1000_0000;

    fn channel_with_w_records(events: &[EventPayload<'_>]) -> Channel<MockGuestMem> {
        let mut gm = MockGuestMem::with_zeroed(BASE, CHANNEL_SIZE);
        let mut hdr = [0u8; OFF_RESERVED];
        ChannelHeader::canonical().write_to(&mut hdr).unwrap();
        gm.write(BASE, &hdr).unwrap();
        // Lay the records contiguously at ring W offset 0 and publish prod.
        let desc = RingId::W.canonical_desc();
        let mut off = 0u32;
        for (seq, ev) in events.iter().enumerate() {
            let mut buf = [0u8; 4096];
            let n = encode_event(&mut buf, seq as u32, 1, 0, ev).unwrap();
            assert_eq!(n, encoded_event_len(ev));
            gm.write(BASE + desc.offset as u64 + off as u64, &buf[..n])
                .unwrap();
            off += n as u32;
        }
        gm.write(BASE + RingId::W.prod_offset() as u64, &off.to_le_bytes())
            .unwrap();
        Channel::attach(gm, BASE).unwrap()
    }

    #[test]
    fn glob_matching() {
        assert!(glob_match("*", ""));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("io_*", "io_read"));
        assert!(!glob_match("io_*", "net_read"));
        assert!(glob_match("*_fault_*", "disk_fault_7"));
        assert!(glob_match("exact", "exact"));
        assert!(!glob_match("exact", "exactly"));
    }

    #[test]
    fn responder_answers_matched_query_via_plan_and_logs_pio() {
        let mut ch = channel_with_w_records(&[
            EventPayload::NameIntern {
                name_id: 1,
                name: b"io_read",
            },
            EventPayload::InjectQuery {
                iseq: 0,
                name_id: 1,
            },
        ]);
        let mut sink = RecordingSink::default();
        ch.drain_events(&mut sink).unwrap();

        let plan = TableFaultPlan::new(vec![FaultRule {
            name_glob: "io_*".into(),
            occurrence: None,
            decision: FaultDecision::Platform { kind: 1, arg: 5 },
        }]);
        let mut responder = InjectResponder::new(plan);
        let v = responder.answer(&mut ch, 0, &mut sink);
        assert_eq!(v, FaultDecision::Platform { kind: 1, arg: 5 }.pack());
        assert_eq!(ch.unmatched_injects, 0);
        assert_eq!(
            sink.ops.last(),
            Some(&SinkOp::PioAnswer {
                port: PORT_INJECT,
                value: v
            })
        );

        // Same iseq again: already consumed → unmatched → Proceed + metric.
        let v2 = responder.answer(&mut ch, 0, &mut sink);
        assert_eq!(v2, 0);
        assert_eq!(ch.unmatched_injects, 1);
    }

    #[test]
    fn occurrence_indexing_selects_nth_match() {
        let mut plan = TableFaultPlan::new(vec![FaultRule {
            name_glob: "io_*".into(),
            occurrence: Some(1),
            decision: FaultDecision::Platform { kind: 2, arg: 64 },
        }]);
        assert_eq!(plan.decide(0, 1, Some("io_read")), FaultDecision::Proceed);
        assert_eq!(
            plan.decide(1, 1, Some("io_read")),
            FaultDecision::Platform { kind: 2, arg: 64 }
        );
        assert_eq!(plan.decide(2, 1, Some("io_read")), FaultDecision::Proceed);
        assert_eq!(plan.decisions.len(), 3);
    }

    #[test]
    fn first_matching_rule_wins() {
        let mut plan = TableFaultPlan::new(vec![
            FaultRule {
                name_glob: "io_read".into(),
                occurrence: None,
                decision: FaultDecision::Workload { kind: 64, arg: 1 },
            },
            FaultRule {
                name_glob: "io_*".into(),
                occurrence: None,
                decision: FaultDecision::Platform { kind: 1, arg: 0 },
            },
        ]);
        assert_eq!(
            plan.decide(0, 1, Some("io_read")),
            FaultDecision::Workload { kind: 64, arg: 1 }
        );
        assert_eq!(
            plan.decide(1, 2, Some("io_write")),
            FaultDecision::Platform { kind: 1, arg: 0 }
        );
    }

    /// The record/replay fixture: three inject points with distinct names.
    /// Returns a freshly-attached channel with the queries drained.
    fn drained_inject_fixture(sink: &mut RecordingSink) -> Channel<MockGuestMem> {
        let mut ch = channel_with_w_records(&[
            EventPayload::NameIntern {
                name_id: 1,
                name: b"io_read",
            },
            EventPayload::NameIntern {
                name_id: 2,
                name: b"io_write",
            },
            EventPayload::NameIntern {
                name_id: 3,
                name: b"net_send",
            },
            EventPayload::InjectQuery {
                iseq: 1,
                name_id: 1,
            },
            EventPayload::InjectQuery {
                iseq: 2,
                name_id: 2,
            },
            EventPayload::InjectQuery {
                iseq: 3,
                name_id: 3,
            },
        ]);
        ch.drain_events(sink).unwrap();
        ch
    }

    fn pio_answers(sink: &RecordingSink) -> Vec<u32> {
        sink.ops
            .iter()
            .filter_map(|op| match op {
                SinkOp::PioAnswer { value, .. } => Some(*value),
                _ => None,
            })
            .collect()
    }

    /// The fixture round trip: record leg with `TableFaultPlan` (varied
    /// Platform/Workload/Proceed decisions), replay leg with `LogFaultPlan`
    /// seeded from the record leg's decisions — identical `pio_answer`
    /// values over a real channel, zero divergences, synthesizer absent.
    #[test]
    fn log_fault_plan_replays_recorded_decisions_verbatim() {
        // Record leg.
        let mut record_sink = RecordingSink::default();
        let mut ch = drained_inject_fixture(&mut record_sink);
        let plan = TableFaultPlan::new(vec![
            FaultRule {
                name_glob: "io_read".into(),
                occurrence: None,
                decision: FaultDecision::Platform { kind: 2, arg: 512 },
            },
            FaultRule {
                name_glob: "net_*".into(),
                occurrence: None,
                decision: FaultDecision::Workload { kind: 64, arg: 7 },
            },
            // io_write matches nothing ⇒ Proceed (the varied third answer).
        ]);
        let mut responder = InjectResponder::new(plan);
        for iseq in [1, 2, 3] {
            responder.answer(&mut ch, iseq, &mut record_sink);
        }
        let recorded = pio_answers(&record_sink);
        assert_eq!(
            recorded,
            vec![
                FaultDecision::Platform { kind: 2, arg: 512 }.pack(),
                0,
                FaultDecision::Workload { kind: 64, arg: 7 }.pack(),
            ]
        );

        // Seed the replay plan from the record leg's public decision trace
        // (the fixture knows its name_ids; DHILOG decoding is round-2 and
        // hypervisor-owned).
        let name_ids = [1u32, 2, 3];
        let decisions = responder
            .plan_mut()
            .decisions
            .iter()
            .zip(name_ids)
            .map(|(&(iseq, decision), name_id)| LoggedDecision {
                iseq,
                name_id,
                decision,
            })
            .collect();

        // Replay leg: same drained queries on a fresh channel, no table plan
        // anywhere (synthesizer absent).
        let mut replay_sink = RecordingSink::default();
        let mut ch2 = drained_inject_fixture(&mut replay_sink);
        let mut replayer = InjectResponder::new(LogFaultPlan::new(decisions));
        for iseq in [1, 2, 3] {
            replayer.answer(&mut ch2, iseq, &mut replay_sink);
        }
        assert_eq!(pio_answers(&replay_sink), recorded);
        assert_eq!(replayer.plan_mut().divergences(), &[]);
        assert_eq!(replayer.plan_mut().remaining(), 0);
        assert_eq!(ch2.unmatched_injects, 0);
    }

    #[test]
    fn log_fault_plan_classifies_each_divergence_and_proceeds() {
        let log = vec![
            LoggedDecision {
                iseq: 1,
                name_id: 1,
                decision: FaultDecision::Platform { kind: 1, arg: 5 },
            },
            LoggedDecision {
                iseq: 2,
                name_id: 2,
                decision: FaultDecision::Workload { kind: 200, arg: 9 },
            },
        ];
        let mut p = LogFaultPlan::new(log);

        // (a) iseq mismatch at cursor: Proceed, slot consumed.
        assert_eq!(p.decide(9, 1, None), FaultDecision::Proceed);
        // (b) name_id mismatch for the matching iseq.
        assert_eq!(p.decide(2, 5, Some("io_write")), FaultDecision::Proceed);
        // (c) query past end of log.
        assert_eq!(p.decide(3, 3, None), FaultDecision::Proceed);

        assert_eq!(
            p.divergences(),
            &[
                LogDivergence::IseqMismatch {
                    expected: 1,
                    got: 9
                },
                LogDivergence::NameIdMismatch {
                    iseq: 2,
                    expected: 2,
                    got: 5
                },
                LogDivergence::PastEnd { got: 3 },
            ]
        );
        assert_eq!(p.remaining(), 0);
    }

    #[test]
    fn default_log_fault_plan_is_an_empty_log() {
        // Kept deliberately: an empty log answers every query Proceed while
        // classifying it PastEnd — a loud zero-fixture state, unlike the
        // silently-green old skeleton.
        let mut p = LogFaultPlan::default();
        assert_eq!(p.decide(1, 1, None), FaultDecision::Proceed);
        assert_eq!(p.divergences(), &[LogDivergence::PastEnd { got: 1 }]);
    }
}
