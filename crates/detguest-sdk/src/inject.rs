use crate::{intern, FaultDecision};

pub(crate) fn inject_point(name: &'static str) -> FaultDecision {
    let _ = intern::valid_name(name);
    FaultDecision::Proceed
}
