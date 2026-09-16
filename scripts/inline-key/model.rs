//! Independent ordered-byte model, run unchanged against baseline and candidate.
use kv9_common::AppliedPosition;
use kv9_engine::{
    ColumnFamily as Cf, Durability, DurableAppliedPosition, Engine, MemEngine, Mutation, ReadView,
    ReplicatedEngine, ScanEntry, WriteBatch,
};
use std::collections::BTreeMap;

type Model = [BTreeMap<Vec<u8>, Vec<u8>>; 3];
fn slot(cf: Cf) -> usize {
    match cf {
        Cf::Default => 0,
        Cf::Lock => 1,
        Cf::Write => 2,
    }
}
fn keys() -> Vec<Vec<u8>> {
    let mut keys = vec![vec![]];
    for len in [1, 2, 27, 35, 39, 40, 41, 42, 64, 128, 1024] {
        for byte in [0, 1, 127, 128, 254, 255] {
            keys.push(vec![byte; len]);
            let mut key = vec![byte; len];
            key[len - 1] = 255 - byte;
            keys.push(key);
        }
    }
    keys.extend([
        b"\0kv9\0manifest_pair".to_vec(),
        b"\0kv9\0retention_v1\0owner".to_vec(),
        b"\0kv9\0retention_v2\0new".to_vec(),
    ]);
    keys.sort();
    keys.dedup();
    keys
}
fn upper(keys: &[Vec<u8>]) -> Vec<u8> {
    let mut end = keys.last().unwrap().clone();
    end.push(0);
    end
}
fn apply(model: &mut Model, batch: &WriteBatch) {
    for op in batch.mutations() {
        match op {
            Mutation::Put { cf, key, value } => {
                model[slot(*cf)].insert(key.clone(), value.clone());
            }
            Mutation::Delete { cf, key } => {
                model[slot(*cf)].remove(key);
            }
        }
    }
}
fn rows(map: &BTreeMap<Vec<u8>, Vec<u8>>, start: &[u8], end: &[u8]) -> Vec<ScanEntry> {
    // Deliberately use a linear filter, independent of rpds range navigation.
    map.iter()
        .filter(|(key, _)| start <= key.as_slice() && key.as_slice() < end)
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}
fn check(view: &dyn ReadView, model: &Model, keys: &[Vec<u8>], epoch: usize) {
    let end = upper(keys);
    for cf in Cf::ALL {
        let map = &model[slot(cf)];
        for key in keys {
            assert_eq!(view.get(cf, key).unwrap().as_ref(), map.get(key));
            assert_eq!(
                view.get_resident(cf, key),
                Some(map.get(key).map(Vec::as_slice))
            );
        }
        let expected = rows(map, b"", &end);
        assert_eq!(
            view.iter(cf, b"", &end)
                .unwrap()
                .collect::<kv9_common::Result<Vec<_>>>()
                .unwrap(),
            expected
        );
        assert_eq!(
            view.iter_rev(cf, b"", &end)
                .unwrap()
                .collect::<kv9_common::Result<Vec<_>>>()
                .unwrap(),
            expected.iter().rev().cloned().collect::<Vec<_>>()
        );
        assert_eq!(view.scan(cf, b"", &end, usize::MAX).unwrap(), expected);
        for j in 0..8 {
            let a = (epoch * 37 + j * 13) % keys.len();
            let b = (epoch * 17 + j * 29) % keys.len();
            let (start, end) = (&keys[a.min(b)], &keys[a.max(b)]);
            let selected = rows(map, start, end);
            assert_eq!(
                view.scan(cf, start, end, j).unwrap(),
                selected.iter().take(j).cloned().collect::<Vec<_>>()
            );
            assert_eq!(
                view.iter(cf, start, end)
                    .unwrap()
                    .collect::<kv9_common::Result<Vec<_>>>()
                    .unwrap(),
                selected
            );
            assert_eq!(
                view.iter_rev(cf, start, end)
                    .unwrap()
                    .collect::<kv9_common::Result<Vec<_>>>()
                    .unwrap(),
                selected.iter().rev().cloned().collect::<Vec<_>>()
            );
            let target = &keys[(a + b) % keys.len()];
            let predecessor = map
                .iter()
                .rev()
                .find(|(key, _)| key.as_slice() <= target.as_slice())
                .map(|(key, value)| (key.clone(), value.clone()));
            assert_eq!(view.seek_le(cf, target).unwrap(), predecessor);
        }
    }
}

