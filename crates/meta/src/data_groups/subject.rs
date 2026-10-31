//! What each data-group task row is about, for the scoped metadata mode
//! (docs/META-REPLICA-MODES.md §3b): the regions, keyspaces, tenants and nodes a row
//! names, plus the earlier tasks whose subject it shares. This is routing, not
//! validation: readers still validate every row they use. A kind not declared here
//! has no subject, and the scope stream stops on it rather than guess.

use super::*;

/// The subject of one task row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskSubject {
    pub kind: u64,
    pub regions: Vec<RegionId>,
    pub keyspaces: Vec<u64>,
    pub tenants: Vec<u64>,
    /// Nodes the row names as a replica, destination or source.
    pub nodes: Vec<NodeId>,
    /// Earlier tasks whose subject this row shares (a creation, an intent, a target).
    pub refs: Vec<u64>,
    /// A backup completion or retirement: shares the subject of the backup intent
    /// with this operation.
    pub backup_operation: Option<[u8; 16]>,
    /// A node drain: about that node alone.
    pub drain: Option<NodeId>,
    /// Lineage: the creation tasks of the regions this row derives from `parents`
    /// (a split's children, a merge's child, a restore target), and the parents.
    pub child_creations: Vec<u64>,
    pub parents: Vec<RegionId>,
    /// Lineage given directly as `(child, parent)` regions (a restore's targets).
    pub edges: Vec<(RegionId, RegionId)>,
    /// A row about a whole keyspace's ranges (backup and restore intents, backup
    /// completions and retirements): delivered where it touches a scope, never a
    /// reason to widen one.
    pub umbrella: bool,
}

/// Every task kind this module declares (the totality test checks production
/// writers against it).
pub const DECLARED_KINDS: &[u64] = &[
    100, 101, 102, 103, 104, 105, 106, 108, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119,
    120, 121, 122, 123, 124,
];

fn real(node: NodeId) -> Option<NodeId> {
    // The synthetic backup destination is not a node.
    (node != kv9_common::BACKUP_DESTINATION_NODE).then_some(node)
}

