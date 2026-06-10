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

/// Replay-mode plan skeleton: reads decisions back from the input log so the
/// same call site gets the same answer bit-for-bit with the synthesizer
/// absent. Final wiring lands in determinism-hypervisor (it owns the DHILOG
/// format); until then this skeleton answers Proceed.
#[derive(Debug, Default)]
pub struct LogFaultPlan {
    // TODO(determinism-hypervisor): hold the replay cursor over the input
    // log's (iseq, decision) records and return them verbatim.
}

impl FaultPlan for LogFaultPlan {
    fn decide(&mut self, _iseq: u32, _name_id: u32, _name: Option<&str>) -> FaultDecision {
        FaultDecision::Proceed
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

    #[test]
    fn log_fault_plan_skeleton_proceeds() {
        let mut p = LogFaultPlan::default();
        assert_eq!(p.decide(0, 1, None), FaultDecision::Proceed);
    }
}
