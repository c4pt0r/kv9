use super::*;
use std::collections::{BTreeMap, HashSet};
use std::sync::{Barrier, Weak};

type Model = BTreeMap<Vec<u8>, Vec<u8>>;

/// Independently reconstruct paths and count terminals; mutation helpers are
/// deliberately not used by this representation checker.
fn validate(map: &RadixMap) -> (usize, usize) {
    let mut pending = Vec::new();
    if let Some(root) = map.root.as_deref() {
        pending.push((root, Vec::new(), 1));
    }
    let mut addresses = HashSet::new();
    let mut entries = 0;
    let mut max_depth = 0;
    while let Some((node, mut path, depth)) = pending.pop() {
        assert!(
            addresses.insert(std::ptr::from_ref(node)),
            "duplicate node in one root"
        );
        max_depth = max_depth.max(depth);
        match node {
            Node::Leaf(entry) => {
                assert!(entry.key.starts_with(&path));
                entries += 1;
            }
            Node::Branch(branch) => {
                path.extend_from_slice(&branch.prefix);
                assert!(branch.edges.len() + usize::from(branch.terminal.is_some()) >= 2);
                assert!(branch.edges.len() <= 256);
                assert!(branch.edges.windows(2).all(|w| w[0].byte < w[1].byte));
                if let Some(entry) = &branch.terminal {
                    assert_eq!(entry.key, path);
                    entries += 1;
                }
                for edge in &branch.edges {
                    let mut child_path = path.clone();
                    child_path.push(edge.byte);
                    pending.push((&edge.node, child_path, depth + 1));
                }
            }
        }
    }
    assert_eq!(entries, map.len());
    assert_eq!(entries == 0, map.root.is_none());
    (addresses.len(), max_depth)
}

fn check(map: &RadixMap, model: &Model, queries: &[Vec<u8>]) {
    validate(map);
    assert_eq!(map.len(), model.len());
    assert_eq!(map.is_empty(), model.is_empty());
    assert!(map.iter().eq(model.iter()));
    assert!(map.iter().rev().eq(model.iter().rev()));
    for key in queries {
        assert_eq!(map.get(key), model.get(key));
        assert_eq!(map.get_key_value(key), model.get_key_value(key));
        assert_eq!(map.seek_le(key), model.range(..=key.clone()).next_back());
    }
    for pair in queries.windows(2) {
        let (a, b) = if pair[0] <= pair[1] {
            (&pair[0], &pair[1])
        } else {
            (&pair[1], &pair[0])
        };
        assert!(map
            .range(a.clone()..b.clone())
            .eq(model.range(a.clone()..b.clone())));
        assert!(map
            .range(a.clone()..=b.clone())
            .rev()
            .eq(model.range(a.clone()..=b.clone()).rev()));
        let bounds = (Bound::Excluded(a.clone()), Bound::Included(b.clone()));
        assert!(map.range(bounds.clone()).eq(model.range(bounds)));
    }
    let mut actual = map.iter();
    let mut expected = model.iter();
    let mut reverse = false;
    loop {
        let (a, b) = if reverse {
            (actual.next_back(), expected.next_back())
        } else {
            (actual.next(), expected.next())
        };
        assert_eq!(a, b);
        if b.is_none() {
            break;
        }
        reverse = !reverse;
    }
    assert_eq!(actual.next(), None);
    assert_eq!(actual.next_back(), None);
}

fn short_keys(alphabet: &[u8], depth: usize) -> Vec<Vec<u8>> {
    let mut keys = vec![Vec::new()];
    let mut level = vec![Vec::new()];
    for _ in 0..depth {
        let mut next = Vec::new();
        for prefix in level {
            for &byte in alphabet {
                let mut key = prefix.clone();
                key.push(byte);
                next.push(key);
            }
        }
        keys.extend(next.clone());
        level = next;
    }
    keys
}

#[test]
fn all_short_prefix_transitions_with_old_roots() {
    let keys = short_keys(&[0, 1, 255], 3);
    assert_eq!(keys.len(), 40);
    // Include uniquely owned paths as well as snapshot-shared paths. Without
    // the first three histories, discarded unique-leaf replacements can hide
    // behind the separate shared-leaf replacement implementation.
    for history in 0..6 {
        let order = history % 3;
        let mut ordered = keys.clone();
        if order == 1 {
            ordered.reverse();
        }
        if order == 2 {
            ordered.sort();
        }
        let mut map = RadixMap::new();
        let mut model = Model::new();
        let mut old = Vec::new();
        for step in 0..keys.len() * 3 {
            let key = &ordered[step % keys.len()];
            if step < keys.len() * 2 {
                let value = vec![step as u8, 0, 255];
                assert_eq!(
                    map.insert_mut(key.clone(), value.clone()),
                    model.insert(key.clone(), value).is_none()
                );
            } else {
                assert_eq!(map.remove_mut(key), model.remove(key).is_some());
            }
            check(&map, &model, &keys);
            if history >= 3 && step % 7 == 0 {
                old.push((map.clone(), model.clone()));
            }
            for (view, expected) in &old {
                check(view, expected, &keys);
            }
        }
        assert!(map.is_empty());
    }
    println!("short_prefix_histories=6 mutation_prefixes=720 keys=40 ownership_modes=2");
}

