//! Committed source-replica removal decisions: replicated authority to
//! remove one EXACT source replica (node and store incarnation) from the
//! migration group's voter set.
//!
//! A decision never changes a configuration by itself. It names one of the
//! creation's initial replicas — never the migration destination — and is
//! bound to the committed evidence row: only a destination that provably
//! adopted the pinned image justifies retiring the replica it replaces.
//! One decision per operation; an exact resubmission is a confirmation;
//! any divergence refuses; the row is immutable.

use super::*;

const SOURCE_REMOVAL: u64 = 106;
const RMV_MAGIC: &[u8; 8] = b"KV9RMV01";
pub const MAX_REMOVAL_BYTES: usize = 8 + 32 + 16 + 8 + 8 + 8 + 8 + 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovalDecision {
    root: RootDigest,
    operation: [u8; 16],
    task: u64,
    evidence_task: u64,
    region: RegionId,
    source: InitialReplica,
}

impl RemovalDecision {
    pub fn operation(&self) -> [u8; 16] {
        self.operation
    }
    pub fn root(&self) -> RootDigest {
        self.root
    }
    pub fn task(&self) -> u64 {
        self.task
    }
    pub fn evidence_task(&self) -> u64 {
        self.evidence_task
    }
    pub fn region(&self) -> RegionId {
        self.region
    }
    pub fn source(&self) -> InitialReplica {
        self.source
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(MAX_REMOVAL_BYTES);
        bytes.extend_from_slice(RMV_MAGIC);
        bytes.extend_from_slice(self.root.as_bytes());
        bytes.extend_from_slice(&self.operation);
        bytes.extend_from_slice(&self.task.to_be_bytes());
        bytes.extend_from_slice(&self.evidence_task.to_be_bytes());
        bytes.extend_from_slice(&self.region.0.to_be_bytes());
        bytes.extend_from_slice(&self.source.node.0.to_be_bytes());
        bytes.extend_from_slice(self.source.incarnation.as_bytes());
        bytes
    }

    /// Decode data only. This does not mint committed truncation authority.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != MAX_REMOVAL_BYTES || &bytes[..8] != RMV_MAGIC {
            return Err(invalid("invalid removal decision encoding"));
        }
        let decision = Self {
            root: RootDigest::from_bytes(bytes[8..40].try_into().unwrap()),
            operation: bytes[40..56].try_into().unwrap(),
            task: u64::from_be_bytes(bytes[56..64].try_into().unwrap()),
            evidence_task: u64::from_be_bytes(bytes[64..72].try_into().unwrap()),
            region: RegionId(u64::from_be_bytes(bytes[72..80].try_into().unwrap())),
            source: InitialReplica {
                node: NodeId(u64::from_be_bytes(bytes[80..88].try_into().unwrap())),
                incarnation: StoreIncarnation::from_bytes(bytes[88..104].try_into().unwrap()),
            },
        };
        if decision.root.as_bytes() == &[0; 32]
            || decision.operation == [0; 16]
            || decision.task < FIRST_DYNAMIC_ID
            || decision.evidence_task < FIRST_DYNAMIC_ID
            || decision.task == decision.evidence_task
            || decision.region.0 < FIRST_DYNAMIC_ID
            || decision.source.node.0 == 0
            || decision.source.incarnation.as_bytes() == &[0; 16]
        {
            return Err(invalid("invalid removal decision identities"));
        }
        Ok(decision)
    }
}

fn from_removal_row(row: &Row, root: RootDigest) -> Result<Option<RemovalDecision>> {
    if row.value.get(ColumnId(2)) != Some(&ColumnValue::Uint(SOURCE_REMOVAL)) {
        return Ok(None);
    }
    let Some(ColumnValue::Bytes(bytes)) = row.value.get(ColumnId(3)) else {
        return Err(invalid("removal decision payload is missing"));
    };
    let decision = RemovalDecision::decode(bytes)?;
    if decision.root != root
        || row.pk != [memcmp_uint(decision.task)]
        || row.value.get(ColumnId(1)) != Some(&ColumnValue::Uint(decision.task))
        || row.value.get(ColumnId(4)) != Some(&ColumnValue::Uint(0))
        || row.value.get(ColumnId(5)) != Some(&ColumnValue::Uint(0))
    {
        return Err(invalid("removal decision row binding differs"));
    }
    Ok(Some(decision))
}

fn matches_authority(
    decision: &RemovalDecision,
    evidence: &super::evidence::InstallEvidence,
    creation: &CreationIntent,
) -> Result<()> {
    if evidence.task() != decision.evidence_task
        || evidence.operation() != decision.operation
        || evidence.root() != decision.root
        || evidence.region() != decision.region
    {
        return Err(invalid("decision differs from the committed evidence"));
    }
    // Only an INITIAL replica of the creation may be retired, and never the
    // migration destination the evidence names.
    if !creation.replicas().contains(&decision.source) {
        return Err(invalid("removal names a replica outside the creation"));
    }
    if decision.source == evidence.destination()
        || decision.source.node == evidence.destination().node
    {
        return Err(invalid("removal cannot name the migration destination"));
    }
    Ok(())
}

