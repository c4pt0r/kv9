//! Immutable, replicated migration intents: committed authority to bind one
//! image's retention owners for one destination replica, and nothing more.
//!
//! An intent never moves data, admits Raft traffic, starts a peer, changes
//! membership or releases a source pin. It binds the destination's current
//! exact store incarnation at planning time; a replacement disk cannot reuse
//! the operation. One live migration row per group bounds concurrent
//! configuration authority to a single committed choice.

use super::*;

const MIGRATE_REPLICA: u64 = 103;
const MIG_MAGIC: &[u8; 8] = b"KV9MIG01";
pub const MAX_MIGRATION_BYTES: usize = 8 + 32 + 16 + 8 + 8 + 8 + 8 + 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationIntent {
    root: RootDigest,
    operation: [u8; 16],
    task: u64,
    creation_task: u64,
    region: RegionId,
    destination: InitialReplica,
}

impl MigrationIntent {
    pub fn operation(&self) -> [u8; 16] {
        self.operation
    }
    pub fn root(&self) -> RootDigest {
        self.root
    }
    pub fn task(&self) -> u64 {
        self.task
    }
    pub fn creation_task(&self) -> u64 {
        self.creation_task
    }
    pub fn region(&self) -> RegionId {
        self.region
    }
    pub fn destination(&self) -> InitialReplica {
        self.destination
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(MAX_MIGRATION_BYTES);
        bytes.extend_from_slice(MIG_MAGIC);
        bytes.extend_from_slice(self.root.as_bytes());
        bytes.extend_from_slice(&self.operation);
        bytes.extend_from_slice(&self.task.to_be_bytes());
        bytes.extend_from_slice(&self.creation_task.to_be_bytes());
        bytes.extend_from_slice(&self.region.0.to_be_bytes());
        bytes.extend_from_slice(&self.destination.node.0.to_be_bytes());
        bytes.extend_from_slice(self.destination.incarnation.as_bytes());
        bytes
    }

    /// Decode data only. This does not mint committed migration authority.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != MAX_MIGRATION_BYTES || &bytes[..8] != MIG_MAGIC {
            return Err(invalid("invalid migration intent encoding"));
        }
        let intent = Self {
            root: RootDigest::from_bytes(bytes[8..40].try_into().unwrap()),
            operation: bytes[40..56].try_into().unwrap(),
            task: u64::from_be_bytes(bytes[56..64].try_into().unwrap()),
            creation_task: u64::from_be_bytes(bytes[64..72].try_into().unwrap()),
            region: RegionId(u64::from_be_bytes(bytes[72..80].try_into().unwrap())),
            destination: InitialReplica {
                node: NodeId(u64::from_be_bytes(bytes[80..88].try_into().unwrap())),
                incarnation: StoreIncarnation::from_bytes(bytes[88..].try_into().unwrap()),
            },
        };
        if intent.root.as_bytes() == &[0; 32]
            || intent.operation == [0; 16]
            || intent.task < FIRST_DYNAMIC_ID
            || intent.creation_task < FIRST_DYNAMIC_ID
            || intent.task == intent.creation_task
            || intent.region.0 < FIRST_DYNAMIC_ID
            || intent.destination.node.0 == 0
            || intent.destination.incarnation.as_bytes() == &[0; 16]
        {
            return Err(invalid("invalid migration intent identities"));
        }
        Ok(intent)
    }

    fn matches_creation(&self, creation: &CreationIntent) -> Result<()> {
        if creation.task() != self.creation_task
            || creation.region() != self.region
            || creation.root() != self.root
        {
            return Err(invalid("migration differs from exact creation"));
        }
        if creation.replicas().iter().any(|r| {
            r.node == self.destination.node || r.incarnation == self.destination.incarnation
        }) {
            return Err(invalid("migration destination is an initial replica"));
        }
        Ok(())
    }
}

/// Only committed readback constructs this capability; a planner overlay or a
/// decoded RPC cannot. It proves both exact rows and the activation desire.
#[derive(Debug, Clone)]
pub struct CommittedMigration {
    intent: MigrationIntent,
    creation: CommittedCreation,
}

impl CommittedMigration {
    pub fn intent(&self) -> &MigrationIntent {
        &self.intent
    }
    pub fn creation(&self) -> &CommittedCreation {
        &self.creation
    }
}

fn rows<E: Engine>(txn: &MetaTxn<'_, E>) -> Result<Vec<Row>> {
    let rows = txn.scan(&TASKS_DESC, MAX_CREATION_TASKS + 1)?;
    if rows.len() > MAX_CREATION_TASKS {
        return Err(invalid("bounded migration task scan exceeded"));
    }
    Ok(rows)
}

