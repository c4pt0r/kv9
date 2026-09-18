//! Committed migration abort: the replicated decision that one described
//! migration operation is permanently abandoned — the stranded-destination
//! recovery. A destination store incarnation lost mid-operation can never
//! adopt or evidence its image, so without this row the operation wedges
//! forever: the source pin sticks at Published, truncation stays blocked
//! and the attached learner stays in the source configuration.
//!
//! An abort never moves data, deletes an image, or touches the retention
//! ledger by itself. It is the committed fact the ledger's migration
//! quiesce fence consults as the ALTERNATIVE to install evidence; the two
//! are mutually exclusive per operation, permanently: evidence planning
//! refuses on a committed abort and abort planning refuses on committed
//! evidence. One abort row per operation; an exact resubmission is a
//! confirmation receipt; any divergence refuses.

use super::*;

const MIGRATION_ABORT: u64 = 108;
const ABT_MAGIC: &[u8; 8] = b"KV9ABT01";
pub const MAX_ABORT_BYTES: usize = 8 + 32 + 16 + 8 + 8 + 8 + 8 + 16 + 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationAbort {
    root: RootDigest,
    operation: [u8; 16],
    task: u64,
    migration_task: u64,
    region: RegionId,
    destination: InitialReplica,
    subject: [u8; 32],
}

impl MigrationAbort {
    pub fn operation(&self) -> [u8; 16] {
        self.operation
    }
    pub fn root(&self) -> RootDigest {
        self.root
    }
    pub fn task(&self) -> u64 {
        self.task
    }
    pub fn migration_task(&self) -> u64 {
        self.migration_task
    }
    pub fn region(&self) -> RegionId {
        self.region
    }
    pub fn destination(&self) -> InitialReplica {
        self.destination
    }
    pub fn subject(&self) -> [u8; 32] {
        self.subject
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(MAX_ABORT_BYTES);
        bytes.extend_from_slice(ABT_MAGIC);
        bytes.extend_from_slice(self.root.as_bytes());
        bytes.extend_from_slice(&self.operation);
        bytes.extend_from_slice(&self.task.to_be_bytes());
        bytes.extend_from_slice(&self.migration_task.to_be_bytes());
        bytes.extend_from_slice(&self.region.0.to_be_bytes());
        bytes.extend_from_slice(&self.destination.node.0.to_be_bytes());
        bytes.extend_from_slice(self.destination.incarnation.as_bytes());
        bytes.extend_from_slice(&self.subject);
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != MAX_ABORT_BYTES || &bytes[..8] != ABT_MAGIC {
            return Err(invalid("invalid migration abort encoding"));
        }
        let abort = Self {
            root: RootDigest::from_bytes(bytes[8..40].try_into().unwrap()),
            operation: bytes[40..56].try_into().unwrap(),
            task: u64::from_be_bytes(bytes[56..64].try_into().unwrap()),
            migration_task: u64::from_be_bytes(bytes[64..72].try_into().unwrap()),
            region: RegionId(u64::from_be_bytes(bytes[72..80].try_into().unwrap())),
            destination: InitialReplica {
                node: NodeId(u64::from_be_bytes(bytes[80..88].try_into().unwrap())),
                incarnation: StoreIncarnation::from_bytes(bytes[88..104].try_into().unwrap()),
            },
            subject: bytes[104..136].try_into().unwrap(),
        };
        if abort.root.as_bytes() == &[0; 32]
            || abort.operation == [0; 16]
            || abort.task < FIRST_DYNAMIC_ID
            || abort.migration_task < FIRST_DYNAMIC_ID
            || abort.task == abort.migration_task
            || abort.region.0 < FIRST_DYNAMIC_ID
            || abort.destination.node.0 == 0
            || abort.destination.incarnation.as_bytes() == &[0; 16]
            || abort.subject == [0; 32]
        {
            return Err(invalid("invalid migration abort identities"));
        }
        Ok(abort)
    }
}

