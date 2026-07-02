pub(crate) const BEACON_SLOTS: usize = 65_536;

#[derive(Debug)]
pub(crate) struct BeaconCounters {
    counts: Vec<u32>,
    seen: Vec<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BeaconHit {
    pub(crate) id: u32,
    pub(crate) first_hit: bool,
}

impl Default for BeaconCounters {
    fn default() -> Self {
        BeaconCounters {
            counts: vec![0; BEACON_SLOTS],
            seen: vec![false; BEACON_SLOTS],
        }
    }
}

impl BeaconCounters {
    pub(crate) fn hit(&mut self, id: u32) -> BeaconHit {
        let index = (id as usize) & (BEACON_SLOTS - 1);
        self.counts[index] = self.counts[index].saturating_add(1);
        let first_hit = !self.seen[index];
        self.seen[index] = true;
        BeaconHit {
            id: index as u32,
            first_hit,
        }
    }

    #[cfg(test)]
    pub(crate) fn count(&self, id: u32) -> u32 {
        self.counts[(id as usize) & (BEACON_SLOTS - 1)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beacon_hits_mask_and_report_first_hit_once() {
        let mut beacons = BeaconCounters::default();
        assert_eq!(
            beacons.hit(0x1_0001),
            BeaconHit {
                id: 1,
                first_hit: true
            }
        );
        assert_eq!(
            beacons.hit(1),
            BeaconHit {
                id: 1,
                first_hit: false
            }
        );
        assert_eq!(beacons.count(1), 2);
    }

    #[test]
    fn beacon_counts_saturate() {
        let mut beacons = BeaconCounters::default();
        beacons.counts[3] = u32::MAX;
        assert!(beacons.hit(3).first_hit);
        assert_eq!(beacons.count(3), u32::MAX);
    }
}
