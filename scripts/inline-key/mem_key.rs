//! Experimental private key representation; public and persisted keys stay bytes.
#![forbid(unsafe_code)]

use std::borrow::Borrow;
use std::cmp::Ordering;

const INLINE_CAPACITY: usize = 40;

#[derive(Clone, Debug)]
enum Representation {
    Inline {
        len: u8,
        bytes: [u8; INLINE_CAPACITY],
    },
    Heap(Vec<u8>),
}

/// Construction is private to this module so inline lengths always fit the buffer.
#[derive(Clone, Debug)]
pub(crate) struct MapKey(Representation);

impl MapKey {
    pub(crate) fn from_slice(key: &[u8]) -> Self {
        if key.len() <= INLINE_CAPACITY {
            let mut bytes = [0; INLINE_CAPACITY];
            bytes[..key.len()].copy_from_slice(key);
            Self(Representation::Inline {
                len: key.len() as u8,
                bytes,
            })
        } else {
            Self(Representation::Heap(key.to_vec()))
        }
    }

    pub(crate) fn as_slice(&self) -> &[u8] {
        match &self.0 {
            Representation::Inline { len, bytes } => &bytes[..usize::from(*len)],
            Representation::Heap(bytes) => bytes,
        }
    }
}

impl Borrow<[u8]> for MapKey {
    fn borrow(&self) -> &[u8] {
        self.as_slice()
    }
}

impl PartialEq for MapKey {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl Eq for MapKey {}

impl PartialOrd for MapKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MapKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_slice().cmp(other.as_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_length_round_trips_and_owns_its_input() {
        for len in 0..=2048 {
            let mut input: Vec<u8> = (0..len).map(|i| (i * 113 + len) as u8).collect();
            let expected = input.clone();
            let key = MapKey::from_slice(&input);
            input.fill(0);
            assert_eq!(key.as_slice(), expected);
            assert_eq!(key.clone().as_slice(), expected);
            assert_eq!(matches!(key.0, Representation::Inline { .. }), len <= 40);
            let borrowed: &[u8] = key.borrow();
            assert_eq!(borrowed, expected);
        }
    }

    #[test]
    fn padding_and_representation_are_unobservable() {
        for len in 0..=40 {
            let bytes: Vec<u8> = (0..len).map(|x| (x * 71) as u8).collect();
            let ordinary = MapKey::from_slice(&bytes);
            let heap = MapKey(Representation::Heap(bytes.clone()));
            let mut padded = [0xa5; 40];
            padded[..len].copy_from_slice(&bytes);
            let padded = MapKey(Representation::Inline {
                len: len as u8,
                bytes: padded,
            });
            assert_eq!(ordinary, heap);
            assert_eq!(heap, padded);
            assert_eq!(ordinary.cmp(&padded), Ordering::Equal);
            assert_eq!(ordinary.partial_cmp(&heap), Some(Ordering::Equal));
        }
    }

    #[test]
    fn order_is_byte_lexicographic_across_the_inline_boundary() {
        let mut keys = vec![vec![]];
        for len in [1, 2, 27, 35, 39, 40, 41, 42, 64, 128] {
            for byte in [0, 1, 127, 128, 254, 255] {
                keys.push(vec![byte; len]);
                let mut key = vec![byte; len];
                key[len - 1] = 255 - byte;
                keys.push(key);
            }
        }
        for a in &keys {
            for b in &keys {
                let x = MapKey::from_slice(a);
                let y = MapKey::from_slice(b);
                assert_eq!(x.cmp(&y), a.cmp(b));
                assert_eq!(x == y, a == b);
            }
        }
        let mut represented: Vec<_> = keys.iter().map(|k| MapKey::from_slice(k)).collect();
        represented.sort();
        keys.sort();
        assert_eq!(
            represented
                .iter()
                .map(|k| k.as_slice().to_vec())
                .collect::<Vec<_>>(),
            keys
        );
    }
}