/// The subject of a TASKS row value. `Ok(None)`: the kind is not declared.
pub fn task_subject(value: &RowValue) -> Result<Option<TaskSubject>> {
    let Some(ColumnValue::Uint(kind)) = value.get(ColumnId(2)) else {
        return Err(invalid("task row without a kind"));
    };
    let kind = *kind;
    if !DECLARED_KINDS.contains(&kind) {
        return Ok(None);
    }
    let Some(ColumnValue::Bytes(bytes)) = value.get(ColumnId(3)) else {
        return Err(invalid("task row without a payload"));
    };
    let mut s = TaskSubject {
        kind,
        ..TaskSubject::default()
    };
    match kind {
        CREATE_EMPTY_GROUP | 101 => {
            let intent = CreationIntent::decode(bytes)?;
            s.regions.push(intent.region());
            s.nodes.extend(intent.replicas().iter().map(|r| r.node));
            if kind == 101 {
                s.refs.push(intent.task());
            }
        }
        102 => {
            if bytes.len() < 8 {
                return Err(invalid("range binding payload too short"));
            }
            s.refs.push(u64::from_be_bytes(bytes[..8].try_into().expect("8 bytes")));
            let range = kv9_common::data_range::DataRange::decode(&bytes[8..])?;
            s.regions.push(range.region);
            s.keyspaces.push(range.keyspace.0 as u64);
            s.tenants.push(range.tenant.0);
        }
        103 => {
            let m = migration::MigrationIntent::decode(bytes)?;
            s.regions.push(m.region());
            s.nodes.push(m.destination().node);
            s.refs.push(m.creation_task());
        }
        104 => {
            let e = evidence::InstallEvidence::decode(bytes)?;
            s.regions.push(e.region());
            s.nodes.extend(real(e.destination().node));
            s.refs.push(e.migration_task());
        }
        105 => {
            let t = truncation::TruncationDecision::decode(bytes)?;
            s.regions.push(t.region());
            s.refs.push(t.evidence_task());
        }
        106 => {
            let r = removal::RemovalDecision::decode(bytes)?;
            s.regions.push(r.region());
            s.nodes.push(r.source().node);
            s.refs.extend([r.evidence_task(), r.origin_task()].into_iter().filter(|t| *t != 0));
        }
        108 => {
            let a = abort::MigrationAbort::decode(bytes)?;
            s.regions.push(a.region());
            s.nodes.extend(real(a.destination().node));
            s.refs.push(a.migration_task());
        }
        110 => {
            let c = compaction::GroupCompaction::decode(bytes)?;
            s.regions.push(c.region());
            s.refs.push(c.creation_task());
        }
        111 => {
            let p = addition::ReplicaPlan::decode(bytes)?;
            s.regions.push(p.region());
            s.nodes.extend(p.source().map(|r| r.node));
            s.refs.extend(Some(p.migration_task()).filter(|t| *t != 0));
        }
        112 => {
            let d = drain::DrainIntent::decode(bytes)?;
            s.drain = Some(d.node());
        }
        113 => {
            let i = split::SplitIntent::decode(bytes)?;
            s.regions.push(i.parent_region());
            s.refs.extend([i.parent_bind_task(), i.child_low(), i.child_high()]);
            s.child_creations.extend([i.child_low(), i.child_high()]);
            s.parents.push(i.parent_region());
        }
        114 => {
            let e = split_evidence::SplitEvidence::decode(bytes)?;
            s.refs.extend([e.intent_task(), e.child_task()]);
        }
        115 => {
            let m = merge::MergeIntent::decode(bytes)?;
            s.regions.extend([m.low_region(), m.high_region()]);
            s.refs.extend([m.low_bind_task(), m.high_bind_task(), m.child()]);
            s.child_creations.push(m.child());
            s.parents.extend([m.low_region(), m.high_region()]);
        }
        116 => {
            let e = merge::MergeEvidence::decode(bytes)?;
            s.refs.extend([e.intent_task(), e.child_task()]);
        }
        117 => {
            let a = range_abort::RangeAbort::decode(bytes)?;
            s.refs.push(a.target_task());
        }
        118 => {
            let e = range_abort::SealEvidence::decode(bytes)?;
            s.regions.push(e.region());
        }
        119 => {
            let b = backup::BackupIntent::decode(bytes)?;
            s.umbrella = true;
            s.keyspaces.push(b.keyspace().0 as u64);
            s.tenants.push(b.tenant().0);
            for range in b.ranges() {
                s.regions.push(range.region());
                s.refs.extend([range.creation_task(), range.bind_task()]);
            }
        }
        120 => {
            let a = backup::BackupAnchor::decode(bytes)?;
            s.regions.push(a.region());
        }
        121 => {
            let r = backup::BackupRetirement::decode(bytes)?;
            s.backup_operation = Some(r.operation());
            s.umbrella = true;
        }
        122 => {
            let c = backup::BackupCompletion::decode(bytes)?;
            s.backup_operation = Some(c.operation());
            s.umbrella = true;
        }
        123 => {
            let r = restore::RestoreIntent::decode(bytes)?;
            s.umbrella = true;
            s.edges.extend(r.targets().iter().map(|t| (t.region(), t.source_region())));
            s.keyspaces.push(r.keyspace().0 as u64);
            s.tenants.push(r.tenant().0);
            s.refs.push(r.backup_task());
            for target in r.targets() {
                s.regions.extend([target.region(), target.source_region()]);
                s.refs.push(target.creation_task());
            }
        }
        124 => {
            let e = restore::RestoreEvidence::decode(bytes)?;
            s.regions.push(e.source_region());
            s.refs.push(e.target_task());
            s.child_creations.push(e.target_task());
            s.parents.push(e.source_region());
        }
        _ => unreachable!("declared kinds are matched above"),
    }
    s.refs.retain(|t| *t != 0);
    Ok(Some(s))
}

/// The operation of a backup intent row, to resolve completions and retirements.
pub fn backup_intent_operation(value: &RowValue) -> Result<Option<[u8; 16]>> {
    if value.get(ColumnId(2)) != Some(&ColumnValue::Uint(119)) {
        return Ok(None);
    }
    let Some(ColumnValue::Bytes(bytes)) = value.get(ColumnId(3)) else {
        return Err(invalid("task row without a payload"));
    };
    Ok(Some(backup::BackupIntent::decode(bytes)?.operation()))
}