pub(crate) fn from_abort_row(row: &Row, root: RootDigest) -> Result<Option<MigrationAbort>> {
    if row.value.get(ColumnId(2)) != Some(&ColumnValue::Uint(MIGRATION_ABORT)) {
        return Ok(None);
    }
    let Some(ColumnValue::Bytes(bytes)) = row.value.get(ColumnId(3)) else {
        return Err(invalid("migration abort payload is missing"));
    };
    let abort = MigrationAbort::decode(bytes)?;
    if abort.root != root
        || row.pk != [memcmp_uint(abort.task)]
        || row.value.get(ColumnId(1)) != Some(&ColumnValue::Uint(abort.task))
        || row.value.get(ColumnId(4)) != Some(&ColumnValue::Uint(0))
        || row.value.get(ColumnId(5)) != Some(&ColumnValue::Uint(0))
    {
        return Err(invalid("migration abort row binding differs"));
    }
    Ok(Some(abort))
}

fn matches_migration(
    abort: &MigrationAbort,
    migration: &super::migration::MigrationIntent,
) -> Result<()> {
    if migration.task() != abort.migration_task
        || migration.operation() != abort.operation
        || migration.root() != abort.root
        || migration.region() != abort.region
        || migration.destination() != abort.destination
    {
        return Err(invalid("abort differs from the committed migration"));
    }
    Ok(())
}

/// Stage one idempotent abort row in the serialized, term-fenced catalog
/// transaction. `false` means the exact abort already exists and this commit
/// is a confirmation receipt. The committed migration row is the authority
/// the abort must match; COMMITTED INSTALL EVIDENCE for the operation
/// refuses permanently — a completed transfer is never abandoned. Nothing
/// here quiesces or releases a pin, detaches a learner, or deletes data.
pub fn plan_abort<E: Engine>(
    txn: &mut MetaTxn<'_, E>,
    operation: [u8; 16],
    subject: [u8; 32],
) -> Result<(MigrationAbort, bool)> {
    if subject == [0; 32] {
        return Err(invalid("abort requires the published image subject"));
    }
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("abort requires a certified metadata root"))?;
    let rows = txn.scan(&TASKS_DESC, MAX_CREATION_TASKS + 1)?;
    if rows.len() > MAX_CREATION_TASKS {
        return Err(invalid("bounded abort task scan exceeded"));
    }
    let migration = super::migration::committed_migrations_in_txn(txn, &rows, root.digest())?
        .into_iter()
        .find(|m| m.operation() == operation)
        .ok_or_else(|| invalid("abort requires the committed migration"))?;
    let abort = MigrationAbort {
        root: root.digest(),
        operation,
        task: 0,
        migration_task: migration.task(),
        region: migration.region(),
        destination: migration.destination(),
        subject,
    };
    let mut existing = None;
    for row in &rows {
        if super::evidence::from_row_for_siblings(row, root.digest())?
            .is_some_and(|e| e.operation() == operation)
        {
            return Err(invalid(
                "committed install evidence forbids aborting a completed operation",
            ));
        }
        if let Some(previous) = from_abort_row(row, root.digest())? {
            if previous.operation == operation {
                let expected = MigrationAbort {
                    task: previous.task,
                    ..abort.clone()
                };
                if expected != previous {
                    return Err(invalid("abort operation binding conflicts"));
                }
                existing = Some(previous);
            }
        }
    }
    if let Some(previous) = existing {
        return Ok((previous, false));
    }
    if rows.len() == MAX_CREATION_TASKS {
        return Err(invalid("abort task capacity reached"));
    }
    let abort = MigrationAbort {
        task: txn.allocate_id(SequenceKind::Task)?,
        ..abort
    };
    MigrationAbort::decode(&abort.encode())?;
    matches_migration(&abort, &migration)?;
    let mut row = RowValue::new();
    for (column, value) in [
        (ColumnId(1), ColumnValue::Uint(abort.task)),
        (ColumnId(2), ColumnValue::Uint(MIGRATION_ABORT)),
        (ColumnId(3), ColumnValue::Bytes(abort.encode())),
        (ColumnId(4), ColumnValue::Uint(0)),
        (ColumnId(5), ColumnValue::Uint(0)),
    ] {
        row.set(column, value);
    }
    txn.insert(&TASKS_DESC, &[memcmp_uint(abort.task)], row)?;
    Ok((abort, true))
}

