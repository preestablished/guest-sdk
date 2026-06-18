pub(crate) const BEACON_SLOTS: usize = 65_536;

pub(crate) fn coverage_beacon(id: u32) {
    let _ = id & 0xFFFF;
}
