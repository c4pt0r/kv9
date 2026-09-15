//! Exclude impossible future-index lookups without assuming receipt order.
//!
//! `maximum_index` bounds every receipt ever inserted during this ring's life.
//! It deliberately does not decrease on eviction. A loose bound costs a scan;
//! it cannot hide a retained receipt or select a different duplicate/outcome.

use super::{RingEntry, APPLIED_RING};

#[derive(Default)]
pub(super) struct AppliedReceipts {
    entries: Vec<RingEntry>,
    maximum_index: u64,
}

impl AppliedReceipts {
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn first(&self) -> Option<&RingEntry> {
        self.entries.first()
    }

    pub(super) fn last(&self) -> Option<&RingEntry> {
        self.entries.last()
    }

    pub(super) fn push(&mut self, entry: RingEntry) {
        self.maximum_index = self.maximum_index.max(entry.index);
        // Preserve the original allocation, append and prefix-eviction order.
        self.entries.push(entry);
        let len = self.entries.len();
        if len > APPLIED_RING {
            self.entries.drain(..len - APPLIED_RING);
        }
    }

    pub(super) fn find_index(
        &self,
        index: u64,
        #[cfg(feature = "write-path-diagnostics")]
        observations: &crate::write_diagnostics::WriteDiagnostics,
    ) -> Option<&RingEntry> {
        if index > self.maximum_index {
            #[cfg(feature = "write-path-diagnostics")]
            observations.record_bounded_lookup(
                self.entries.len(),
                None,
                self.entries.last().map(|entry| entry.index),
                index,
                true,
            );
            return None;
        }
        #[cfg(not(feature = "write-path-diagnostics"))]
        {
            self.entries.iter().find(|entry| entry.index == index)
        }
        #[cfg(feature = "write-path-diagnostics")]
        {
            let slot = self.entries.iter().position(|entry| entry.index == index);
            observations.record_bounded_lookup(
                self.entries.len(),
                slot,
                self.entries.last().map(|entry| entry.index),
                index,
                false,
            );
            slot.map(|slot| &self.entries[slot])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(index: u64, term: u64) -> RingEntry {
        RingEntry {
            index,
            term,
            outcome: match term % 3 {
                0 => crate::ApplyOutcome::Plain,
                1 => crate::ApplyOutcome::FenceRejected(kv9_common::RegionId(term)),
                _ => crate::ApplyOutcome::Manifest(crate::ManifestVerdict::AlreadyApplied {
                    region: kv9_common::RegionId(term),
                    generation: index,
                }),
            },
        }
    }

    fn lookup(ring: &AppliedReceipts, index: u64) -> Option<&RingEntry> {
        ring.find_index(
            index,
            #[cfg(feature = "write-path-diagnostics")]
            &crate::write_diagnostics::WriteDiagnostics::default(),
        )
    }

    fn compare(trace: impl IntoIterator<Item = RingEntry>) -> AppliedReceipts {
        let mut ring = AppliedReceipts::default();
        let mut reference = Vec::new();
        for query in [0, 1, u64::MAX] {
            assert_eq!(lookup(&ring, query), None);
        }
        for next in trace {
            reference.push(next);
            if reference.len() > APPLIED_RING {
                reference.drain(..reference.len() - APPLIED_RING);
            }
            ring.push(next);
            assert_eq!(ring.entries, reference);
            assert_eq!(ring.entries.capacity(), reference.capacity());
            assert!(reference
                .iter()
                .all(|entry| entry.index <= ring.maximum_index));
            for query in [
                0,
                1,
                18,
                next.index,
                next.index.saturating_add(1),
                next.index.saturating_sub(1),
                reference[0].index,
                reference[0].index.saturating_sub(1),
                reference[reference.len() / 2].index,
                u64::MAX,
            ] {
                assert_eq!(
                    lookup(&ring, query),
                    reference.iter().find(|e| e.index == query)
                );
            }
        }
        ring
    }

    #[test]
    fn future_bound_preserves_gaps_zero_and_vector_eviction() {
        let ring = compare((0..(3 * APPLIED_RING + 17) as u64).map(|n| entry(n * 3, n)));
        assert_eq!(lookup(&ring, ring.maximum_index + 1), None);
        assert_eq!(ring.len(), APPLIED_RING);
    }

    #[test]
    fn reordered_and_duplicate_indexes_keep_the_first_full_receipt() {
        let ring = compare([entry(19, 1), entry(3, 2), entry(19, 99), entry(2, 100)]);
        assert_eq!(lookup(&ring, 19), Some(&entry(19, 1)));
        assert_eq!(lookup(&ring, 3), Some(&entry(3, 2)));
        assert_eq!(lookup(&ring, 20), None);
    }

    #[test]
    fn evicting_the_largest_index_keeps_a_conservative_nonwrapping_bound() {
        let ring = compare(
            [entry(u64::MAX, 1)]
                .into_iter()
                .chain((0..(2 * APPLIED_RING) as u64).map(|n| entry(n, n))),
        );
        assert_eq!(ring.maximum_index, u64::MAX);
        assert_eq!(lookup(&ring, u64::MAX), None);
        assert_eq!(lookup(&ring, (2 * APPLIED_RING - 1) as u64), ring.last());
    }

    #[cfg(feature = "write-path-diagnostics")]
    #[test]
    fn observations_count_actual_skipped_scans_and_fallback_comparisons() {
        let mut ring = AppliedReceipts::default();
        for item in [entry(19, 1), entry(3, 2), entry(19, 99)] {
            ring.push(item);
        }
        let observations = crate::write_diagnostics::WriteDiagnostics::default();
        assert_eq!(ring.find_index(20, &observations), None);
        assert_eq!(ring.find_index(19, &observations), Some(&entry(19, 1)));
        assert_eq!(ring.find_index(4, &observations), None);
        let snapshot = observations.snapshot();
        assert_eq!((snapshot.lookup_hits, snapshot.lookup_misses), (1, 2));
        assert_eq!(snapshot.distributions[14].sum, 4);
        assert_eq!(snapshot.distributions[17].count, 1);
        assert_eq!(snapshot.distributions[17].sum, 3);
        assert_eq!(snapshot.distributions[13].sum, 9);
        assert_eq!(snapshot.distributions[15].sum, 2);
    }
}
