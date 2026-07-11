use crate::BEACON_SLOTS;

pub(crate) const NAME_SLOTS: usize = 1_024;
pub(crate) const STATS_REGION_SIZE: usize = 0x46040;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ReachableStat {
    pub(crate) name_id: u32,
    pub(crate) hits: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AssertStat {
    pub(crate) name_id: u32,
    pub(crate) pass_lo: u32,
    pub(crate) fail_lo: u32,
    pub(crate) _pad: u32,
}

#[repr(C, align(64))]
#[derive(Debug)]
pub(crate) struct StatsRegion {
    pub(crate) stats_version: u32,
    pub(crate) _pad0: u32,
    pub(crate) asserts_passed_total: u64,
    pub(crate) asserts_failed_total: u64,
    pub(crate) reachable_names: u64,
    pub(crate) inject_queries_total: u64,
    pub(crate) _reserved: [u8; 24],
    pub(crate) beacon_counts: [u32; BEACON_SLOTS],
    pub(crate) reachable: [ReachableStat; NAME_SLOTS],
    pub(crate) assertions: [AssertStat; NAME_SLOTS],
}

impl Default for StatsRegion {
    fn default() -> Self {
        Self {
            stats_version: 1,
            _pad0: 0,
            asserts_passed_total: 0,
            asserts_failed_total: 0,
            reachable_names: 0,
            inject_queries_total: 0,
            _reserved: [0; 24],
            beacon_counts: [0; BEACON_SLOTS],
            reachable: [ReachableStat::default(); NAME_SLOTS],
            assertions: [AssertStat::default(); NAME_SLOTS],
        }
    }
}

impl StatsRegion {
    pub(crate) fn record_beacon(&mut self, id: u32) {
        let slot = (id as usize) & (BEACON_SLOTS - 1);
        self.beacon_counts[slot] = self.beacon_counts[slot].saturating_add(1);
    }

    pub(crate) fn record_reachable(&mut self, name_id: u32, hits: u32) {
        if let Some(slot) = named_slot(&mut self.reachable, name_id, |entry| entry.name_id) {
            slot.name_id = name_id;
            slot.hits = hits;
        }
    }

    pub(crate) fn record_assert(&mut self, name_id: u32, pass: u32, fail: u32) {
        if let Some(slot) = named_slot(&mut self.assertions, name_id, |entry| entry.name_id) {
            slot.name_id = name_id;
            slot.pass_lo = pass;
            slot.fail_lo = fail;
        }
    }
}

fn named_slot<T>(entries: &mut [T], name_id: u32, id: impl Fn(&T) -> u32) -> Option<&mut T> {
    let index = entries
        .iter()
        .position(|entry| id(entry) == name_id)
        .or_else(|| entries.iter().position(|entry| id(entry) == 0))?;
    Some(&mut entries[index])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, offset_of, size_of};

    #[test]
    fn layout_v1_is_byte_pinned() {
        assert_eq!(align_of::<StatsRegion>(), 64);
        assert_eq!(size_of::<StatsRegion>(), STATS_REGION_SIZE);
        assert_eq!(offset_of!(StatsRegion, stats_version), 0x00000);
        assert_eq!(offset_of!(StatsRegion, asserts_passed_total), 0x00008);
        assert_eq!(offset_of!(StatsRegion, asserts_failed_total), 0x00010);
        assert_eq!(offset_of!(StatsRegion, reachable_names), 0x00018);
        assert_eq!(offset_of!(StatsRegion, inject_queries_total), 0x00020);
        assert_eq!(offset_of!(StatsRegion, beacon_counts), 0x00040);
        assert_eq!(offset_of!(StatsRegion, reachable), 0x40040);
        assert_eq!(offset_of!(StatsRegion, assertions), 0x42040);
    }
}