#[cfg(test)]
mod tests {
    use super::super::tests::fixture;
    use super::*;
    use crate::scope::{classify, closure, route, snapshot_rows, Resolver, Route, ALL_END, ALL_START};
    use std::collections::BTreeSet;
    use crate::schema::{NODES_DESC, TENANTS_DESC};
    use kv9_common::TenantId;
    use kv9_engine::{Engine, MemEngine};

    fn add_node(txn: &mut MetaTxn<'_, MemEngine>, id: u64) {
        let mut row = RowValue::new();
        row.set(ColumnId(2), ColumnValue::Text(format!("127.0.0.1:{}", 18000 + id)));
        row.set(ColumnId(3), ColumnValue::Uint(2));
        row.set(ColumnId(4), ColumnValue::Uint(0));
        row.set(ColumnId(5), ColumnValue::Bytes(vec![id as u8; 16]));
        txn.insert(&NODES_DESC, &[memcmp_uint(id)], row).unwrap();
    }
    fn add_tenant(txn: &mut MetaTxn<'_, MemEngine>, id: u64) {
        let mut row = RowValue::new();
        row.set(ColumnId(2), ColumnValue::Text(format!("tenant-{id}")));
        row.set(ColumnId(3), ColumnValue::Uint(0));
        row.set(ColumnId(4), ColumnValue::Uint(0));
        txn.insert(&TENANTS_DESC, &[memcmp_uint(id)], row).unwrap();
    }

