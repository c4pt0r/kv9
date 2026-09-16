use super::*;
use std::collections::BTreeMap;

#[global_allocator]
static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

type Model = BTreeMap<Vec<u8>, Vec<u8>>;

fn check(map: &PackedMap, model: &Model, probes: &[Vec<u8>]) {
    assert_eq!(map.len(), model.len());
    assert_eq!(map.is_empty(), model.is_empty());
    assert_eq!(map.validate().entries, model.len());
    assert_eq!(
        map.iter().collect::<Vec<_>>(),
        model.iter().collect::<Vec<_>>()
    );
    for probe in probes {
        assert_eq!(map.get(probe), model.get(probe));
        assert_eq!(
            map.seek_le(probe),
            model.range(..=probe.clone()).next_back()
        );
    }
    for pair in probes.windows(2) {
        let mut range = map.range(&pair[0], &pair[1]);
        let expected: Vec<_> = if pair[0] < pair[1] {
            model.range(pair[0].clone()..pair[1].clone()).collect()
        } else {
            vec![]
        };
        assert_eq!(range.by_ref().collect::<Vec<_>>(), expected);
        assert_eq!(range.next(), None);
        assert_eq!(range.next(), None);
        assert_eq!(map.range(&pair[0], &pair[1]).take(0).count(), 0);
        assert_eq!(
            map.range(&pair[0], &pair[1]).take(7).collect::<Vec<_>>(),
            expected.into_iter().take(7).collect::<Vec<_>>()
        );
    }
}

fn key(index: u64) -> Vec<u8> {
    match index {
        0 => vec![],
        1 => vec![0],
        2 => vec![0, 0],
        3 => vec![255],
        4 => vec![255, 0],
        _ => {
            let mut key = vec![b'r', 0, 0, 1, (index % 5) as u8];
            key.extend_from_slice(&index.to_be_bytes());
            key
        }
    }
}

fn probes(changed: &[u8]) -> Vec<Vec<u8>> {
    vec![
        vec![],
        vec![0],
        vec![0, 0],
        changed.to_vec(),
        vec![127],
        vec![255],
        vec![255, 0],
        vec![255, 255],
    ]
}

fn random(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

#[test]
fn ordered_split_merge_and_root_contraction_in_both_directions() {
    for insert_reverse in [false, true] {
        for delete_reverse in [false, true] {
            let mut keys: Vec<_> = (0..2049).map(key).collect();
            keys.sort();
            if insert_reverse {
                keys.reverse();
            }
            let mut map = PackedMap::new();
            let mut model = Model::new();
            for (i, key) in keys.iter().enumerate() {
                let value = (i as u64).to_le_bytes().to_vec();
                assert_eq!(
                    map.insert(key.clone(), value.clone()),
                    model.insert(key.clone(), value).is_none()
                );
                check(&map, &model, &probes(key));
            }
            assert!(map.validate().height >= 3);
            let old = map.clone();
            let before = model.clone();
            keys.sort();
            if delete_reverse {
                keys.reverse();
            }
            for key in &keys {
                assert_eq!(map.remove(key), model.remove(key).is_some());
                check(&map, &model, &probes(key));
                check(&old, &before, &probes(key));
                assert!(!map.remove(key));
            }
            assert_eq!(map.validate(), Shape::default());
        }
    }
}

#[test]
fn mixed_histories_preserve_multiple_snapshot_generations() {
    for seed in [1, 71, 0x719af20d] {
        let mut state = seed;
        let mut map = PackedMap::new();
        let mut model = Model::new();
        let mut retained = vec![(map.clone(), model.clone())];
        for step in 0..6000 {
            let number = random(&mut state);
            let key = key((number >> 8) % 2048);
            if number.is_multiple_of(3) {
                assert_eq!(map.remove(&key), model.remove(&key).is_some());
            } else {
                let mut value = vec![(number >> 32) as u8; (number as usize >> 16) % 129];
                if !value.is_empty() {
                    value[0] = (step % 256) as u8;
                }
                assert_eq!(
                    map.insert(key.clone(), value.clone()),
                    model.insert(key.clone(), value).is_none()
                );
            }
            check(&map, &model, &probes(&key));
            for (snapshot, expected) in &retained {
                check(snapshot, expected, &probes(&key));
            }
            if step % 97 == 0 {
                retained.push((map.clone(), model.clone()));
                if retained.len() > 5 {
                    retained.remove(1);
                }
            }
        }
    }
}

fn permutations(values: &mut [u64], start: usize, output: &mut Vec<Vec<u64>>) {
    if start == values.len() {
        output.push(values.to_vec());
        return;
    }
    for i in start..values.len() {
        values.swap(start, i);
        permutations(values, start + 1, output);
        values.swap(start, i);
    }
}

#[test]
fn exhaustive_small_insert_delete_permutations_with_binary_boundaries() {
    let mut orders = Vec::new();
    permutations(&mut [0, 1, 2, 3, 4], 0, &mut orders);
    assert_eq!(orders.len(), 120);
    for insert in &orders {
        for delete in &orders {
            let mut map = PackedMap::new();
            let mut model = Model::new();
            for index in insert {
                let k = key(*index);
                assert!(map.insert(k.clone(), vec![]));
                model.insert(k.clone(), vec![]);
                check(&map, &model, &probes(&k));
            }
            let old = map.clone();
            let before = model.clone();
            for index in delete {
                let k = key(*index);
                assert!(map.remove(&k));
                model.remove(&k);
                check(&map, &model, &probes(&k));
                check(&old, &before, &probes(&k));
            }
        }
    }
}

#[test]
fn snapshots_share_roots_and_unchanged_subtrees() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<PackedMap>();
    let mut map = PackedMap::new();
    for i in 0..1024u64 {
        map.insert(i.to_be_bytes().to_vec(), vec![1]);
    }
    let old = map.clone();
    assert!(Arc::ptr_eq(
        map.root.as_ref().unwrap(),
        old.root.as_ref().unwrap()
    ));
    assert!(!map.remove(b"absent"));
    assert!(Arc::ptr_eq(
        map.root.as_ref().unwrap(),
        old.root.as_ref().unwrap()
    ));
    map.insert(0u64.to_be_bytes().to_vec(), vec![2]);
    assert_eq!(old.get(&0u64.to_be_bytes()), Some(&vec![1]));
    assert_eq!(map.get(&0u64.to_be_bytes()), Some(&vec![2]));
    assert!(!Arc::ptr_eq(
        map.root.as_ref().unwrap(),
        old.root.as_ref().unwrap()
    ));
    let (Body::Branch(current), Body::Branch(previous)) = (
        &map.root.as_deref().unwrap().body,
        &old.root.as_deref().unwrap().body,
    ) else {
        panic!("fixture must have multiple root children")
    };
    assert!(current
        .iter()
        .zip(previous)
        .skip(1)
        .all(|(a, b)| Arc::ptr_eq(&a.node, &b.node)));
    map.validate();
    old.validate();
}