/// Stage one idempotent removal decision in the serialized, term-fenced
/// catalog transaction. `false` means the exact decision already exists and
/// this commit is a confirmation receipt. The committed evidence row and
/// creation are the authority; nothing here changes a configuration.
pub fn plan_removal<E: Engine>(
    txn: &mut MetaTxn<'_, E>,
    operation: [u8; 16],
    source: InitialReplica,
) -> Result<(RemovalDecision, bool)> {
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("removal requires a certified metadata root"))?;
    let rows = txn.scan(&TASKS_DESC, MAX_CREATION_TASKS + 1)?;
    if rows.len() > MAX_CREATION_TASKS {
        return Err(invalid("bounded removal task scan exceeded"));
    }
    let mut evidence = None;
    for row in &rows {
        if let Some(committed) = super::evidence::from_row_for_siblings(row, root.digest())? {
            if committed.operation() == operation {
                evidence = Some(committed);
            }
        }
    }
    let evidence =
        evidence.ok_or_else(|| invalid("removal requires the committed install evidence"))?;
    let migration = super::migration::committed_migrations_in_txn(txn, &rows, root.digest())?
        .into_iter()
        .find(|m| m.operation() == operation)
        .ok_or_else(|| invalid("removal requires the committed migration"))?;
    let creation_row = txn
        .get(&TASKS_DESC, &[memcmp_uint(migration.creation_task())])?
        .ok_or_else(|| invalid("removal creation is missing"))?;
    let creation = from_row(&creation_row, root.digest())?
        .ok_or_else(|| invalid("removal creation row differs"))?;
    let decision = RemovalDecision {
        root: root.digest(),
        operation,
        task: 0,
        evidence_task: evidence.task(),
        region: evidence.region(),
        source,
    };
    for row in &rows {
        if let Some(previous) = from_removal_row(row, root.digest())? {
            if previous.operation == operation {
                let expected = RemovalDecision {
                    task: previous.task,
                    ..decision.clone()
                };
                if expected != previous {
                    return Err(invalid("removal operation binding conflicts"));
                }
                return Ok((previous, false));
            }
            if previous.region == evidence.region() {
                return Err(invalid("a committed removal already binds this group"));
            }
        }
    }
    let decision = RemovalDecision {
        task: txn.allocate_id(SequenceKind::Task)?,
        ..decision
    };
    matches_authority(&decision, &evidence, &creation)?;
    RemovalDecision::decode(&decision.encode())?;
    if rows.len() == MAX_CREATION_TASKS {
        return Err(invalid("removal task capacity reached"));
    }
    let mut row = RowValue::new();
    for (column, value) in [
        (ColumnId(1), ColumnValue::Uint(decision.task)),
        (ColumnId(2), ColumnValue::Uint(SOURCE_REMOVAL)),
        (ColumnId(3), ColumnValue::Bytes(decision.encode())),
        (ColumnId(4), ColumnValue::Uint(0)),
        (ColumnId(5), ColumnValue::Uint(0)),
    ] {
        row.set(column, value);
    }
    txn.insert(&TASKS_DESC, &[memcmp_uint(decision.task)], row)?;
    Ok((decision, true))
}

