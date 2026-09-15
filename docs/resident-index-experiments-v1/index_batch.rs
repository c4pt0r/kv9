//! Experimental ordering of mutations inside one atomic resident-index update.
//!
//! The WAL keeps the original batch. Only the final mutation of each (CF, key)
//! matters to the resident index; intermediate states must never be published.

use crate::{ColumnFamily, Mutation};

const MIN_MUTATIONS: usize = 128;
const MAX_MUTATIONS: usize = 8192;

fn physical_key(mutation: &Mutation) -> (u8, &[u8]) {
    let (cf, key) = match mutation {
        Mutation::Put { cf, key, .. } | Mutation::Delete { cf, key } => (cf, key),
    };
    let cf = match cf {
        ColumnFamily::Default => 0,
        ColumnFamily::Lock => 1,
        ColumnFamily::Write => 2,
    };
    (cf, key)
}

/// Visit the last mutation of each physical key in key order. Small or oversized
/// batches retain their original order without allocating scratch space.
///
/// Scratch is at most 8192 indices (64 KiB on a 64-bit target). The unstable
/// sort allocates no additional buffer. The descending ordinal tie-break makes
/// the last original mutation the first member of each equal-key run, which
/// `dedup_by` retains. No key/value bytes or caller-owned batch are modified.
pub(crate) fn visit(mutations: &[Mutation], mut apply: impl FnMut(&Mutation)) {
    if !(MIN_MUTATIONS..=MAX_MUTATIONS).contains(&mutations.len()) {
        for mutation in mutations {
            apply(mutation);
        }
        return;
    }
    let mut order: Vec<usize> = (0..mutations.len()).collect();
    order.sort_unstable_by(|&a, &b| {
        physical_key(&mutations[a])
            .cmp(&physical_key(&mutations[b]))
            .then_with(|| b.cmp(&a))
    });
    order.dedup_by(|a, b| physical_key(&mutations[*a]) == physical_key(&mutations[*b]));
    for index in order {
        apply(&mutations[index]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WriteBatch;
    use std::collections::BTreeMap;

    fn update(map: &mut BTreeMap<(u8, Vec<u8>), Vec<u8>>, m: &Mutation) {
        let (cf, key) = physical_key(m);
        match m {
            Mutation::Put { value, .. } => {
                map.insert((cf, key.to_vec()), value.clone());
            }
            Mutation::Delete { .. } => {
                map.remove(&(cf, key.to_vec()));
            }
        }
    }

    #[test]
    fn matches_sequential_updates_for_every_short_history_and_bounded_fallbacks() {
        // Exhaust every four-operation history over two CFs, an empty key,
        // empty/nonempty values and deletion. Padding enters the optimized path.
        let ops = [
            Mutation::Put {
                cf: ColumnFamily::Default,
                key: vec![],
                value: vec![],
            },
            Mutation::Put {
                cf: ColumnFamily::Default,
                key: vec![],
                value: vec![1],
            },
            Mutation::Delete {
                cf: ColumnFamily::Default,
                key: vec![],
            },
            Mutation::Put {
                cf: ColumnFamily::Lock,
                key: vec![],
                value: vec![2],
            },
            Mutation::Delete {
                cf: ColumnFamily::Lock,
                key: vec![],
            },
            Mutation::Put {
                cf: ColumnFamily::Write,
                key: vec![255],
                value: vec![],
            },
        ];
        for n in 0..6usize.pow(4) {
            let mut mutations = vec![ops[0].clone(); MIN_MUTATIONS];
            let mut digits = n;
            for _ in 0..4 {
                mutations.push(ops[digits % 6].clone());
                digits /= 6;
            }
            let mut expected = BTreeMap::from([((0, vec![]), vec![9]), ((1, vec![]), vec![8])]);
            let mut actual = expected.clone();
            for m in &mutations {
                update(&mut expected, m);
            }
            visit(&mutations, |m| update(&mut actual, m));
            assert_eq!(actual, expected, "history {n}");
        }
        for size in [
            0,
            1,
            MIN_MUTATIONS - 1,
            MIN_MUTATIONS,
            MAX_MUTATIONS,
            MAX_MUTATIONS + 1,
        ] {
            let mutations = vec![ops[0].clone(); size];
            let mut visits = 0;
            visit(&mutations, |_| visits += 1);
            assert_eq!(
                visits,
                if (MIN_MUTATIONS..=MAX_MUTATIONS).contains(&size) {
                    1
                } else {
                    size
                }
            );
        }
    }

    #[test]
    fn unique_keys_keep_every_mutation_and_duplicates_keep_the_last_value() {
        let mut batch = WriteBatch::new();
        for key in (0u32..2048).rev() {
            batch.put(ColumnFamily::Default, key.to_be_bytes().to_vec(), vec![1]);
        }
        let mut visited = Vec::new();
        visit(batch.mutations(), |m| {
            visited.push(physical_key(m).1.to_vec())
        });
        assert_eq!(visited.len(), 2048);
        assert!(visited.windows(2).all(|w| w[0] < w[1]));
        for key in 0u32..2048 {
            batch.delete(ColumnFamily::Default, key.to_be_bytes().to_vec());
            batch.put(ColumnFamily::Default, key.to_be_bytes().to_vec(), vec![2]);
        }
        let mut actual = BTreeMap::new();
        visit(batch.mutations(), |m| update(&mut actual, m));
        assert_eq!(actual.len(), 2048);
        assert!(actual.values().all(|value| value == &[2]));
    }

    #[test]
    #[ignore = "local CPU experiment; requires KV9_INDEX_CORPUS and explicit output capture"]
    fn retained_corpus_microbenchmark() {
        use rpds::RedBlackTreeMapSync;
        use std::hint::black_box;
        use std::time::Instant;
        type Map = RedBlackTreeMapSync<Vec<u8>, Vec<u8>>;
        fn apply(map: &mut Map, m: &Mutation) {
            match m {
                Mutation::Put { key, value, .. } => {
                    map.insert_mut(key.clone(), value.clone());
                }
                Mutation::Delete { key, .. } => {
                    map.remove_mut(key);
                }
            }
        }
        let data =
            std::fs::read(std::env::var("KV9_INDEX_CORPUS").expect("KV9_INDEX_CORPUS")).unwrap();
        let mut remaining = data.as_slice();
        let mut batches = Vec::new();
        while !remaining.is_empty() {
            let size = u32::from_le_bytes(remaining[..4].try_into().unwrap()) as usize;
            batches.push(crate::wal::decode_batch(&remaining[4..4 + size]).unwrap());
            remaining = &remaining[4 + size..];
        }
        assert!(batches
            .iter()
            .flat_map(WriteBatch::mutations)
            .all(|m| physical_key(m).0 == 0));
        // One source, same allocator, identical inputs and warm-up. Only index
        // visitation changes. Snapshot copying is an explicitly separate case.
        for pinned in [false, true] {
            for optimized in [false, true, true, false] {
                let mut map = Map::new_sync();
                for batch in &batches {
                    for m in batch.mutations() {
                        apply(&mut map, m);
                    }
                }
                let mut durations = Vec::new();
                for _ in 0..12 {
                    for batch in &batches {
                        let snapshot = pinned.then(|| map.clone());
                        let start = Instant::now();
                        if optimized {
                            visit(batch.mutations(), |m| apply(&mut map, m));
                        } else {
                            for m in batch.mutations() {
                                apply(&mut map, m);
                            }
                        }
                        durations.push(start.elapsed().as_nanos() as u64);
                        black_box(&map);
                        black_box(&snapshot);
                    }
                }
                durations.sort_unstable();
                let sum: u64 = durations.iter().sum();
                println!(
                    "INDEX_MICRO={}",
                    serde_json::json!({
                        "pinned_snapshot": pinned, "optimized": optimized,
                        "batches": durations.len(), "total_ns": sum,
                        "mean_ns": sum as f64 / durations.len() as f64,
                        "p99_ns": durations[(durations.len() * 99).div_ceil(100) - 1],
                        "final_items": map.size(),
                        "scope": "resident-map CPU only; not a database throughput or client latency result"
                    })
                );
            }
        }
    }
}