#[test]
fn generated_histories_preserve_all_column_families_and_old_views() {
    let engine = MemEngine::new();
    let keys = keys();
    let mut model: Model = Default::default();
    let mut snapshots = Vec::new();
    let mut seed = 0x67493be8ac1251u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut revision = 0;
    for epoch in 0..240 {
        if epoch % 30 == 0 {
            snapshots.push((engine.try_resident_snapshot().unwrap(), model.clone()));
        }
        let mut batch = WriteBatch::new();
        for n in 0..(epoch % 9) {
            let key = keys[(next() as usize) % keys.len()].clone();
            let cf = Cf::ALL[(next() as usize) % 3];
            if next() % 4 == 0 {
                batch.delete(cf, key);
            } else {
                batch.put(cf, key, vec![(epoch + n) as u8; (next() % 129) as usize]);
            }
        }
        // Repeated mutations in the same batch must retain their order.
        if epoch % 13 == 0 {
            let key = keys[(epoch * 17) % keys.len()].clone();
            batch.put(Cf::Default, key.clone(), b"first".to_vec());
            batch.delete(Cf::Default, key.clone());
            batch.put(Cf::Default, key, b"last".to_vec());
        }
        let affects_revision = batch.mutations().iter().any(|m| {
            let (cf, key) = match m {
                Mutation::Put { cf, key, .. } | Mutation::Delete { cf, key } => (cf, key),
            };
            *cf != Cf::Default
                || !(key.starts_with(b"\0kv9\0manifest_")
                    || key.starts_with(b"\0kv9\0retention_v1\0"))
        });
        apply(&mut model, &batch);
        let at = AppliedPosition {
            term: (240 - epoch) as u64,
            index: (epoch * 3 + 1) as u64,
        };
        engine.write_applied(batch, at).unwrap();
        revision += u64::from(affects_revision);
        assert_eq!(engine.volatile_applied_position(), Some(at));
        assert_eq!(engine.data_revision(), revision);
        assert_eq!(
            engine.applied_position().unwrap(),
            DurableAppliedPosition::Volatile
        );
        assert_eq!(engine.durability(), Durability::Volatile);
        if epoch % 11 == 0 {
            for refused in [at.index, at.index - 1] {
                let mut batch = WriteBatch::new();
                for cf in Cf::ALL {
                    batch.put(cf, keys[epoch % keys.len()].clone(), b"refused".to_vec());
                }
                assert!(engine
                    .write_applied(
                        batch,
                        AppliedPosition {
                            term: 999,
                            index: refused
                        }
                    )
                    .is_err());
                assert_eq!(engine.volatile_applied_position(), Some(at));
                assert_eq!(engine.data_revision(), revision);
            }
        }
        check(engine.snapshot().unwrap().as_ref(), &model, &keys, epoch);
        for (view, old) in &snapshots {
            check(view.as_ref(), old, &keys, epoch);
        }
    }
}

#[test]
fn range_deletion_checksum_and_iterator_lifetimes_preserve_bytes() {
    let engine = MemEngine::new();
    let keys = keys();
    let mut model: Model = Default::default();
    let mut batch = WriteBatch::new();
    for cf in Cf::ALL {
        for key in &keys {
            batch.put(cf, key.clone(), key.iter().rev().copied().collect());
        }
    }
    apply(&mut model, &batch);
    engine.write(batch).unwrap();
    let before = engine.snapshot().unwrap();
    let before_model = model.clone();
    let end = upper(&keys);
    // Bounds may be dropped before the stream is consumed.
    let start_buffer = vec![];
    let end_buffer = end.clone();
    let retained = before
        .iter(Cf::Default, &start_buffer, &end_buffer)
        .unwrap();
    drop(start_buffer);
    drop(end_buffer);
    for cf in Cf::ALL {
        for (start, end) in [
            (&keys[0], &keys[0]),
            (&keys[13], &keys[80]),
            (&keys[80], &end),
        ] {
            let expected = rows(&model[slot(cf)], start, end);
            let checksum = expected
                .iter()
                .flat_map(|(k, v)| k.iter().chain(v))
                .fold(0xcbf29ce484222325u64, |h, b| {
                    (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
                });
            assert_eq!(engine.checksum(cf, start, end).unwrap(), checksum);
            engine.delete_range(cf, start, end).unwrap();
            model[slot(cf)].retain(|key, _| !(start <= key && key < end));
            check(engine.snapshot().unwrap().as_ref(), &model, &keys, 0);
            check(before.as_ref(), &before_model, &keys, 0);
        }
    }
    assert_eq!(
        retained.collect::<kv9_common::Result<Vec<_>>>().unwrap(),
        rows(&before_model[0], b"", &end)
    );
    assert_eq!(engine.data_revision(), 0);
    assert_eq!(engine.volatile_applied_position(), None);
}

#[test]
fn metadata_only_batches_and_threshold_keys_preserve_revision_rules() {
    let engine = MemEngine::new();
    for (n, key) in [
        b"\0kv9\0manifest_pair".as_slice(),
        b"\0kv9\0retention_v1\0owner",
    ]
    .iter()
    .enumerate()
    {
        let mut batch = WriteBatch::new();
        batch.put(Cf::Default, key.to_vec(), vec![]);
        engine
            .write_applied(
                batch,
                AppliedPosition {
                    term: 1,
                    index: n as u64 + 1,
                },
            )
            .unwrap();
        assert_eq!(engine.data_revision(), 0);
    }
    let mut batch = WriteBatch::new();
    for cf in Cf::ALL {
        for n in [39, 40, 41] {
            batch.put(cf, vec![0; n], vec![n as u8]);
        }
    }
    engine
        .write_applied(batch, AppliedPosition { term: 0, index: 3 })
        .unwrap();
    assert_eq!(engine.data_revision(), 1);
    let mut batch = WriteBatch::new();
    batch.delete(Cf::Lock, b"\0kv9\0manifest_pair".to_vec());
    engine
        .write_applied(batch, AppliedPosition { term: 0, index: 4 })
        .unwrap();
    assert_eq!(engine.data_revision(), 2);
}