/// Read every committed removal decision from one fresh applied snapshot,
/// with the exact evidence/creation rows re-validated. Decisions never expire.
pub fn committed_removals<E: Engine>(store: &MetaStore<E>) -> Result<Vec<RemovalDecision>> {
    let txn = store.begin()?;
    let root = crate::root::certified_root(&txn)?
        .ok_or_else(|| invalid("removal readback requires a certified metadata root"))?;
    let rows = txn.scan(&TASKS_DESC, MAX_CREATION_TASKS + 1)?;
    if rows.len() > MAX_CREATION_TASKS {
        return Err(invalid("bounded removal task scan exceeded"));
    }
    let migrations = super::migration::committed_migrations_in_txn(&txn, &rows, root.digest())?;
    let mut out: Vec<RemovalDecision> = Vec::new();
    for row in &rows {
        if let Some(decision) = from_removal_row(row, root.digest())? {
            let mut evidence = None;
            for other in &rows {
                if let Some(committed) =
                    super::evidence::from_row_for_siblings(other, root.digest())?
                {
                    if committed.operation() == decision.operation {
                        evidence = Some(committed);
                    }
                }
            }
            let evidence = evidence
                .ok_or_else(|| invalid("removal requires the committed install evidence"))?;
            let migration = migrations
                .iter()
                .find(|m| m.operation() == decision.operation)
                .ok_or_else(|| invalid("removal requires the committed migration"))?;
            let creation_row = txn
                .get(&TASKS_DESC, &[memcmp_uint(migration.creation_task())])?
                .ok_or_else(|| invalid("removal creation is missing"))?;
            let creation = from_row(&creation_row, root.digest())?
                .ok_or_else(|| invalid("removal creation row differs"))?;
            matches_authority(&decision, &evidence, &creation)?;
            if out.iter().any(|prior| {
                prior.task == decision.task
                    || prior.operation == decision.operation
                    || prior.region == decision.region
            }) {
                return Err(invalid("duplicate removal identity"));
            }
            out.push(decision);
        }
    }
    Ok(out)
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

    fn settled_operation(store: &MetaStore<kv9_engine::MemEngine>) -> ([u8; 16], CreationIntent) {
        register_node(store, 4, [40; 16]);
        let mut txn = store.begin().unwrap();
        let creation =
            plan_empty_group(&mut txn, [1; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        super::super::activation::plan_activation(&mut txn, &creation).unwrap();
        let (migration, _) =
            super::super::migration::plan_migration(&mut txn, [2; 16], creation.task(), NodeId(4))
                .unwrap();
        let receipt = super::super::evidence::InstallEvidence::receipt(
            &migration,
            StoreIncarnation::from_bytes([41; 16]),
            RootDigest::sha256(b"image"),
            [42; 32],
            kv9_common::AppliedPosition { term: 3, index: 12 },
        )
        .unwrap();
        super::super::evidence::plan_install_evidence(&mut txn, &receipt).unwrap();
        txn.commit().unwrap();
        ([2; 16], creation)
    }

    #[test]
    fn a_removal_names_only_an_initial_replica_and_never_the_destination() {
        let store = super::super::tests::fixture();
        {
            let mut txn = store.begin().unwrap();
            let outsider = InitialReplica {
                node: NodeId(1),
                incarnation: StoreIncarnation::from_bytes([10; 16]),
            };
            assert!(
                plan_removal(&mut txn, [2; 16], outsider).is_err(),
                "no evidence yet: refuse"
            );
        }
        let (operation, creation) = settled_operation(&store);
        let mut txn = store.begin().unwrap();
        let destination = InitialReplica {
            node: NodeId(4),
            incarnation: StoreIncarnation::from_bytes([40; 16]),
        };
        assert!(
            plan_removal(&mut txn, operation, destination).is_err(),
            "the migration destination can never be the removed replica"
        );
        let foreign = InitialReplica {
            node: creation.replicas()[0].node,
            incarnation: StoreIncarnation::from_bytes([99; 16]),
        };
        assert!(
            plan_removal(&mut txn, operation, foreign).is_err(),
            "a replaced disk is not the creation replica"
        );
        let source = creation.replicas()[2];
        let (planned, changed) = plan_removal(&mut txn, operation, source).unwrap();
        assert!(changed);
        assert_eq!(planned.source(), source);
        assert!(committed_removals(&store).unwrap().is_empty());
        txn.commit().unwrap();
        assert_eq!(committed_removals(&store).unwrap(), vec![planned.clone()]);
        let mut txn = store.begin().unwrap();
        let (retry, changed) = plan_removal(&mut txn, operation, source).unwrap();
        assert!(!changed);
        assert_eq!(retry, planned);
        assert!(
            plan_removal(&mut txn, operation, creation.replicas()[1]).is_err(),
            "one operation never retires a second replica"
        );
    }

    #[test]
    fn removal_readback_refuses_row_and_authority_rebinding() {
        for defect in [
            "task",
            "state",
            "created",
            "truncated",
            "operation",
            "source-node",
        ] {
            let store = super::super::tests::fixture();
            let (operation, creation) = settled_operation(&store);
            let mut txn = store.begin().unwrap();
            let (planned, _) = plan_removal(&mut txn, operation, creation.replicas()[2]).unwrap();
            txn.commit().unwrap();
            let mut txn = store.begin().unwrap();
            let row = txn
                .get(&TASKS_DESC, &[memcmp_uint(planned.task())])
                .unwrap()
                .unwrap();
            let change = match defect {
                "task" => (ColumnId(1), ColumnValue::Uint(999)),
                "state" => (ColumnId(4), ColumnValue::Uint(1)),
                "created" => (ColumnId(5), ColumnValue::Uint(1)),
                "truncated" => (
                    ColumnId(3),
                    ColumnValue::Bytes(planned.encode()[..80].to_vec()),
                ),
                "operation" | "source-node" => {
                    let mut bytes = planned.encode();
                    bytes[if defect == "operation" { 40 } else { 87 }] ^= 1;
                    (ColumnId(3), ColumnValue::Bytes(bytes))
                }
                _ => unreachable!(),
            };
            if defect == "task" {
                assert!(txn.update(&TASKS_DESC, &row.pk, vec![change]).is_err());
                continue;
            }
            txn.update(&TASKS_DESC, &row.pk, vec![change]).unwrap();
            txn.commit().unwrap();
            assert!(committed_removals(&store).is_err(), "accepted {defect}");
        }
    }
}