fn decode<E: Engine>(
    txn: &MetaTxn<'_, E>,
    rows: &[Row],
    root: RootDigest,
) -> Result<Vec<(MigrationIntent, CreationIntent)>> {
    let activations = activation::decode(txn, rows, root)?;
    let mut intents: Vec<(MigrationIntent, CreationIntent)> = Vec::new();
    for row in rows {
        if row.value.get(ColumnId(2)) != Some(&ColumnValue::Uint(MIGRATE_REPLICA)) {
            continue;
        }
        let Some(ColumnValue::Bytes(bytes)) = row.value.get(ColumnId(3)) else {
            return Err(invalid("migration task payload is missing"));
        };
        let intent = MigrationIntent::decode(bytes)?;
        if intent.root != root
            || row.pk != [memcmp_uint(intent.task)]
            || row.value.get(ColumnId(1)) != Some(&ColumnValue::Uint(intent.task))
            || row.value.get(ColumnId(4)) != Some(&ColumnValue::Uint(0))
            || row.value.get(ColumnId(5)) != Some(&ColumnValue::Uint(0))
        {
            return Err(invalid("migration task row binding differs"));
        }
        let creation = txn
            .get(&TASKS_DESC, &[memcmp_uint(intent.creation_task)])?
            .ok_or_else(|| invalid("migration creation is missing"))?;
        let creation =
            from_row(&creation, root)?.ok_or_else(|| invalid("migration creation row differs"))?;
        intent.matches_creation(&creation)?;
        if !activations.contains(&creation) {
            return Err(invalid("migration requires the committed activation"));
        }
        if intents.iter().any(|(prior, _)| {
            prior.task == intent.task
                || prior.region == intent.region
                || prior.operation == intent.operation
        }) {
            return Err(invalid("duplicate migration identity"));
        }
        intents.push((intent, creation));
    }
    Ok(intents)
}

/// Stage one idempotent migration intent in the serialized, term-fenced
/// catalog transaction. `false` means the exact intent already exists and the
/// new commit is a confirmation receipt. Any divergence refuses; nothing here
/// grants transfer, voting, serving or pin-release capability.
pub fn plan_migration<E: Engine>(
    txn: &mut MetaTxn<'_, E>,
    operation: [u8; 16],
    creation_task: u64,
    destination: NodeId,
) -> Result<(MigrationIntent, bool)> {
    if operation == [0; 16] {
        return Err(invalid("migration needs a nonzero operation"));
    }
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("migration requires a certified metadata root"))?;
    let creation_row = txn
        .get(&TASKS_DESC, &[memcmp_uint(creation_task)])?
        .ok_or_else(|| invalid("migration creation is missing"))?;
    let creation = from_row(&creation_row, root.digest())?
        .ok_or_else(|| invalid("migration creation row differs"))?;
    let rows = rows(txn)?;
    if !activation::decode(txn, &rows, root.digest())?.contains(&creation) {
        return Err(invalid("migration requires the committed activation"));
    }
    let endpoint = crate::endpoint::node_endpoint(txn, destination)?
        .filter(|endpoint| endpoint.active)
        .ok_or_else(|| invalid("migration destination is not an active registered store"))?;
    let intent = MigrationIntent {
        root: root.digest(),
        operation,
        task: 0,
        creation_task,
        region: creation.region(),
        destination: InitialReplica {
            node: destination,
            incarnation: endpoint.incarnation,
        },
    };
    intent.matches_creation(&creation)?;
    for (previous, _) in decode(txn, &rows, root.digest())? {
        if previous.operation == operation {
            let expected = MigrationIntent {
                task: previous.task,
                ..intent.clone()
            };
            if expected != previous {
                return Err(invalid("migration operation binding conflicts"));
            }
            return Ok((previous, false));
        }
        if previous.region == creation.region() {
            return Err(invalid("a committed migration already binds this group"));
        }
    }
    if rows.len() == MAX_CREATION_TASKS {
        return Err(invalid("migration task capacity reached"));
    }
    let intent = MigrationIntent {
        task: txn.allocate_id(SequenceKind::Task)?,
        ..intent
    };
    MigrationIntent::decode(&intent.encode())?;
    let mut row = RowValue::new();
    for (column, value) in [
        (ColumnId(1), ColumnValue::Uint(intent.task)),
        (ColumnId(2), ColumnValue::Uint(MIGRATE_REPLICA)),
        (ColumnId(3), ColumnValue::Bytes(intent.encode())),
        (ColumnId(4), ColumnValue::Uint(0)),
        (ColumnId(5), ColumnValue::Uint(0)),
    ] {
        row.set(column, value);
    }
    txn.insert(&TASKS_DESC, &[memcmp_uint(intent.task)], row)?;
    Ok((intent, true))
}

