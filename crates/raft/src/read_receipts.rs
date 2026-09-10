//! An index over the bounded FIFO of exact quorum-confirmed read contexts.
//!
//! Lookup preserves the original vector's first-match behavior, including
//! duplicate contexts. A hit is not consumed and does not change retention.

use std::collections::{BTreeMap, VecDeque};

pub(crate) struct ReadReceipts {
    capacity: usize,
    order: VecDeque<Vec<u8>>,
    indices: BTreeMap<Vec<u8>, VecDeque<u64>>,
}

impl ReadReceipts {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            capacity,
            order: VecDeque::new(),
            indices: BTreeMap::new(),
        }
    }

    pub(crate) fn push(&mut self, context: Vec<u8>, index: u64) {
        if self.capacity == 0 {
            return;
        }
        // Evict before insertion so the live index never exceeds its bound.
        if self.order.len() == self.capacity {
            let expired = self.order.pop_front().expect("nonempty bounded FIFO");
            let occurrences = self.indices.get_mut(&expired).expect("indexed FIFO member");
            occurrences.pop_front().expect("indexed FIFO occurrence");
            if occurrences.is_empty() {
                self.indices.remove(&expired);
            }
        }
        self.indices
            .entry(context.clone())
            .or_default()
            .push_back(index);
        self.order.push_back(context);
    }

    pub(crate) fn get(&self, context: &[u8]) -> Option<u64> {
        self.indices.get(context)?.front().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_context_lookup_is_nonconsuming_and_fifo_eviction_is_not_lru() {
        let mut receipts = ReadReceipts::new(2);
        receipts.push(b"incarnation-a/request-1".to_vec(), 7);
        receipts.push(b"incarnation-a/request-2".to_vec(), 11);
        assert_eq!(
            receipts.get(b"incarnation-b/request-1"),
            None,
            "a different exact context acquired confirmation"
        );
        assert_eq!(receipts.get(b"incarnation-a/request-1/extra"), None);
        for _ in 0..3 {
            assert_eq!(receipts.get(b"incarnation-a/request-1"), Some(7));
        }
        receipts.push(b"incarnation-b/request-1".to_vec(), 19);
        assert_eq!(
            receipts.get(b"incarnation-a/request-1"),
            None,
            "evicted read context remained confirmed"
        );
        assert_eq!(receipts.get(b"incarnation-a/request-2"), Some(11));
        assert_eq!(receipts.get(b"incarnation-b/request-1"), Some(19));
    }

    #[test]
    fn duplicate_context_keeps_the_first_retained_confirmation() {
        let mut receipts = ReadReceipts::new(3);
        receipts.push(vec![1], 7);
        receipts.push(vec![2], 9);
        receipts.push(vec![1], 13);
        assert_eq!(
            receipts.get(&[1]),
            Some(7),
            "duplicate context replaced its first retained receipt"
        );
        receipts.push(vec![3], 17);
        assert_eq!(receipts.get(&[1]), Some(13));
        receipts.push(vec![3], 23);
        receipts.push(vec![4], 29);
        assert_eq!(
            receipts.get(&[1]),
            None,
            "evicted read context remained confirmed"
        );
        assert_eq!(receipts.get(&[3]), Some(17));
    }

    #[test]
    fn bounded_index_matches_vector_specification_across_all_short_histories() {
        // Every length-seven history over three contexts; values differ on
        // repeated contexts. Compare every lookup and the representation after
        // every prefix at capacities including zero and repeated eviction.
        for capacity in 0..=4 {
            for history in 0..3usize.pow(7) {
                let mut encoded = history;
                let mut receipts = ReadReceipts::new(capacity);
                let mut vector = Vec::<(Vec<u8>, u64)>::new();
                for step in 0..7 {
                    let context = vec![(encoded % 3) as u8];
                    encoded /= 3;
                    let index = step * 11 + 1;
                    vector.push((context.clone(), index));
                    let discard = vector.len().saturating_sub(capacity);
                    vector.drain(..discard);
                    receipts.push(context, index);
                    assert!(receipts.order.len() <= capacity);
                    assert_eq!(receipts.order.len(), vector.len());
                    assert_eq!(
                        receipts.indices.values().map(VecDeque::len).sum::<usize>(),
                        vector.len()
                    );
                    assert!(receipts.indices.values().all(|v| !v.is_empty()));
                    for key in 0..=3 {
                        let expected = vector
                            .iter()
                            .find(|(ctx, _)| ctx == &[key])
                            .map(|(_, at)| *at);
                        assert_eq!(
                            receipts.get(&[key]),
                            expected,
                            "indexed receipt disagreed with first retained exact context"
                        );
                    }
                }
            }
        }
    }
}
