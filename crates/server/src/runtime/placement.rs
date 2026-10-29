//! Where a replica goes (docs/DRAIN-NODE.md §4, shared by REPLICA-REPAIR and
//! DRAIN-NODE): the active registered store that is not excluded (a member of
//! the group, the lost or drained node), **not draining**, with the fewest
//! committed data-group memberships, lowest node id among equals. Catalog only;
//! no load or locality signal (D06 remains open).

use std::collections::{BTreeMap, BTreeSet};

use kv9_common::{NodeId, Result, RootDigest};
use kv9_engine::Engine;
use kv9_meta::data_groups::drain::{committed_drains_in_txn, is_draining};
use kv9_meta::data_groups::{committed_creations, committed_members, InitialReplica};
use kv9_meta::schema::NODES_DESC;
use kv9_meta::{ColumnValue, MetaStore, MetaTxn};

/// The least-loaded placeable store outside `exclude`, or `None` when no active,
/// non-draining registered store qualifies.
pub(super) fn choose_destination<E: Engine>(
    txn: &MetaTxn<'_, E>,
    store: &MetaStore<E>,
    root: RootDigest,
    exclude: &BTreeSet<u64>,
) -> Result<Option<NodeId>> {
    let mut hosted: BTreeMap<u64, usize> = BTreeMap::new();
    for creation in committed_creations(store)? {
        for (m, _) in committed_members(txn, root, creation.intent())? {
            *hosted.entry(m.node.0).or_insert(0) += 1;
        }
    }
    let drains = committed_drains_in_txn(txn)?;
    let mut best: Option<(usize, u64)> = None;
    for row in txn.scan(&NODES_DESC, usize::MAX)? {
        let Some(ColumnValue::Uint(id)) = row.value.get(kv9_meta::schema::ColumnId(1)).cloned() else {
            continue;
        };
        if exclude.contains(&id) {
            continue;
        }
        let Some(endpoint) = kv9_meta::endpoint::node_endpoint(txn, NodeId(id)).ok().flatten() else {
            continue;
        };
        if !endpoint.active
            || is_draining(
                &drains,
                InitialReplica {
                    node: NodeId(id),
                    incarnation: endpoint.incarnation,
                },
            )
        {
            continue;
        }
        let load = hosted.get(&id).copied().unwrap_or(0);
        if best.is_none_or(|(l, n)| (load, id) < (l, n)) {
            best = Some((load, id));
        }
    }
    Ok(best.map(|(_, id)| NodeId(id)))
}