#[test]
fn every_byte_edge_grows_shrinks_and_keeps_terminals_distinct() {
    let mut map = RadixMap::new();
    let mut model = Model::new();
    let mut keys = vec![Vec::new()];
    for b in 0..=255 {
        keys.extend([vec![b], vec![b, 0], vec![b, 255], vec![b, 0, 128]]);
    }
    for (i, key) in keys.iter().enumerate() {
        let value = (i as u64).to_le_bytes().to_vec();
        assert!(map.insert_mut(key.clone(), value.clone()));
        model.insert(key.clone(), value);
        check(
            &map,
            &model,
            &[vec![], key.clone(), vec![127], vec![255, 255, 255]],
        );
    }
    let frozen = map.clone();
    let frozen_model = model.clone();
    for key in keys.iter().step_by(2).chain(keys.iter().skip(1).step_by(2)) {
        assert!(map.remove_mut(key));
        model.remove(key);
        check(
            &map,
            &model,
            &[vec![], key.clone(), vec![127], vec![255, 255, 255]],
        );
        assert_eq!(frozen.get(key), frozen_model.get(key));
    }
    check(&frozen, &frozen_model, &keys);
    assert!(map.root.is_none());
    println!("all_byte_edges=256 mutation_prefixes=2050 retained_complete_roots=1");
}