/// Read every committed abort from one fresh applied snapshot with the exact
/// migration rows re-validated and the evidence exclusion re-checked. Aborts
/// are immutable and never expire.
pub fn committed_aborts<E: Engine>(store: &MetaStore<E>) -> Result<Vec<MigrationAbort>> {
    let txn = store.begin()?;
    let root = crate::root::certified_root(&txn)?
        .ok_or_else(|| invalid("abort readback requires a certified metadata root"))?;
    let rows = txn.scan(&TASKS_DESC, MAX_CREATION_TASKS + 1)?;
    if rows.len() > MAX_CREATION_TASKS {
        return Err(invalid("bounded abort task scan exceeded"));
    }
    let migrations = super::migration::committed_migrations_in_txn(&txn, &rows, root.digest())?;
    let mut out: Vec<MigrationAbort> = Vec::new();
    for row in &rows {
        if let Some(abort) = from_abort_row(row, root.digest())? {
            let migration = migrations
                .iter()
                .find(|m| m.operation() == abort.operation)
                .ok_or_else(|| invalid("abort requires the committed migration"))?;
            matches_migration(&abort, migration)?;
            if rows.iter().any(|other| {
                super::evidence::from_row_for_siblings(other, root.digest())
                    .ok()
                    .flatten()
                    .is_some_and(|e| e.operation() == abort.operation)
            }) {
                return Err(invalid("an operation carries both evidence and an abort"));
            }
            if out
                .iter()
                .any(|prior| prior.task == abort.task || prior.operation == abort.operation)
            {
                return Err(invalid("duplicate abort identity"));
            }
            out.push(abort);
        }
    }
    Ok(out)
}