/// Decode the committed migration intents visible in an already-scanned row
/// set, with creation/activation cross-validation. Shared with the evidence
/// planner so both readers apply identical committed-authority rules.
pub(crate) fn committed_migrations_in_txn<E: Engine>(
    txn: &MetaTxn<'_, E>,
    rows: &[Row],
    root: RootDigest,
) -> Result<Vec<MigrationIntent>> {
    Ok(decode(txn, rows, root)?
        .into_iter()
        .map(|(intent, _)| intent)
        .collect())
}

/// Read every committed migration from one fresh applied snapshot, with the
/// exact creation and activation rows re-validated. Immutable intents do not
/// expire or cancel; followers can reconcile without a control-plane leader.
pub fn committed_migrations<E: Engine>(store: &MetaStore<E>) -> Result<Vec<CommittedMigration>> {
    let txn = store.begin()?;
    let root = crate::root::certified_root(&txn)?
        .ok_or_else(|| invalid("migration readback requires a certified metadata root"))?;
    Ok(decode(&txn, &rows(&txn)?, root.digest())?
        .into_iter()
        .map(|(intent, creation)| CommittedMigration {
            intent,
            creation: CommittedCreation(creation),
        })
        .collect())
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

    fn committed_group(store: &MetaStore<kv9_engine::MemEngine>, operation: u8) -> CreationIntent {
        let mut txn = store.begin().unwrap();
        let intent = plan_empty_group(
            &mut txn,
            [operation; 16],
            &[NodeId(1), NodeId(2), NodeId(3)],
        )
        .unwrap();
        activation::plan_activation(&mut txn, &intent).unwrap();
        txn.commit().unwrap();
        intent
    }

    #[test]
    fn migration_requires_committed_activated_creation_and_active_destination() {
        let store = super::super::tests::fixture();
        register_node(&store, 4, [40; 16]);
        let mut txn = store.begin().unwrap();
        let creation =
            plan_empty_group(&mut txn, [1; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        assert!(
            plan_migration(&mut txn, [2; 16], creation.task(), NodeId(4)).is_err(),
            "an unactivated creation must not accept a migration"
        );
        activation::plan_activation(&mut txn, &creation).unwrap();
        // The uncommitted overlay plans, but readback must stay empty.
        plan_migration(&mut txn, [2; 16], creation.task(), NodeId(4)).unwrap();
        assert!(committed_migrations(&store).unwrap().is_empty());
        drop(txn);
        let creation = committed_group(&store, 1);
        let mut txn = store.begin().unwrap();
        assert!(
            plan_migration(&mut txn, [2; 16], creation.task() + 100, NodeId(4)).is_err(),
            "a missing creation row must refuse"
        );
        assert!(
            plan_migration(&mut txn, [2; 16], creation.task(), NodeId(9)).is_err(),
            "an unregistered destination must refuse"
        );
        assert!(
            plan_migration(&mut txn, [2; 16], creation.task(), NodeId(3)).is_err(),
            "an initial replica cannot be the migration destination"
        );
        assert!(
            plan_migration(&mut txn, [0; 16], creation.task(), NodeId(4)).is_err(),
            "a zero operation must refuse"
        );
        let (planned, changed) =
            plan_migration(&mut txn, [2; 16], creation.task(), NodeId(4)).unwrap();
        assert!(changed);
        assert_eq!(planned.destination().node, NodeId(4));
        assert_eq!(
            planned.destination().incarnation,
            StoreIncarnation::from_bytes([40; 16])
        );
        assert_eq!(planned.region(), creation.region());
        txn.commit().unwrap();
        let committed = committed_migrations(&store).unwrap();
        assert_eq!(committed.len(), 1);
        assert_eq!(committed[0].intent(), &planned);
        assert_eq!(committed[0].creation().intent(), &creation);
    }

    #[test]
    fn migration_retry_is_idempotent_and_divergence_refuses() {
        let store = super::super::tests::fixture();
        register_node(&store, 4, [40; 16]);
        register_node(&store, 5, [50; 16]);
        let creation = committed_group(&store, 1);
        let mut txn = store.begin().unwrap();
        let (planned, _) = plan_migration(&mut txn, [2; 16], creation.task(), NodeId(4)).unwrap();
        txn.commit().unwrap();
        let mut txn = store.begin().unwrap();
        let (retry, changed) =
            plan_migration(&mut txn, [2; 16], creation.task(), NodeId(4)).unwrap();
        assert!(!changed, "an exact retry is a confirmation receipt");
        assert_eq!(retry, planned);
        assert!(
            plan_migration(&mut txn, [2; 16], creation.task(), NodeId(5)).is_err(),
            "a changed destination cannot reuse the operation"
        );
        assert!(
            plan_migration(&mut txn, [3; 16], creation.task(), NodeId(5)).is_err(),
            "a second live migration for the same group must refuse"
        );
        drop(txn);
        // A replacement disk (new incarnation) cannot reuse the operation.
        let mut txn = store.begin().unwrap();
        txn.update(
            &TASKS_DESC,
            &[memcmp_uint(planned.task())],
            vec![(ColumnId(2), ColumnValue::Uint(MIGRATE_REPLICA))],
        )
        .unwrap();
        txn.commit().unwrap();
        register_node(&store, 6, [60; 16]);
        let second = committed_group(&store, 7);
        let mut txn = store.begin().unwrap();
        let (other, changed) = plan_migration(&mut txn, [8; 16], second.task(), NodeId(6)).unwrap();
        assert!(changed, "an independent group accepts its own migration");
        assert_ne!(other.task(), planned.task());
        txn.commit().unwrap();
        assert_eq!(committed_migrations(&store).unwrap().len(), 2);
    }

    #[test]
    fn migration_readback_refuses_row_creation_and_activation_rebinding() {
        for defect in [
            "task",
            "state",
            "created",
            "truncated",
            "root",
            "region",
            "creation-task",
            "creation-state",
            "duplicate",
        ] {
            let store = super::super::tests::fixture();
            register_node(&store, 4, [40; 16]);
            let creation = committed_group(&store, 1);
            let mut txn = store.begin().unwrap();
            let (planned, _) =
                plan_migration(&mut txn, [2; 16], creation.task(), NodeId(4)).unwrap();
            txn.commit().unwrap();
            let mut txn = store.begin().unwrap();
            let request = txn
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
                // Operation and destination are the row's own immutable
                // identity; cross-bound fields must refuse on any divergence.
                "root" | "region" | "creation-task" => {
                    let mut bytes = planned.encode();
                    bytes[match defect {
                        "root" => 8,
                        "region" => 79,
                        _ => 71,
                    }] ^= 1;
                    (ColumnId(3), ColumnValue::Bytes(bytes))
                }
                "creation-state" => {
                    txn.update(
                        &TASKS_DESC,
                        &[memcmp_uint(creation.task())],
                        vec![(ColumnId(4), ColumnValue::Uint(1))],
                    )
                    .unwrap();
                    txn.commit().unwrap();
                    assert!(committed_migrations(&store).is_err(), "accepted {defect}");
                    continue;
                }
                "duplicate" => {
                    let task = txn.allocate_id(SequenceKind::Task).unwrap();
                    let mut row = request.value.clone();
                    row.set(ColumnId(1), ColumnValue::Uint(task));
                    txn.insert(&TASKS_DESC, &[memcmp_uint(task)], row).unwrap();
                    txn.commit().unwrap();
                    assert!(committed_migrations(&store).is_err(), "accepted {defect}");
                    continue;
                }
                _ => unreachable!(),
            };
            if defect == "task" {
                assert!(txn.update(&TASKS_DESC, &request.pk, vec![change]).is_err());
                continue;
            }
            txn.update(&TASKS_DESC, &request.pk, vec![change]).unwrap();
            txn.commit().unwrap();
            assert!(committed_migrations(&store).is_err(), "accepted {defect}");
        }
    }

    #[test]
    fn migration_respects_shared_task_capacity() {
        let store = super::super::tests::fixture();
        register_node(&store, 4, [40; 16]);
        let creation = committed_group(&store, 1);
        let mut txn = store.begin().unwrap();
        // Two committed rows exist; fill to exactly the shared budget.
        for id in 2..=254 {
            plan_empty_group(&mut txn, [id; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        }
        txn.commit().unwrap();
        let mut txn = store.begin().unwrap();
        assert!(
            plan_migration(&mut txn, [255; 16], creation.task(), NodeId(4)).is_err(),
            "the shared 255-row budget bounds migration rows"
        );
        drop(txn);
        assert!(committed_migrations(&store).unwrap().is_empty());
    }
}
