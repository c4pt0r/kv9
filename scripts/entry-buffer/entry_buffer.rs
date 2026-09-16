//! Experimental immutable key/value bytes with a private, checked key boundary.
#![forbid(unsafe_code)]

use std::borrow::Borrow;
use std::cmp::Ordering;

#[derive(Clone, Debug)]
pub(crate) struct EntryBuffer {
    bytes: Box<[u8]>,
    key_len: usize,
}

impl EntryBuffer {
    pub(crate) fn checked_len(key_len: usize, value_len: usize) -> Option<usize> {
        key_len
            .checked_add(value_len)
            .filter(|total| *total <= isize::MAX as usize)
    }

    /// The engine checks every Put length before changing any column family.
    /// Allocation failure follows ordinary Vec/Box behavior; no unsafe storage
    /// or partially initialized bytes are exposed.
    pub(crate) fn from_slices(key: &[u8], value: &[u8]) -> Self {
        let total = Self::checked_len(key.len(), value.len())
            .expect("entry key/value lengths must fit an addressable buffer");
        let mut bytes = Vec::with_capacity(total);
        bytes.extend_from_slice(key);
        bytes.extend_from_slice(value);
        Self {
            bytes: bytes.into_boxed_slice(),
            key_len: key.len(),
        }
    }

    pub(crate) fn key(&self) -> &[u8] {
        &self.bytes[..self.key_len]
    }

    pub(crate) fn value(&self) -> &[u8] {
        &self.bytes[self.key_len..]
    }
}

impl Borrow<[u8]> for EntryBuffer {
    fn borrow(&self) -> &[u8] {
        self.key()
    }
}
impl PartialEq for EntryBuffer {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}
impl Eq for EntryBuffer {}
impl PartialOrd for EntryBuffer {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for EntryBuffer {
    fn cmp(&self, other: &Self) -> Ordering {
        self.key().cmp(other.key())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_lengths_reject_overflow_without_allocating() {
        let max = isize::MAX as usize;
        for (key, value, expected) in [
            (0, 0, Some(0)),
            (max, 0, Some(max)),
            (max - 1, 1, Some(max)),
            (max, 1, None),
            (usize::MAX, 0, None),
            (usize::MAX, 1, None),
            (1, usize::MAX, None),
        ] {
            assert_eq!(EntryBuffer::checked_len(key, value), expected);
        }
    }

    #[test]
    fn every_boundary_round_trips_and_owns_both_inputs() {
        for key_len in 0..=2048 {
            for value_len in [0, 1, 7, 128, 513] {
                let mut key: Vec<u8> = (0..key_len).map(|n| (n * 113 + key_len) as u8).collect();
                let mut value: Vec<u8> =
                    (0..value_len).map(|n| (n * 71 + value_len) as u8).collect();
                let expected = (key.clone(), value.clone());
                let entry = EntryBuffer::from_slices(&key, &value);
                key.fill(0);
                value.fill(0);
                assert_eq!(entry.key(), expected.0);
                assert_eq!(entry.value(), expected.1);
                let copy = entry.clone();
                drop(entry);
                assert_eq!(copy.key(), expected.0);
                assert_eq!(copy.value(), expected.1);
                let borrowed: &[u8] = copy.borrow();
                assert_eq!(borrowed, expected.0);
            }
        }
    }

    #[test]
    fn identical_bytes_with_different_boundaries_are_distinct_keys() {
        for boundary in 0..=128 {
            let bytes: Vec<u8> = (0..128).map(|n| (n * 71) as u8).collect();
            let a = EntryBuffer::from_slices(&bytes[..boundary], &bytes[boundary..]);
            assert_eq!(a.bytes.as_ref(), bytes);
            for other in 0..=128 {
                let b = EntryBuffer::from_slices(&bytes[..other], &bytes[other..]);
                assert_eq!(a == b, boundary == other);
                assert_eq!(a.cmp(&b), bytes[..boundary].cmp(&bytes[..other]));
            }
        }
    }

    #[test]
    fn values_do_not_affect_key_equality_or_order() {
        let mut keys = vec![vec![]];
        for len in [1, 27, 35, 39, 40, 41, 128, 1024] {
            for byte in [0, 1, 127, 128, 255] {
                keys.push(vec![byte; len]);
            }
        }
        for a in &keys {
            let a1 = EntryBuffer::from_slices(a, b"\xff\0payload");
            let a2 = EntryBuffer::from_slices(a, b"");
            assert_eq!(a1, a2);
            assert_eq!(a1.partial_cmp(&a2), Some(Ordering::Equal));
            for b in &keys {
                let b1 = EntryBuffer::from_slices(b, b"\0\xffdifferent");
                assert_eq!(a1.cmp(&b1), a.cmp(b));
                assert_eq!(a1 == b1, a == b);
            }
        }
    }
}