/// The ledger fence's view-level probe: does a committed abort row exist
/// whose derived retention operation digest and subject match this owner
/// pair? Mirrors `evidence_matches_owner_pair` exactly — same bounded raw
/// scan of the SAME applied view, same refusal in the safe direction.
pub fn abort_matches_owner_pair(
    view: &dyn kv9_engine::ReadView,
    root: RootDigest,
    operation_digest: [u8; 32],
    subject: [u8; 32],
) -> Result<bool> {
    use kv9_engine::ColumnFamily;
    let (start, end) = crate::codec::row_range(TASKS_DESC.id)?;
    let entries = view.scan(ColumnFamily::Default, &start, &end, MAX_CREATION_TASKS + 1)?;
    if entries.len() > MAX_CREATION_TASKS {
        return Err(invalid("bounded abort task scan exceeded"));
    }
    for (key, value) in entries {
        let row_value = RowValue::decode(&value)?;
        if row_value.get(ColumnId(2)) != Some(&ColumnValue::Uint(MIGRATION_ABORT)) {
            continue;
        }
        let Some(ColumnValue::Bytes(bytes)) = row_value.get(ColumnId(3)) else {
            return Err(invalid("migration abort payload is missing"));
        };
        let abort = MigrationAbort::decode(bytes)?;
        if abort.root != root
            || key != crate::codec::encode_row_key(TASKS_DESC.id, &[memcmp_uint(abort.task)])?
            || row_value.get(ColumnId(1)) != Some(&ColumnValue::Uint(abort.task))
        {
            return Err(invalid("migration abort row binding differs"));
        }
        if super::evidence::migration_operation_digest(root, abort.operation) == operation_digest
            && abort.subject == subject
        {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::NODES_DESC;
    use kv9_common::AppliedPosition;

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

    fn committed_migration(
        store: &MetaStore<kv9_engine::MemEngine>,
    ) -> super::super::migration::MigrationIntent {
        register_node(store, 4, [40; 16]);
        let mut txn = store.begin().unwrap();
        let creation =
            plan_empty_group(&mut txn, [1; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        super::super::activation::plan_activation(&mut txn, &creation).unwrap();
        let (migration, _) =
            super::super::migration::plan_migration(&mut txn, [2; 16], creation.task(), NodeId(4))
                .unwrap();
        txn.commit().unwrap();
        migration
    }

    #[test]
    fn an_abort_binds_the_committed_migration_once_and_confirms_idempotently() {
        let store = super::super::tests::fixture();
        let migration = committed_migration(&store);
        let mut txn = store.begin().unwrap();
        assert!(
            plan_abort(&mut txn, [9; 16], [42; 32]).is_err(),
            "an uncommitted operation must refuse"
        );
        assert!(
            plan_abort(&mut txn, migration.operation(), [0; 32]).is_err(),
            "a zero subject must refuse"
        );
        let (planned, changed) = plan_abort(&mut txn, migration.operation(), [42; 32]).unwrap();
        assert!(changed);
        assert_eq!(planned.operation(), migration.operation());
        assert_eq!(planned.migration_task(), migration.task());
        assert_eq!(planned.destination(), migration.destination());
        assert_eq!(planned.region(), migration.region());
        assert!(committed_aborts(&store).unwrap().is_empty());
        txn.commit().unwrap();
        assert_eq!(committed_aborts(&store).unwrap(), vec![planned.clone()]);
        let mut txn = store.begin().unwrap();
        let (retry, changed) = plan_abort(&mut txn, migration.operation(), [42; 32]).unwrap();
        assert!(!changed);
        assert_eq!(retry, planned);
        assert!(
            plan_abort(&mut txn, migration.operation(), [43; 32]).is_err(),
            "one operation must never name a second abort subject"
        );
    }

    #[test]
    fn evidence_and_abort_are_mutually_exclusive_permanently() {
        // Abort first: evidence must refuse forever.
        let store = super::super::tests::fixture();
        let migration = committed_migration(&store);
        let mut txn = store.begin().unwrap();
        plan_abort(&mut txn, migration.operation(), [42; 32]).unwrap();
        txn.commit().unwrap();
        let receipt = super::super::evidence::InstallEvidence::receipt(
            &migration,
            StoreIncarnation::from_bytes([41; 16]),
            RootDigest::sha256(b"image"),
            [42; 32],
            AppliedPosition { term: 3, index: 12 },
        )
        .unwrap();
        let mut txn = store.begin().unwrap();
        assert!(
            super::super::evidence::plan_install_evidence(&mut txn, &receipt).is_err(),
            "a committed abort forbids evidencing"
        );
        drop(txn);
        // Evidence first: abort must refuse forever.
        let store = super::super::tests::fixture();
        let migration = committed_migration(&store);
        let receipt = super::super::evidence::InstallEvidence::receipt(
            &migration,
            StoreIncarnation::from_bytes([41; 16]),
            RootDigest::sha256(b"image"),
            [42; 32],
            AppliedPosition { term: 3, index: 12 },
        )
        .unwrap();
        let mut txn = store.begin().unwrap();
        super::super::evidence::plan_install_evidence(&mut txn, &receipt).unwrap();
        txn.commit().unwrap();
        let mut txn = store.begin().unwrap();
        assert!(
            plan_abort(&mut txn, migration.operation(), [42; 32]).is_err(),
            "committed evidence forbids aborting"
        );
    }

    #[test]
    fn abort_readback_refuses_row_and_migration_rebinding() {
        for defect in [
            "task",
            "state",
            "created",
            "truncated",
            "operation",
            "duplicate",
        ] {
            let store = super::super::tests::fixture();
            let migration = committed_migration(&store);
            let mut txn = store.begin().unwrap();
            let (planned, _) = plan_abort(&mut txn, migration.operation(), [42; 32]).unwrap();
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
                    ColumnValue::Bytes(planned.encode()[..100].to_vec()),
                ),
                "operation" => {
                    let mut bytes = planned.encode();
                    bytes[40] ^= 1;
                    (ColumnId(3), ColumnValue::Bytes(bytes))
                }
                "duplicate" => {
                    let task = txn.allocate_id(SequenceKind::Task).unwrap();
                    let mut copy = row.value.clone();
                    copy.set(ColumnId(1), ColumnValue::Uint(task));
                    txn.insert(&TASKS_DESC, &[memcmp_uint(task)], copy).unwrap();
                    txn.commit().unwrap();
                    assert!(committed_aborts(&store).is_err(), "{defect}");
                    continue;
                }
                _ => unreachable!(),
            };
            if defect == "task" {
                assert!(txn.update(&TASKS_DESC, &row.pk, vec![change]).is_err());
                continue;
            }
            txn.update(&TASKS_DESC, &row.pk, vec![change]).unwrap();
            txn.commit().unwrap();
            assert!(committed_aborts(&store).is_err(), "accepted {defect}");
        }
    }

    #[test]
    fn an_abort_unblocks_a_new_migration_for_the_same_region() {
        let store = super::super::tests::fixture();
        let migration = committed_migration(&store);
        register_node(&store, 5, [50; 16]);
        let mut txn = store.begin().unwrap();
        assert!(
            super::super::migration::plan_migration(
                &mut txn,
                [3; 16],
                migration.creation_task(),
                NodeId(5)
            )
            .is_err(),
            "an unsettled predecessor keeps the region blocked"
        );
        plan_abort(&mut txn, migration.operation(), [42; 32]).unwrap();
        txn.commit().unwrap();
        let mut txn = store.begin().unwrap();
        let (fresh, changed) = super::super::migration::plan_migration(
            &mut txn,
            [3; 16],
            migration.creation_task(),
            NodeId(5),
        )
        .unwrap();
        assert!(changed, "the aborted region is re-migratable");
        assert_eq!(fresh.region(), migration.region());
        assert_ne!(fresh.operation(), migration.operation());
        txn.commit().unwrap();
        // Readback stays whole with BOTH intents committed: one settled by
        // abort, one live. (A refusal here would freeze every reader — the
        // cascade-hang defect class.)
        let committed = super::super::migration::committed_migrations(&store).unwrap();
        assert_eq!(committed.len(), 2);
        assert_eq!(committed_aborts(&store).unwrap().len(), 1);
        // A THIRD migration while the second is live keeps refusing.
        register_node(&store, 6, [60; 16]);
        let mut txn = store.begin().unwrap();
        assert!(
            super::super::migration::plan_migration(
                &mut txn,
                [4; 16],
                migration.creation_task(),
                NodeId(6)
            )
            .is_err(),
            "a live successor keeps the region blocked"
        );
    }

    #[test]
    fn the_view_probe_matches_only_the_exact_owner_pair_derivation() {
        let store = super::super::tests::fixture();
        let migration = committed_migration(&store);
        let root = migration.root();
        let digest =
            super::super::evidence::migration_operation_digest(root, migration.operation());
        let view = store.engine().snapshot().unwrap();
        assert!(!abort_matches_owner_pair(view.as_ref(), root, digest, [42; 32]).unwrap());
        let mut txn = store.begin().unwrap();
        plan_abort(&mut txn, migration.operation(), [42; 32]).unwrap();
        txn.commit().unwrap();
        let view = store.engine().snapshot().unwrap();
        assert!(abort_matches_owner_pair(view.as_ref(), root, digest, [42; 32]).unwrap());
        assert!(!abort_matches_owner_pair(view.as_ref(), root, digest, [43; 32]).unwrap());
        assert!(!abort_matches_owner_pair(
            view.as_ref(),
            root,
            super::super::evidence::migration_operation_digest(root, [9; 16]),
            [42; 32]
        )
        .unwrap());
    }
}
