pub(crate) fn valid_name(name: &'static str) -> bool {
    name.len() <= detguest_wire::events::MAX_NAME
}
