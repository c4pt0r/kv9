//! Committed node drain (docs/DRAIN-NODE.md): one immutable row says that store
//! `incarnation` of `node` leaves the cluster. The row moves no data and changes
//! no membership by itself; it is the intent the drained node (leadership
//! hand-off) and the metadata leader (move plans, metadata membership) act on,
//! and the fact placement consults to send nothing new to that store.
//!
//! One row per store incarnation: the operation is derived from
//! `(root, node, incarnation)`, so a resubmission confirms and a later store on
//! the same node id is a different, placeable store.

use super::*;

const NODE_DRAIN: u64 = 112;
const DRN_MAGIC: &[u8; 8] = b"KV9DRN02";
pub const MAX_DRAIN_BYTES: usize = 8 + 32 + 16 + 8 + 8 + 16 + 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrainIntent {
    root: RootDigest,
    operation: [u8; 16],
    task: u64,
    store: InitialReplica,
    /// How many drains of this store came before (aborted ones): part of the identity.
    sequence: u64,
}

impl DrainIntent {
    pub fn root(&self) -> RootDigest {
        self.root
    }
    pub fn operation(&self) -> [u8; 16] {
        self.operation
    }
    pub fn task(&self) -> u64 {
        self.task
    }
    /// The exact store that leaves: node id and store incarnation.
    pub fn store(&self) -> InitialReplica {
        self.store
    }
    pub fn node(&self) -> NodeId {
        self.store.node
    }
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(MAX_DRAIN_BYTES);
        bytes.extend_from_slice(DRN_MAGIC);
        bytes.extend_from_slice(self.root.as_bytes());
        bytes.extend_from_slice(&self.operation);
        bytes.extend_from_slice(&self.task.to_be_bytes());
        bytes.extend_from_slice(&self.store.node.0.to_be_bytes());
        bytes.extend_from_slice(self.store.incarnation.as_bytes());
        bytes.extend_from_slice(&self.sequence.to_be_bytes());
        bytes
    }

    /// Decode data only. This does not mint committed authority.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != MAX_DRAIN_BYTES || &bytes[..8] != DRN_MAGIC {
            return Err(invalid("invalid drain encoding"));
        }
        let drain = Self {
            root: RootDigest::from_bytes(bytes[8..40].try_into().unwrap()),
            operation: bytes[40..56].try_into().unwrap(),
            task: u64::from_be_bytes(bytes[56..64].try_into().unwrap()),
            store: InitialReplica {
                node: NodeId(u64::from_be_bytes(bytes[64..72].try_into().unwrap())),
                incarnation: StoreIncarnation::from_bytes(bytes[72..88].try_into().unwrap()),
            },
            sequence: u64::from_be_bytes(bytes[88..96].try_into().unwrap()),
        };
        if drain.root.as_bytes() == &[0; 32]
            || drain.task < FIRST_DYNAMIC_ID
            || drain.store.node.0 == 0
            || drain.store.incarnation.as_bytes() == &[0; 16]
            || drain.operation != drain_operation(drain.root, drain.store, drain.sequence)
        {
            return Err(invalid("invalid drain identities"));
        }
        Ok(drain)
    }
}

/// `sha16("kv9-drain/v2" ++ root ++ node ++ incarnation ++ sequence)`: one identity per
/// drain of a store; an aborted drain leaves the next sequence free.
pub fn drain_operation(root: RootDigest, store: InitialReplica, sequence: u64) -> [u8; 16] {
    let mut bytes = b"kv9-drain/v2".to_vec();
    bytes.extend_from_slice(root.as_bytes());
    bytes.extend_from_slice(&store.node.0.to_be_bytes());
    bytes.extend_from_slice(store.incarnation.as_bytes());
    bytes.extend_from_slice(&sequence.to_be_bytes());
    let digest = RootDigest::sha256(&bytes);
    let mut out = [0u8; 16];
    out.copy_from_slice(&digest.as_bytes()[..16]);
    out
}

pub(crate) fn from_drain_row(row: &Row, root: RootDigest) -> Result<Option<DrainIntent>> {
    if row.value.get(ColumnId(2)) != Some(&ColumnValue::Uint(NODE_DRAIN)) {
        return Ok(None);
    }
    let Some(ColumnValue::Bytes(bytes)) = row.value.get(ColumnId(3)) else {
        return Err(invalid("drain payload is missing"));
    };
    let drain = DrainIntent::decode(bytes)?;
    if drain.root != root
        || row.pk != [memcmp_uint(drain.task)]
        || row.value.get(ColumnId(1)) != Some(&ColumnValue::Uint(drain.task))
        || row.value.get(ColumnId(4)) != Some(&ColumnValue::Uint(0))
        || row.value.get(ColumnId(5)) != Some(&ColumnValue::Uint(0))
    {
        return Err(invalid("drain row binding differs"));
    }
    Ok(Some(drain))
}

