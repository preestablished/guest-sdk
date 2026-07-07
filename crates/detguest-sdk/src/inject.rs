//! `inject_point` — the SDK side of the Ms5 fault-injection round trip
//! (API.md §5): publish a critical `InjectQuery` on ring W, then OUT/IN on
//! `PORT_INJECT` and decode the host's packed [`FaultDecision`].

use detguest_wire::events::EventPayload;
use detguest_wire::ports::PORT_INJECT;

use crate::channel::EventClass;
use crate::{pio, vnanos, FaultDecision, SdkState};

/// First iseq an SDK instance allocates. iseq 0 is never emitted, so it can
/// serve as an unambiguous "no query" sentinel host-side.
pub(crate) const FIRST_ISEQ: u32 = 1;

impl SdkState {
    /// The wire contract has two load-bearing orderings:
    ///
    /// 1. The `InjectQuery` publication (with doorbell) MUST precede the OUT —
    ///    the host drains the query inside the PIO exit, and an OUT for an
    ///    unpublished iseq is answered Proceed + `unmatched_injects`. The
    ///    early returns below make the ordering structural: no OUT is
    ///    reachable unless the emit succeeded.
    /// 2. The iseq the OUT carries is the same one in the ring-W event — one
    ///    counter allocates both (the host folds drained queries as
    ///    iseq → name_id and matches the OUT's eax against that map).
    ///
    /// Every error path — invalid name, intern-table exhaustion, ring-W
    /// publication failure with the Critical retry discipline exhausted,
    /// detcall failure — deterministically returns `Proceed`.
    pub(crate) fn inject_point(&mut self, name: &'static str) -> FaultDecision {
        let Some(name_id) = self.intern_name(name, 0) else {
            return FaultDecision::Proceed;
        };
        let iseq = self.next_iseq;
        self.next_iseq = self.next_iseq.wrapping_add(1);
        let ev = EventPayload::InjectQuery { iseq, name_id };
        if self
            ._channel
            .emit_w_event_with_doorbell(vnanos(), 0, &ev, EventClass::Critical)
            .is_err()
        {
            return FaultDecision::Proceed;
        }
        if pio::detcall_out(PORT_INJECT, iseq).is_err() {
            return FaultDecision::Proceed;
        }
        match pio::detcall_in(PORT_INJECT) {
            Ok(packed) => FaultDecision::unpack(packed),
            Err(_) => FaultDecision::Proceed,
        }
    }
}
