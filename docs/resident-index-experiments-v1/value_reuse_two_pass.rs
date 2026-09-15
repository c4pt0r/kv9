//! Reuse the live value allocation on overwrite, respecting persistent snapshots.

use super::CfMap;

// Vec references are intentional: clone_from reuses capacity for equal-sized
// values while retaining the full Vec clone semantics on a shared entry.
pub(super) fn put(map: &mut CfMap, key: &Vec<u8>, value: &Vec<u8>) {
    if let Some(existing) = map.get_mut(key.as_slice()) {
        existing.clone_from(value);
    } else {
        map.insert_mut(key.clone(), value.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ColumnFamily, Mutation, WriteBatch};
    use std::collections::BTreeMap;

    #[test]
    fn overwrites_preserve_owned_snapshots_and_match_ordered_map() {
        let mut map = CfMap::new_sync();
        let mut expected = BTreeMap::new();
        let mut snapshots = Vec::new();
        for step in 0u32..512 {
            if step % 17 == 0 {
                snapshots.push((map.clone(), expected.clone()));
            }
            let key = (step.wrapping_mul(71) % 127).to_be_bytes().to_vec();
            let value = vec![(step % 251) as u8; [0, 1, 128, 4096][step as usize % 4]];
            put(&mut map, &key, &value);
            expected.insert(key.clone(), value);
            if step % 11 == 0 {
                map.remove_mut(&key);
                expected.remove(&key);
            }
            assert!(map.iter().eq(expected.iter()));
            for (snapshot, old) in &snapshots {
                assert!(snapshot.iter().eq(old.iter()));
            }
        }
        put(&mut map, &vec![], &vec![]);
        assert_eq!(map.get(&[] as &[u8]), Some(&vec![]));
    }

    #[test]
    #[ignore = "local CPU experiment; requires KV9_INDEX_CORPUS and explicit output capture"]
    fn retained_corpus_microbenchmark() {
        use std::hint::black_box;
        use std::time::Instant;
        fn apply(map: &mut CfMap, m: &Mutation, reuse: bool) {
            match m {
                Mutation::Put { key, value, .. } => {
                    if reuse {
                        put(map, key, value);
                    } else {
                        map.insert_mut(key.clone(), value.clone());
                    }
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
            .all(|m| match m {
                Mutation::Put { cf, .. } | Mutation::Delete { cf, .. } =>
                    *cf == ColumnFamily::Default,
            }));
        let original_batches = batches;
        for (workload, cold, unique) in [
            ("retained_overwrite", false, false),
            ("retained_cold_start", true, false),
            ("unique_insert", true, true),
        ] {
            let mut batches = original_batches.clone();
            if unique {
                let mut ordinal = 0u64;
                for batch in &mut batches {
                    for m in &mut batch.mutations {
                        let key = match m {
                            Mutation::Put { key, .. } | Mutation::Delete { key, .. } => key,
                        };
                        let start = key.len().checked_sub(8).unwrap();
                        key[start..].copy_from_slice(
                            &ordinal.wrapping_mul(0x9e3779b97f4a7c15).to_be_bytes(),
                        );
                        ordinal += 1;
                    }
                }
            }
            for pinned in [false, true] {
                for reuse in [false, true, true, false] {
                    let mut map = CfMap::new_sync();
                    if !cold {
                        for batch in &batches {
                            for m in batch.mutations() {
                                apply(&mut map, m, false);
                            }
                        }
                    }
                    let mut durations = Vec::new();
                    for _ in 0..12 {
                        if cold {
                            map = CfMap::new_sync();
                        }
                        for batch in &batches {
                            let snapshot = pinned.then(|| map.clone());
                            let start = Instant::now();
                            for m in batch.mutations() {
                                apply(&mut map, m, reuse);
                            }
                            durations.push(start.elapsed().as_nanos() as u64);
                            black_box(&map);
                            black_box(&snapshot);
                        }
                    }
                    durations.sort_unstable();
                    let sum: u64 = durations.iter().sum();
                    println!(
                        "VALUE_REUSE_MICRO={}",
                        serde_json::json!({
                            "workload": workload,
                            "cold_start_per_pass": cold, "pinned_snapshot": pinned, "reuse": reuse,
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
}
