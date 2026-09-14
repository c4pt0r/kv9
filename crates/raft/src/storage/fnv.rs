//! Legacy Raft WAL checksums, with independent four-record computation.
//!
//! The interleaved kernel borrows four bodies and allocates no memory. Each
//! accumulator consumes only its own bytes, in order, then its scalar tail.
//! This module is experimental and is not wired into the database writer yet.

const OFFSET: u32 = 0x811c9dc5;
const PRIME: u32 = 0x01000193;

#[inline]
fn finish(mut hash: u32, bytes: &[u8]) -> u32 {
    for &byte in bytes {
        hash = (hash ^ u32::from(byte)).wrapping_mul(PRIME);
    }
    hash
}

/// The existing bytewise FNV-1a recurrence, unchanged.
#[inline]
pub(super) fn fnv1a(bytes: &[u8]) -> u32 {
    finish(OFFSET, bytes)
}

#[inline]
pub(super) fn fnv1a_four(inputs: [&[u8]; 4]) -> [u32; 4] {
    fnv1a_four_from(inputs, [OFFSET; 4])
}

/// Compute four independent continuations using constant auxiliary space.
/// Empty or unequal inputs are handled by the original scalar recurrence.
#[inline]
pub(super) fn fnv1a_four_from(inputs: [&[u8]; 4], initial: [u32; 4]) -> [u32; 4] {
    let [a, b, c, d] = inputs;
    let common = a.len().min(b.len()).min(c.len()).min(d.len());
    let [mut h0, mut h1, mut h2, mut h3] = initial;
    for i in 0..common {
        h0 = (h0 ^ u32::from(a[i])).wrapping_mul(PRIME);
        h1 = (h1 ^ u32::from(b[i])).wrapping_mul(PRIME);
        h2 = (h2 ^ u32::from(c[i])).wrapping_mul(PRIME);
        h3 = (h3 ^ u32::from(d[i])).wrapping_mul(PRIME);
    }
    [
        finish(h0, &a[common..]),
        finish(h1, &b[common..]),
        finish(h2, &c[common..]),
        finish(h3, &d[common..]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    // An independent copy of the existing writer recurrence; source binding
    // also checks it against the committed storage.rs before qualification.
    fn old_from(mut hash: u32, bytes: &[u8]) -> u32 {
        for &b in bytes {
            hash ^= u32::from(b);
            hash = hash.wrapping_mul(0x01000193);
        }
        hash
    }

    fn bytes(len: usize, seed: u32) -> Vec<u8> {
        let mut state = seed;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state as u8
            })
            .collect()
    }

    #[test]
    fn all_short_lengths_and_lane_values_match_the_original() {
        for len in 0..=257 {
            let data = std::array::from_fn::<_, 4, _>(|lane| bytes(len, 71 + lane as u32));
            let inputs = data.each_ref().map(Vec::as_slice);
            assert_eq!(fnv1a_four(inputs), inputs.map(|v| old_from(OFFSET, v)));
            for input in inputs {
                assert_eq!(fnv1a(input), old_from(OFFSET, input));
            }
        }
    }

    #[test]
    fn empty_ragged_large_and_arbitrary_state_continuations_match() {
        let lengths = [0, 1, 7, 8, 9, 204, 205, 10599, 10600, 65536];
        let data = std::array::from_fn::<_, 4, _>(|lane| bytes(65536, 101 + lane as u32));
        let states = [0, u32::MAX, OFFSET, 0x01020304];
        // Exercise each possible shortest lane and nonzero tails, including
        // an empty common prefix with a large remaining body.
        for &short in &lengths {
            for &long in &lengths {
                for lane in 0..4 {
                    let sizes =
                        std::array::from_fn::<_, 4, _>(|i| if i == lane { short } else { long });
                    let inputs = std::array::from_fn::<_, 4, _>(|i| &data[i][..sizes[i]]);
                    assert_eq!(
                        fnv1a_four_from(inputs, states),
                        std::array::from_fn(|i| old_from(states[i], inputs[i]))
                    );
                }
            }
        }
    }

    #[test]
    fn continuations_preserve_fragment_boundaries_and_lane_identity() {
        let data = std::array::from_fn::<_, 4, _>(|i| bytes(10600 + i, 501 + i as u32));
        for split in [0, 1, 205, 4096, 10599, 10600] {
            let prefix = data.each_ref().map(|v| &v[..split]);
            let tail = data.each_ref().map(|v| &v[split..]);
            let actual = fnv1a_four_from(tail, fnv1a_four(prefix));
            assert_eq!(actual, data.each_ref().map(|v| old_from(OFFSET, v)));
            let mut permuted = tail;
            permuted.swap(0, 3);
            let mut states = fnv1a_four(prefix);
            states.swap(0, 3);
            let mut expected = actual;
            expected.swap(0, 3);
            assert_eq!(fnv1a_four_from(permuted, states), expected);
        }
    }
}