#[test]
fn deep_tree_shuffled_deletion_retains_old_generations() {
    let mut map = PackedMap::new();
    let mut model = Model::new();
    let mut keys: Vec<_> = (0..20000u64).map(|i| i.to_be_bytes().to_vec()).collect();
    for (index, key) in keys.iter().enumerate() {
        map.insert(key.clone(), key.clone());
        model.insert(key.clone(), key.clone());
        if index % 64 == 0 {
            check(&map, &model, &probes(key));
        }
    }
    assert!(map.validate().height >= 4);
    let old = map.clone();
    let before = model.clone();
    let mut state = 71;
    for i in (1..keys.len()).rev() {
        let other = random(&mut state) as usize % (i + 1);
        keys.swap(i, other);
    }
    for (index, key) in keys.iter().enumerate() {
        assert!(map.remove(key));
        model.remove(key);
        if index % 64 == 0 {
            check(&map, &model, &probes(key));
            check(&old, &before, &probes(key));
        }
    }
    check(&map, &model, &probes(&[]));
    check(&old, &before, &probes(&[]));
}

#[test]
fn independent_model_detects_corrupted_separator_and_size() {
    let mut map = PackedMap::new();
    for i in 0..100u64 {
        map.insert(i.to_be_bytes().to_vec(), vec![]);
    }
    let mut wrong_size = map.clone();
    wrong_size.size += 1;
    assert!(std::panic::catch_unwind(|| wrong_size.validate()).is_err());
    let Body::Branch(children) = &mut Arc::make_mut(map.root.as_mut().unwrap()).body else {
        unreachable!()
    };
    children[1].minimum = vec![255];
    assert!(std::panic::catch_unwind(|| map.validate()).is_err());
}

#[test]
fn guarded_suffix_comparison_matches_complete_unsigned_byte_order() {
    let mut keys = vec![vec![]];
    let alphabet = [0, 1, 127, 255];
    let mut last = vec![vec![]];
    for _ in 0..3 {
        let mut next = Vec::new();
        for key in &last {
            for byte in alphabet {
                let mut key = key.clone();
                key.push(byte);
                next.push(key);
            }
        }
        keys.extend(next.clone());
        last = next;
    }
    for reference in &keys {
        for stored in &keys {
            let prefix = common_prefix_len(reference, stored);
            for query in &keys {
                let skip = query_skip(prefix, reference, query);
                assert!(skip <= stored.len() && skip <= query.len());
                assert_eq!(suffix_cmp(stored, query, skip), stored.cmp(query));
            }
        }
    }
}

#[test]
fn prefix_changes_split_merge_and_short_queries_preserve_snapshots() {
    let prefix = vec![127; 24];
    let mut map = PackedMap::new();
    let mut model = Model::new();
    let mut inserted = Vec::new();
    for i in 0..1025u64 {
        let mut key = prefix.clone();
        key.extend_from_slice(&i.to_be_bytes());
        map.insert(key.clone(), vec![i as u8]);
        model.insert(key.clone(), vec![i as u8]);
        check(&map, &model, &probes(&key));
        inserted.push(key);
    }
    assert!(map.root.as_ref().unwrap().prefix >= 24);
    let old = map.clone();
    let before = model.clone();
    for length in (0..24).rev() {
        let key = prefix[..length].to_vec();
        map.insert(key.clone(), vec![]);
        model.insert(key.clone(), vec![]);
        check(&map, &model, &probes(&key));
        check(&old, &before, &probes(&key));
        inserted.push(key);
    }
    for key in [vec![0], vec![255], vec![255, 0], vec![0, 255]] {
        map.insert(key.clone(), vec![255]);
        model.insert(key.clone(), vec![255]);
        check(&map, &model, &probes(&key));
        inserted.push(key);
    }
    for key in inserted.into_iter().rev() {
        assert_eq!(map.remove(&key), model.remove(&key).is_some());
        check(&map, &model, &probes(&key));
        check(&old, &before, &probes(&key));
    }
    let mut corrupt = old.clone();
    Arc::make_mut(corrupt.root.as_mut().unwrap()).prefix = 1000;
    assert!(std::panic::catch_unwind(|| corrupt.validate()).is_err());
}