fn random(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

#[test]
fn generated_histories_match_btree_after_every_mutation() {
    for seed in [1, 0x9876_5432, 0xdead_beef, u64::MAX] {
        let mut state = seed;
        let mut keys = short_keys(&[0, 128, 255], 3);
        for i in 0..472 {
            let len = (random(&mut state) % 65) as usize;
            let mut key: Vec<_> = (0..len).map(|_| random(&mut state) as u8).collect();
            if i % 3 == 0 {
                key.splice(..0, b"shared\0prefix\xff".iter().copied());
            }
            keys.push(key);
        }
        let mut map = RadixMap::new();
        let mut model = Model::new();
        let mut retained = Vec::new();
        for step in 0..1_000 {
            let index = (random(&mut state) as usize) % keys.len();
            let key = &keys[index];
            if random(&mut state).is_multiple_of(4) {
                assert_eq!(map.remove_mut(key), model.remove(key).is_some());
            } else {
                let value: Vec<_> = (0..random(&mut state) % 80)
                    .map(|_| random(&mut state) as u8)
                    .collect();
                assert_eq!(
                    map.insert_mut(key.clone(), value.clone()),
                    model.insert(key.clone(), value).is_none()
                );
            }
            let queries: Vec<_> = (0..8)
                .map(|n| keys[(index + n * 67) % keys.len()].clone())
                .collect();
            check(&map, &model, &queries);
            if step % 47 == 0 {
                retained.push((map.clone(), model.clone()));
                if retained.len() > 6 {
                    retained.remove(0);
                }
            }
            for (view, expected) in &retained {
                check(view, expected, &queries);
            }
        }
    }
    println!("generated_seeds=4 mutation_prefixes=4000 retained_roots_at_once=6");
}

#[test]
fn all_bound_kinds_and_mixed_iteration_match_btree() {
    let keys = short_keys(&[0, 255], 3);
    let mut map = RadixMap::new();
    let mut model = Model::new();
    for key in &keys {
        map.insert_mut(key.clone(), key.clone());
        model.insert(key.clone(), key.clone());
    }
    let mut endpoints = keys;
    endpoints.extend([vec![1], vec![255, 1], vec![0, 0, 0, 0], vec![255; 4]]);
    endpoints.sort();
    endpoints.dedup();
    let mut bounds = vec![Bound::Unbounded];
    for key in &endpoints {
        bounds.extend([Bound::Included(key.clone()), Bound::Excluded(key.clone())]);
    }
    let mut checked = 0;
    for lower in &bounds {
        for upper in &bounds {
            if let (
                Bound::Included(a) | Bound::Excluded(a),
                Bound::Included(b) | Bound::Excluded(b),
            ) = (lower, upper)
            {
                if a > b
                    || (a == b
                        && matches!((lower, upper), (Bound::Excluded(_), Bound::Excluded(_))))
                {
                    continue;
                }
            }
            let limits = (lower.clone(), upper.clone());
            let expected: Vec<_> = model.range(limits.clone()).collect();
            assert_eq!(map.range(limits.clone()).collect::<Vec<_>>(), expected);
            assert!(map
                .range(limits.clone())
                .rev()
                .eq(expected.iter().rev().copied()));
            for pattern in [0u8, 1, 0b0101_0101, 0b1100_1010, 255] {
                let mut actual = map.range(limits.clone());
                let mut reference = model.range(limits.clone());
                for step in 0..model.len() + 3 {
                    if pattern & (1 << (step % 8)) == 0 {
                        assert_eq!(actual.next(), reference.next());
                    } else {
                        assert_eq!(actual.next_back(), reference.next_back());
                    }
                }
            }
            checked += 1;
        }
    }
    println!("valid_bound_pairs={checked} mixed_direction_patterns=5");
}

#[test]
fn range_owns_no_caller_bound_references_and_rejects_invalid_bounds() {
    let mut map = RadixMap::new();
    map.insert_mut(vec![1], vec![9]);
    let mut iter = {
        let low = vec![0];
        let high = vec![2];
        map.range(low..high)
    };
    assert_eq!(iter.next(), Some((&vec![1], &vec![9])));
    assert_eq!(iter.next_back(), None);
    assert!(std::panic::catch_unwind(|| map.range(vec![2]..vec![1])).is_err());
    assert!(std::panic::catch_unwind(
        || map.range((Bound::Excluded(vec![1]), Bound::Excluded(vec![1])))
    )
    .is_err());
    assert!(RadixMap::new().iter().next().is_none());
}

#[test]
fn snapshots_share_roots_and_unmodified_subtrees() {
    let mut map = RadixMap::new();
    for key in [b"aa", b"ab", b"za", b"zb"] {
        map.insert_mut(key.to_vec(), vec![1]);
    }
    let snapshot = map.clone();
    assert!(Arc::ptr_eq(
        map.root.as_ref().unwrap(),
        snapshot.root.as_ref().unwrap()
    ));
    assert_eq!(Arc::strong_count(map.root.as_ref().unwrap()), 2);
    assert!(!map.remove_mut(b"missing"));
    assert!(Arc::ptr_eq(
        map.root.as_ref().unwrap(),
        snapshot.root.as_ref().unwrap()
    ));
    assert!(!map.insert_mut(b"aa".to_vec(), vec![2]));
    let Node::Branch(live) = map.root.as_deref().unwrap() else {
        panic!()
    };
    let Node::Branch(old) = snapshot.root.as_deref().unwrap() else {
        panic!()
    };
    assert!(!Arc::ptr_eq(&live.edges[0].node, &old.edges[0].node));
    assert!(Arc::ptr_eq(&live.edges[1].node, &old.edges[1].node));
    assert_eq!(snapshot.get(b"aa"), Some(&vec![1]));
    assert_eq!(map.get(b"aa"), Some(&vec![2]));
    let mut fork = snapshot.clone();
    fork.remove_mut(b"za");
    fork.insert_mut(b"new".to_vec(), vec![]);
    assert_eq!(map.get(b"za"), Some(&vec![1]));
    assert_eq!(snapshot.get(b"new"), None);
}

#[test]
fn long_compressed_prefix_splits_and_collapses_at_each_position() {
    let original = vec![129; 16_384];
    for cut in [0, 1, 127, 8_191, 16_383, 16_384] {
        let mut map = RadixMap::new();
        let mut model = Model::new();
        let mut keys = vec![original.clone(), original[..cut].to_vec()];
        for byte in [0, 128, 130, 255] {
            let mut key = original[..cut].to_vec();
            key.push(byte);
            key.extend([0, 255]);
            keys.push(key);
        }
        for (i, key) in keys.iter().enumerate() {
            map.insert_mut(key.clone(), vec![i as u8]);
            model.insert(key.clone(), vec![i as u8]);
            check(&map, &model, &keys);
        }
        let frozen = map.clone();
        let frozen_model = model.clone();
        for key in &keys {
            assert_eq!(map.remove_mut(key), model.remove(key).is_some());
            check(&map, &model, &keys);
            check(&frozen, &frozen_model, &keys);
        }
    }
}

fn weak_nodes(map: &RadixMap) -> Vec<Weak<Node>> {
    let mut pending = Vec::new();
    let mut weak = Vec::new();
    if let Some(root) = &map.root {
        pending.push(root);
    }
    while let Some(node) = pending.pop() {
        weak.push(Arc::downgrade(node));
        if let Node::Branch(branch) = node.as_ref() {
            pending.extend(branch.edges.iter().map(|edge| &edge.node));
        }
    }
    weak
}

fn prefix_ladder(depth: usize) -> RadixMap {
    let mut map = RadixMap::new();
    for len in (0..=depth).rev() {
        map.insert_mut(vec![0; len], vec![len as u8]);
    }
    map
}

#[test]
fn deep_prefix_updates_iteration_and_reclamation_on_small_stack() {
    std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(|| {
            let depth = 4_096;
            let mut map = prefix_ladder(depth);
            assert_eq!(validate(&map).1, depth + 1);
            let snapshot = map.clone();
            let old_nodes = weak_nodes(&map);
            assert!(!map.insert_mut(vec![0; depth], vec![9]));
            assert_eq!(snapshot.get(&vec![0; depth]), Some(&vec![0]));
            assert!(map.remove_mut(&vec![0; depth - 1]));
            assert!(map.remove_mut(&vec![0; depth]));
            assert_eq!(map.iter().count(), depth - 1);
            assert_eq!(snapshot.iter().rev().count(), depth + 1);
            assert_eq!(map.seek_le(&vec![0; depth]).unwrap().0.len(), depth - 2);
            assert_eq!(map.range(vec![0; depth - 3]..).count(), 2);
            assert_eq!(
                format!("{map:?}"),
                format!("RadixMap {{ len: {} }}", depth - 1)
            );
            drop(snapshot);
            assert!(old_nodes.iter().all(|node| node.upgrade().is_none()));
            let live_nodes = weak_nodes(&map);
            for len in 0..depth - 1 {
                assert!(map.remove_mut(&vec![0; len]));
            }
            assert!(map.is_empty());
            assert!(live_nodes.iter().all(|node| node.upgrade().is_none()));
            drop(map);
            println!("deep_prefix_depth=4096 thread_stack_bytes=65536 weak_nodes_reclaimed=true");
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn concurrent_shared_root_and_subtree_reclamation_on_small_stacks() {
    let map = prefix_ladder(2_048);
    let mut weak = weak_nodes(&map);
    let barrier = Arc::new(Barrier::new(8));
    let mut threads = Vec::new();
    for i in 0..8 {
        let mut view = map.clone();
        // Half share the exact root; others retain shared deeper subtrees.
        if i % 2 == 0 {
            view.insert_mut(vec![i + 1], vec![i]);
        }
        weak.extend(weak_nodes(&view));
        let barrier = barrier.clone();
        threads.push(
            std::thread::Builder::new()
                .stack_size(64 * 1024)
                .spawn(move || {
                    barrier.wait();
                    drop(view);
                })
                .unwrap(),
        );
    }
    drop(map);
    for thread in threads {
        thread.join().unwrap();
    }
    assert!(weak.iter().all(|node| node.upgrade().is_none()));
    println!("concurrent_drop_threads=8 prefix_depth=2048 thread_stack_bytes=65536");
}

#[test]
fn one_megabyte_binary_keys_have_no_public_length_cap() {
    let mut key = vec![255; 1 << 20];
    key[0] = 0;
    key[1 << 19] = 0;
    let mut map = RadixMap::new();
    assert!(map.insert_mut(key.clone(), vec![0, 255]));
    let snapshot = map.clone();
    let mut extension = key.clone();
    extension.push(0);
    assert!(map.insert_mut(extension.clone(), vec![]));
    assert_eq!(map.get(&key), Some(&vec![0, 255]));
    assert_eq!(map.seek_le(&extension).unwrap().0, &extension);
    assert_eq!(map.iter().count(), 2);
    assert!(map.remove_mut(&key));
    assert_eq!(map.get(&extension), Some(&vec![]));
    assert_eq!(snapshot.get(&key), Some(&vec![0, 255]));
    assert!(map.remove_mut(&extension));
    assert!(map.is_empty());
}

#[test]
fn layout_is_reported_without_claiming_allocator_measurements() {
    println!(
        "layout_bytes map={} node={} branch={} edge={} entry={} arc={}",
        std::mem::size_of::<RadixMap>(),
        std::mem::size_of::<Node>(),
        std::mem::size_of::<Branch>(),
        std::mem::size_of::<Edge>(),
        std::mem::size_of::<Entry>(),
        std::mem::size_of::<Arc<Node>>()
    );
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<RadixMap>();
}