    /// Two data keyspaces on disjoint replica sets: `a` on nodes 1-3, `b` on 4-6.
    fn two_keyspaces() -> (MetaStore<MemEngine>, (CreationIntent, u64), (CreationIntent, u64)) {
        let store = fixture();
        let mut txn = store.begin().unwrap();
        add_tenant(&mut txn, 1);
        add_tenant(&mut txn, 2);
        for id in 4..=6 {
            add_node(&mut txn, id);
        }
        let a = plan_empty_group(&mut txn, [41; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        let b = plan_empty_group(&mut txn, [42; 16], &[NodeId(4), NodeId(5), NodeId(6)]).unwrap();
        txn.commit().unwrap();
        let mut txn = store.begin().unwrap();
        let (ra, _) = ranges::plan_keyspace(&mut txn, a.root(), a.task(), "ka", TenantId(1)).unwrap();
        let (rb, _) = ranges::plan_keyspace(&mut txn, b.root(), b.task(), "kb", TenantId(2)).unwrap();
        txn.commit().unwrap();
        (store, (a, ra.keyspace.0 as u64), (b, rb.keyspace.0 as u64))
    }

    #[test]
    fn a_scope_is_its_keyspaces_their_tenants_and_the_nodes_it_names() {
        let (store, (a, ka), (b, kb)) = two_keyspaces();
        let view = store.engine().snapshot().unwrap();
        let c4 = closure(view.as_ref(), 4, &[1, 2, 3]).unwrap();
        assert_eq!(c4.regions.iter().copied().collect::<Vec<_>>(), vec![b.region().0]);
        assert_eq!(c4.keyspaces.iter().copied().collect::<Vec<_>>(), vec![kb]);
        assert_eq!(c4.tenants.iter().copied().collect::<Vec<_>>(), vec![2]);
        assert_eq!(c4.nodes.iter().copied().collect::<Vec<_>>(), vec![1, 2, 3, 4, 5, 6]);
        let c1 = closure(view.as_ref(), 1, &[1, 2, 3]).unwrap();
        assert_eq!(c1.regions.iter().copied().collect::<Vec<_>>(), vec![a.region().0]);
        assert_eq!(c1.keyspaces.iter().copied().collect::<Vec<_>>(), vec![ka]);
        // A node that hosts nothing holds the globals and the full nodes only.
        let c9 = closure(view.as_ref(), 9, &[1, 2, 3]).unwrap();
        assert!(c9.regions.is_empty() && c9.keyspaces.is_empty());
    }

    #[test]
    fn a_snapshot_holds_its_scope_and_nothing_of_another_keyspace() {
        let (store, (a, ka), (b, kb)) = two_keyspaces();
        let view = store.engine().snapshot().unwrap();
        let c4 = closure(view.as_ref(), 4, &[1, 2, 3]).unwrap();
        let rows = snapshot_rows(view.as_ref(), &c4).unwrap();
        let mut resolver = Resolver::new(view.as_ref());
        let mut tasks = 0;
        for (key, value) in &rows {
            if let Some(crate::scope::RowClass::Task(s)) = classify(key, Some(value)).unwrap() {
                let r = resolver.resolve(&s).unwrap();
                assert!(!r.regions.contains(&a.region().0), "task of the other keyspace delivered");
                assert!(r.regions.contains(&b.region().0) || r.keyspaces.contains(&kb));
                tasks += 1;
            }
            if let Some(crate::scope::RowClass::Keyspace(k)) = classify(key, Some(value)).unwrap() {
                assert_eq!(k, kb);
                assert_ne!(k, ka);
            }
        }
        assert!(tasks >= 3, "creation, activation and binding are in scope");
        // The root certificate and cluster identity are in every scope.
        let root = crate::codec::encode_row_key(crate::schema::ROOT_META, &[memcmp_uint(0)]).unwrap();
        assert!(rows.iter().any(|(k, _)| k == &root));
    }

    #[test]
    fn every_key_a_data_keyspace_and_a_drain_write_is_declared() {
        let (store, _, _) = two_keyspaces();
        let mut txn = store.begin().unwrap();
        drain::plan_drain(&mut txn, NodeId(5)).unwrap();
        txn.commit().unwrap();
        let view = store.engine().snapshot().unwrap();
        crate::scope::for_each(view.as_ref(), ALL_START, ALL_END, |key, value| {
            assert!(classify(key, Some(value)).unwrap().is_some(), "undeclared key {key:?}");
            assert!(classify(key, None).unwrap().is_some(), "undeclared delete {key:?}");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn an_unrelated_change_routes_nowhere_and_a_change_naming_the_node_widens_it() {
        let (store, (a, _), _) = two_keyspaces();
        let before = store.engine().snapshot().unwrap();
        let c4 = closure(before.as_ref(), 4, &[1, 2, 3]).unwrap();
        drop(before);
        // A group on nodes 1-3 is not node 4's business; one naming node 4 is.
        let mut txn = store.begin().unwrap();
        let other = plan_empty_group(&mut txn, [43; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        let mine = plan_empty_group(&mut txn, [44; 16], &[NodeId(4), NodeId(5), NodeId(6)]).unwrap();
        let (drain, _) = drain::plan_drain(&mut txn, NodeId(4)).unwrap();
        txn.commit().unwrap();
        let view = store.engine().snapshot().unwrap();
        let mut resolver = Resolver::new(view.as_ref());
        let at = |task: u64| {
            let key = crate::codec::encode_row_key(crate::schema::TASKS, &[memcmp_uint(task)]).unwrap();
            let value = view.get(kv9_engine::ColumnFamily::Default, &key).unwrap().unwrap();
            (key, value)
        };
        let (k, v) = at(other.task());
        assert_eq!(route(&c4, &mut resolver, &k, Some(&v)).unwrap(), Route::Skip);
        let (k, v) = at(mine.task());
        assert_eq!(route(&c4, &mut resolver, &k, Some(&v)).unwrap(), Route::Resnapshot);
        let (k, v) = at(drain.task());
        assert_eq!(route(&c4, &mut resolver, &k, Some(&v)).unwrap(), Route::Deliver);
        let c5 = closure(view.as_ref(), 5, &[1, 2, 3]).unwrap();
        assert_eq!(route(&c5, &mut resolver, &k, Some(&v)).unwrap(), Route::Skip);
        // A delete of a task row is delivered only where it is held.
        assert_eq!(route(&c4, &mut resolver, &k, None).unwrap(), Route::IfHeld);
        // An undeclared raw key stops the stream.
        assert_eq!(route(&c4, &mut resolver, b"\0kv9\0something-new\0", Some(b"x")).unwrap(), Route::Undeclared);
        let _ = a;
    }

    /// Split `region` at `key` under `operation` and publish it (both evidence rows).
    fn split_published(
        store: &MetaStore<MemEngine>,
        operation: [u8; 16],
        region: RegionId,
        key: &[u8],
    ) -> (CreationIntent, CreationIntent) {
        let mut txn = store.begin().unwrap();
        let (intent, low, high, _) = split::plan_split(&mut txn, operation, region, key).unwrap();
        txn.commit().unwrap();
        for child in [low.task(), high.task()] {
            let receipt = split_evidence::SplitEvidence::receipt(
                &intent,
                child,
                RootDigest::sha256(&child.to_be_bytes()),
                1,
            )
            .unwrap();
            let mut txn = store.begin().unwrap();
            split_evidence::plan_split_evidence(&mut txn, &receipt).unwrap();
            txn.commit().unwrap();
        }
        let mut txn = store.begin().unwrap();
        split::plan_split_publication(&mut txn, operation).unwrap();
        txn.commit().unwrap();
        (low, high)
    }

    #[test]
    fn a_region_scope_holds_its_ancestors_and_their_siblings_not_the_keyspace() {
        // One keyspace on nodes 1-3: P splits into L and H, L splits into LL and LH.
        let store = fixture();
        let mut txn = store.begin().unwrap();
        add_tenant(&mut txn, 1);
        add_node(&mut txn, 9);
        let p = plan_empty_group(&mut txn, [51; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        txn.commit().unwrap();
        let mut txn = store.begin().unwrap();
        ranges::plan_keyspace(&mut txn, p.root(), p.task(), "tree", TenantId(1)).unwrap();
        txn.commit().unwrap();
        let (l, h) = split_published(&store, [52; 16], p.region(), b"m");
        let (ll, lh) = split_published(&store, [53; 16], l.region(), b"f");
        // Node 9 hosts H alone (a region-peer row, as a moved replica would show).
        let mut txn = store.begin().unwrap();
        let mut peer = RowValue::new();
        peer.set(ColumnId(1), ColumnValue::Uint(h.region().0));
        peer.set(ColumnId(2), ColumnValue::Uint(9));
        peer.set(ColumnId(3), ColumnValue::Uint(1));
        txn.insert(
            &crate::schema::REGION_PEERS_DESC,
            &[memcmp_uint(h.region().0), memcmp_uint(9)],
            peer,
        )
        .unwrap();
        txn.commit().unwrap();

        let view = store.engine().snapshot().unwrap();
        let c9 = closure(view.as_ref(), 9, &[1, 2, 3]).unwrap();
        let ids = |v: &[&CreationIntent]| v.iter().map(|c| c.region().0).collect::<BTreeSet<_>>();
        assert_eq!(c9.anchors, ids(&[&h, &p]), "H and its parent");
        assert_eq!(c9.regions, ids(&[&h, &p, &l]), "plus the parent's other child, one hop");
        // Nothing of the sibling's own subtree reaches node 9.
        let rows = snapshot_rows(view.as_ref(), &c9).unwrap();
        let mut resolver = Resolver::new(view.as_ref());
        for (key, value) in &rows {
            if let Some(crate::scope::RowClass::Task(s)) = classify(key, Some(value)).unwrap() {
                let r = resolver.resolve(&s).unwrap();
                assert!(
                    !r.regions.contains(&ll.region().0) && !r.regions.contains(&lh.region().0),
                    "a row of L's subtree reached node 9: kind {}",
                    s.kind
                );
            }
        }
        // A node on every region holds the whole tree.
        let c1 = closure(view.as_ref(), 1, &[1, 2, 3]).unwrap();
        assert_eq!(c1.regions, ids(&[&p, &l, &h, &ll, &lh]));
        // The snapshot is a fixpoint: none of its rows asks for another snapshot, and
        // L's split (outside node 9's regions) is skipped, not a reason to widen.
        let mut resolver = Resolver::new(view.as_ref());
        for (key, value) in &rows {
            assert_ne!(route(&c9, &mut resolver, key, Some(value)).unwrap(), Route::Resnapshot);
        }
        crate::scope::for_each(view.as_ref(), crate::scope::ALL_START, crate::scope::ALL_END, |key, value| {
            let route = route(&c9, &mut resolver, key, Some(value)).unwrap();
            assert_ne!(route, Route::Resnapshot, "a committed row widens a fixpoint closure");
            Ok(())
        })
        .unwrap();
    }
}
