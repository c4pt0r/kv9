//! Check a tail-offset hint before ordered search, preserving vector retention.
//!
//! Increasing indexes permit binary search. The flag is a checked sufficient
//! condition, not a new assumption about Raft: any non-increasing append
//! permanently selects the original first-match lookup for this ring lifetime.

use super::{RingEntry, APPLIED_RING};

pub(super) struct AppliedRing {
    entries: Vec<RingEntry>,
    strictly_increasing: bool,
}

impl Default for AppliedRing {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            strictly_increasing: true,
        }
    }
}

impl AppliedRing {
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
        self.strictly_increasing &= self
            .entries
            .last()
            .is_none_or(|last| last.index < entry.index);
        // Match the original push_ring operations and their order exactly.
        // Search indexing does not change growth, retention or eviction.
        self.entries.push(entry);
        let len = self.entries.len();
        if len > APPLIED_RING {
            self.entries.drain(..len - APPLIED_RING);
        }
    }

    pub(super) fn find_index(&self, index: u64) -> Option<&RingEntry> {
        if self.strictly_increasing {
            // Strict ordering implies uniqueness, so the binary search result
            // is the original iterator's first matching receipt, if present.
            // Consecutive indexes near the tail need one checked slot access.
            // Gaps can make the offset miss; the exact index check prevents a
            // neighboring receipt from being used as this proposal's outcome.
            if let Some(last) = self.entries.last() {
                if let Some(distance) = last.index.checked_sub(index) {
                    if let Ok(distance) = usize::try_from(distance) {
                        if distance < self.entries.len() {
                            let slot = self.entries.len() - 1 - distance;
                            let entry = &self.entries[slot];
                            if entry.index == index {
                                return Some(entry);
                            }
                        }
                    }
                }
            }
            self.entries
                .binary_search_by_key(&index, |entry| entry.index)
                .ok()
                .map(|slot| &self.entries[slot])
        } else {
            // Keep duplicate/out-of-order behavior exactly, including which
            // term and exclusive apply outcome wins for a repeated index.
            self.entries.iter().find(|entry| entry.index == index)
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

    fn compare_with_original_fifo(trace: impl IntoIterator<Item = RingEntry>) -> AppliedRing {
        let mut ring = AppliedRing::default();
        let mut reference = Vec::new();
        assert!(ring.find_index(0).is_none());
        assert!(ring.first().is_none() && ring.last().is_none());
        for next in trace {
            reference.push(next);
            if reference.len() > APPLIED_RING {
                let len = reference.len();
                reference.drain(..len - APPLIED_RING);
            }
            ring.push(next);
            assert_eq!(ring.len(), reference.len());
            assert_eq!(ring.entries.capacity(), reference.capacity());
            assert_eq!(ring.first(), reference.first());
            assert_eq!(ring.last(), reference.last());
            assert_eq!(ring.entries, reference);
            for index in [
                0,
                18,
                u64::MAX,
                next.index,
                next.index.saturating_add(1),
                reference[0].index,
                reference[0].index.saturating_sub(1),
            ] {
                assert_eq!(
                    ring.find_index(index),
                    reference.iter().find(|entry| entry.index == index),
                    "receipt lookup changed at index {index}"
                );
            }
        }
        ring
    }

    #[test]
    fn indexed_receipts_preserve_vector_eviction_gaps_and_extreme_indexes() {
        let trace = (0..(3 * APPLIED_RING + 17) as u64)
            .map(|n| entry(n * 3, n))
            .chain([entry(u64::MAX, u64::MAX)]);
        let ring = compare_with_original_fifo(trace);
        assert!(ring.strictly_increasing);
        assert_eq!(ring.find_index(u64::MAX), Some(&entry(u64::MAX, u64::MAX)));
    }

    #[test]
    fn duplicate_or_reordered_receipts_keep_the_original_first_match() {
        let trace = (0..APPLIED_RING as u64)
            .map(|n| entry(n, n))
            // Different terms/outcomes for the same index must not replace
            // the first retained receipt merely to accelerate lookup.
            .chain([entry(18, 90), entry(17, 91), entry(18, 92)])
            .chain((0..(2 * APPLIED_RING) as u64).map(|n| entry(10_000 + n, n)))
            .chain([entry(u64::MAX, 93), entry(0, 94), entry(u64::MAX, 95)]);
        let ring = compare_with_original_fifo(trace);
        assert!(!ring.strictly_increasing);
        assert_eq!(ring.find_index(u64::MAX), Some(&entry(u64::MAX, 93)));
    }

    #[test]
    fn tail_hint_preserves_all_receipts_across_dense_runs_and_gaps() {
        let mut ring = AppliedRing::default();
        let mut reference = Vec::new();
        for (ordinal, index) in (0..(2 * APPLIED_RING) as u64)
            .chain((10_000..10_128).step_by(3))
            .chain(u64::MAX - 32..=u64::MAX)
            .enumerate()
        {
            let next = entry(index, ordinal as u64);
            ring.push(next);
            reference.push(next);
            if reference.len() > APPLIED_RING {
                reference.remove(0);
            }
            for query in [
                0,
                index,
                index.saturating_sub(1),
                index.saturating_sub(31),
                index.saturating_add(1),
                reference[reference.len() / 2].index,
                reference[0].index,
                reference[0].index.saturating_sub(1),
                u64::MAX,
            ] {
                assert_eq!(
                    ring.find_index(query),
                    reference.iter().find(|e| e.index == query),
                    "tail hint changed full receipt for query {query}, tail {index}"
                );
            }
        }
        assert!(ring.strictly_increasing);
    }

    #[test]
    fn a_gap_never_turns_the_hint_neighbor_into_a_receipt() {
        let mut ring = AppliedRing::default();
        for index in [1, 3, 4] {
            ring.push(entry(index, index));
        }
        // Tail distance 4 - 2 points to slot zero, whose index is 1.
        // Bounds alone cannot establish the queried receipt.
        assert_eq!(ring.find_index(2), None);
        assert_eq!(ring.find_index(1), Some(&entry(1, 1)));
        assert_eq!(ring.find_index(3), Some(&entry(3, 3)));
        assert_eq!(ring.find_index(5), None);
        assert_eq!(ring.find_index(u64::MAX), None);
        // This duplicate would make the tail slot match a different verdict;
        // the checked ordering certificate must select the oldest match.
        ring.push(entry(3, 999));
        assert_eq!(ring.find_index(3), Some(&entry(3, 3)));
        assert!(!ring.strictly_increasing);
    }
}