/// Stage one idempotent drain row for the CURRENT store of `node` in the
/// serialized, term-fenced catalog transaction. `false` means the exact row
/// already exists and this commit is a confirmation receipt. The node must be a
/// registered store; its incarnation comes from the catalog, never the caller.
pub fn plan_drain<E: Engine>(txn: &mut MetaTxn<'_, E>, node: NodeId) -> Result<(DrainIntent, bool)> {
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("drain requires a certified metadata root"))?;
    let endpoint = crate::endpoint::node_endpoint(txn, node)?
        .ok_or_else(|| invalid("drain names a node that is not a registered store"))?;
    let store = InitialReplica {
        node,
        incarnation: endpoint.incarnation,
    };
    // A live (unaborted) drain of this store confirms; aborted ones only count.
    let aborted = super::range_abort::effective_aborts_in_txn(txn, root.digest())?;
    let mut sequence = 0u64;
    for row in task_rows(txn) {
        if let Some(previous) = from_drain_row(&row?, root.digest())? {
            if previous.store != store {
                continue;
            }
            if !aborted.contains(&previous.operation) {
                return Ok((previous, false));
            }
            sequence = sequence.max(previous.sequence + 1);
        }
    }
    let drain = DrainIntent {
        root: root.digest(),
        operation: drain_operation(root.digest(), store, sequence),
        task: txn.allocate_id(SequenceKind::Task)?,
        store,
        sequence,
    };
    DrainIntent::decode(&drain.encode())?;
    let mut row = RowValue::new();
    for (column, value) in [
        (ColumnId(1), ColumnValue::Uint(drain.task)),
        (ColumnId(2), ColumnValue::Uint(NODE_DRAIN)),
        (ColumnId(3), ColumnValue::Bytes(drain.encode())),
        (ColumnId(4), ColumnValue::Uint(0)),
        (ColumnId(5), ColumnValue::Uint(0)),
    ] {
        row.set(column, value);
    }
    txn.insert(&TASKS_DESC, &[memcmp_uint(drain.task)], row)?;
    Ok((drain, true))
}

/// Every committed, UNABORTED drain, from one applied snapshot. Drains are
/// immutable; a drain is *finished* when its store is in no data group and its
/// node is no member of the metadata group — derived by the caller, never
/// recorded. An aborted drain is history and is left out here.
pub fn committed_drains<E: Engine>(store: &MetaStore<E>) -> Result<Vec<DrainIntent>> {
    let txn = store.begin()?;
    committed_drains_in_txn(&txn)
}

pub fn committed_drains_in_txn<E: Engine>(txn: &MetaTxn<'_, E>) -> Result<Vec<DrainIntent>> {
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("drain readback requires a certified metadata root"))?;
    let aborted = super::range_abort::effective_aborts_in_txn(txn, root.digest())?;
    let mut out: Vec<DrainIntent> = Vec::new();
    for row in task_rows(txn) {
        if let Some(drain) = from_drain_row(&row?, root.digest())? {
            if aborted.contains(&drain.operation) {
                continue;
            }
            if out
                .iter()
                .any(|prior| prior.task == drain.task || prior.operation == drain.operation)
            {
                return Err(invalid("duplicate drain identity"));
            }
            out.push(drain);
        }
    }
    Ok(out)
}

/// Is `store` (node and exact incarnation) named by a committed drain?
pub fn is_draining(drains: &[DrainIntent], store: InitialReplica) -> bool {
    drains.iter().any(|d| d.store == store)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::NODES_DESC;

    fn register_node(store: &MetaStore<kv9_engine::MemEngine>, node: u64, incarnation: [u8; 16]) {
        let mut txn = store.begin().unwrap();
        let mut row = RowValue::new();
        row.set(
            ColumnId(2),
            ColumnValue::Text(format!("127.0.0.1:{}", 18000 + node)),
        );
        row.set(ColumnId(3), ColumnValue::Uint(2));
        row.set(ColumnId(4), ColumnValue::Uint(0));
        row.set(ColumnId(5), ColumnValue::Bytes(incarnation.to_vec()));
        txn.insert(&NODES_DESC, &[memcmp_uint(node)], row).unwrap();
        txn.commit().unwrap();
    }

    #[test]
    fn drain_is_one_row_per_store_incarnation() {
        let store = super::super::tests::fixture();
        register_node(&store, 7, [7; 16]);
        let mut txn = store.begin().unwrap();
        let (first, changed) = plan_drain(&mut txn, NodeId(7)).unwrap();
        assert!(changed);
        txn.commit().unwrap();
        let mut txn = store.begin().unwrap();
        let (again, changed) = plan_drain(&mut txn, NodeId(7)).unwrap();
        assert!(!changed);
        assert_eq!(first, again);
        assert_eq!(first.store().incarnation.as_bytes(), &[7; 16]);
        assert!(plan_drain(&mut txn, NodeId(8)).is_err(), "unregistered node");
        drop(txn);
        let drains = committed_drains(&store).unwrap();
        assert_eq!(drains, vec![first.clone()]);
        assert!(is_draining(&drains, first.store()));
        assert!(!is_draining(
            &drains,
            InitialReplica {
                node: NodeId(7),
                incarnation: StoreIncarnation::from_bytes([8; 16]),
            }
        ));
        // A tampered operation is refused by decode.
        let mut bytes = first.encode();
        bytes[40] ^= 1;
        assert!(DrainIntent::decode(&bytes).is_err());
        // Abort it: it stops counting, and the next drain of the same store is a new row.
        let mut txn = store.begin().unwrap();
        let (abort, changed) = super::super::range_abort::plan_range_abort(&mut txn, first.task()).unwrap();
        assert!(changed && abort.target() == super::super::range_abort::AbortTarget::Drain);
        txn.commit().unwrap();
        assert!(committed_drains(&store).unwrap().is_empty(), "an aborted drain is history");
        let mut txn = store.begin().unwrap();
        let (second, changed) = plan_drain(&mut txn, NodeId(7)).unwrap();
        assert!(changed && second.sequence() == 1 && second.operation() != first.operation());
        txn.commit().unwrap();
        assert_eq!(committed_drains(&store).unwrap(), vec![second]);
    }
}
