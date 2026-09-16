//! Actual public-engine checks for payloads stored in private map keys.
use kv9_common::AppliedPosition;
use kv9_engine::{ColumnFamily as Cf, Engine, MemEngine, ReplicatedEngine, WriteBatch};
use std::collections::BTreeMap;

fn keys() -> Vec<Vec<u8>> {
    let mut out = vec![vec![], b"a".to_vec(), b"ab".to_vec(), b"abc".to_vec()];
    for len in [1, 27, 35, 39, 40, 41, 128, 1024] {
        for byte in [0, 127, 255] {
            out.push(vec![byte; len]);
        }
    }
    out.sort();
    out.dedup();
    out
}

#[test]
fn same_key_replacement_changes_value_and_retains_every_old_version() {
    let engine = MemEngine::new();
    let keys = keys();
    let mut end = keys.last().unwrap().clone();
    end.push(0);
    let mut versions = Vec::new();
    for (generation, length) in [0, 1, 7, 128, 65536, 2, 0].into_iter().enumerate() {
        let mut batch = WriteBatch::new();
        let mut model = BTreeMap::new();
        for (ordinal, key) in keys.iter().enumerate() {
            let value: Vec<u8> = (0..length)
                .map(|i| (i + ordinal + generation * 71) as u8)
                .collect();
            for cf in Cf::ALL {
                batch.put(cf, key.clone(), value.clone());
            }
            model.insert(key.clone(), value);
        }
        engine
            .write_applied(
                batch,
                AppliedPosition {
                    term: 1,
                    index: generation as u64 + 1,
                },
            )
            .unwrap();
        versions.push((engine.try_resident_snapshot().unwrap(), model));
        for (view, model) in &versions {
            let expected: Vec<_> = model.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            for cf in Cf::ALL {
                assert_eq!(view.scan(cf, b"", &end, usize::MAX).unwrap(), expected);
                assert_eq!(
                    view.iter_rev(cf, b"", &end)
                        .unwrap()
                        .collect::<kv9_common::Result<Vec<_>>>()
                        .unwrap(),
                    expected.iter().rev().cloned().collect::<Vec<_>>()
                );
                for key in &keys {
                    assert_eq!(view.get(cf, key).unwrap().as_ref(), model.get(key));
                    assert_eq!(
                        view.get_resident(cf, key),
                        Some(model.get(key).map(Vec::as_slice))
                    );
                    assert_eq!(
                        view.seek_le(cf, key).unwrap(),
                        Some((key.clone(), model[key].clone()))
                    );
                }
            }
        }
    }
    let mut remove = WriteBatch::new();
    for cf in Cf::ALL {
        for key in &keys {
            remove.delete(cf, key.clone());
        }
    }
    engine
        .write_applied(
            remove,
            AppliedPosition {
                term: 0,
                index: 100,
            },
        )
        .unwrap();
    for cf in Cf::ALL {
        assert!(engine.scan(cf, b"", &end, usize::MAX).unwrap().is_empty());
    }
    assert!(versions
        .iter()
        .all(|(v, m)| v.get(Cf::Default, b"a").unwrap().as_ref() == m.get(b"a".as_slice())));
}

#[test]
fn split_collisions_are_distinct_and_public_results_own_their_bytes() {
    let engine = MemEngine::new();
    let mut batch = WriteBatch::new();
    for (k, v) in [
        (b"".as_slice(), b"abc".as_slice()),
        (b"a", b"bc"),
        (b"ab", b"c"),
        (b"abc", b""),
    ] {
        batch.put(Cf::Default, k.to_vec(), v.to_vec());
    }
    engine.write(batch).unwrap();
    let view = engine.snapshot().unwrap();
    let mut owned = view.scan(Cf::Default, b"", b"z", 10).unwrap();
    assert_eq!(owned.len(), 4);
    let original = owned.clone();
    for (key, value) in &mut owned {
        key.fill(255);
        value.fill(255);
    }
    assert_eq!(view.scan(Cf::Default, b"", b"z", 10).unwrap(), original);
    engine.delete_range(Cf::Default, b"a", b"abc").unwrap();
    assert_eq!(
        engine.scan(Cf::Default, b"", b"z", 10).unwrap(),
        vec![original[0].clone(), original[3].clone()]
    );
    assert_eq!(view.scan(Cf::Default, b"", b"z", 10).unwrap(), original);
}
