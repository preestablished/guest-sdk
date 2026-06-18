#[derive(Debug)]
pub(crate) struct InternTable {
    entries: Vec<InternEntry>,
    next_id: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InternedName {
    pub(crate) id: u32,
    pub(crate) is_new: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AssertCounts {
    pub(crate) pass_count: u32,
    pub(crate) fail_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InternError {
    NameTooLong,
    TableFull,
    UnknownName,
}

#[derive(Debug, Clone)]
struct InternEntry {
    name: &'static str,
    id: u32,
    reachable_hits: u32,
    assert_passes: u32,
    assert_failures: u32,
}

impl Default for InternTable {
    fn default() -> Self {
        InternTable {
            entries: Vec::new(),
            next_id: 1,
        }
    }
}

impl InternTable {
    pub(crate) fn intern(&mut self, name: &'static str) -> Result<InternedName, InternError> {
        if !valid_name(name) {
            return Err(InternError::NameTooLong);
        }
        if let Some(entry) = self.entries.iter().find(|entry| entry.name == name) {
            return Ok(InternedName {
                id: entry.id,
                is_new: false,
            });
        }
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).ok_or(InternError::TableFull)?;
        self.entries.push(InternEntry {
            name,
            id,
            reachable_hits: 0,
            assert_passes: 0,
            assert_failures: 0,
        });
        Ok(InternedName { id, is_new: true })
    }

    pub(crate) fn record_reachable(&mut self, id: u32) -> Result<u32, InternError> {
        let entry = self.entry_mut(id)?;
        entry.reachable_hits = entry.reachable_hits.saturating_add(1);
        Ok(entry.reachable_hits)
    }

    pub(crate) fn record_assert(
        &mut self,
        id: u32,
        passed: bool,
    ) -> Result<AssertCounts, InternError> {
        let entry = self.entry_mut(id)?;
        if passed {
            entry.assert_passes = entry.assert_passes.saturating_add(1);
        } else {
            entry.assert_failures = entry.assert_failures.saturating_add(1);
        }
        Ok(AssertCounts {
            pass_count: entry.assert_passes,
            fail_count: entry.assert_failures,
        })
    }

    fn entry_mut(&mut self, id: u32) -> Result<&mut InternEntry, InternError> {
        self.entries
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or(InternError::UnknownName)
    }
}

pub(crate) fn valid_name(name: &'static str) -> bool {
    name.len() <= detguest_wire::events::MAX_NAME
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_ids_start_at_one_and_duplicates_are_not_new() {
        let mut table = InternTable::default();
        assert_eq!(
            table.intern("alpha").unwrap(),
            InternedName {
                id: 1,
                is_new: true
            }
        );
        assert_eq!(
            table.intern("beta").unwrap(),
            InternedName {
                id: 2,
                is_new: true
            }
        );
        assert_eq!(
            table.intern("alpha").unwrap(),
            InternedName {
                id: 1,
                is_new: false
            }
        );
    }

    #[test]
    fn counters_are_stable_across_repeated_calls() {
        let mut table = InternTable::default();
        let id = table.intern("site").unwrap().id;
        assert_eq!(table.record_reachable(id).unwrap(), 1);
        assert_eq!(table.record_reachable(id).unwrap(), 2);
        assert_eq!(
            table.record_assert(id, true).unwrap(),
            AssertCounts {
                pass_count: 1,
                fail_count: 0
            }
        );
        assert_eq!(
            table.record_assert(id, false).unwrap(),
            AssertCounts {
                pass_count: 1,
                fail_count: 1
            }
        );
        assert_eq!(
            table.record_assert(id, false).unwrap(),
            AssertCounts {
                pass_count: 1,
                fail_count: 2
            }
        );
    }

    #[test]
    fn overlong_names_are_rejected() {
        let mut table = InternTable::default();
        let name = "x".repeat(detguest_wire::events::MAX_NAME + 1);
        let leaked: &'static str = Box::leak(name.into_boxed_str());
        assert_eq!(table.intern(leaked), Err(InternError::NameTooLong));
    }
}
