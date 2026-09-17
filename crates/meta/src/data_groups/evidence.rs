//! Committed destination-install evidence: the replicated receipt that one
//! described migration image was durably adopted at the destination's exact
//! store incarnation.
//!
//! Evidence never moves data, starts or promotes a peer, or touches the
//! retention ledger by itself. It is the committed fact the ledger's
//! migration quiesce fence consults: without a matching evidence row,
//! QuiesceAfterTransfer and Release keep refusing exactly as before. One
//! evidence row per operation; an exact resubmission is a confirmation
//! receipt; any divergence refuses.

use super::*;
use kv9_common::AppliedPosition;

const INSTALL_EVIDENCE: u64 = 104;
const EVD_MAGIC: &[u8; 8] = b"KV9EVD01";
pub const MAX_EVIDENCE_BYTES: usize = 8 + 32 + 16 + 8 + 8 + 8 + 8 + 16 + 16 + 32 + 32 + 8 + 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallEvidence {
    root: RootDigest,
    operation: [u8; 16],
    task: u64,
    migration_task: u64,
    region: RegionId,
    destination: InitialReplica,
    generation: StoreIncarnation,
    image_digest: RootDigest,
    subject: [u8; 32],
    cut: AppliedPosition,
}

impl InstallEvidence {
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
    pub fn generation(&self) -> StoreIncarnation {
        self.generation
    }
    pub fn image_digest(&self) -> RootDigest {
        self.image_digest
    }
    pub fn subject(&self) -> [u8; 32] {
        self.subject
    }
    pub fn cut(&self) -> AppliedPosition {
        self.cut
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(MAX_EVIDENCE_BYTES);
        bytes.extend_from_slice(EVD_MAGIC);
        bytes.extend_from_slice(self.root.as_bytes());
        bytes.extend_from_slice(&self.operation);
        bytes.extend_from_slice(&self.task.to_be_bytes());
        bytes.extend_from_slice(&self.migration_task.to_be_bytes());
        bytes.extend_from_slice(&self.region.0.to_be_bytes());
        bytes.extend_from_slice(&self.destination.node.0.to_be_bytes());
        bytes.extend_from_slice(self.destination.incarnation.as_bytes());
        bytes.extend_from_slice(self.generation.as_bytes());
        bytes.extend_from_slice(self.image_digest.as_bytes());
        bytes.extend_from_slice(&self.subject);
        bytes.extend_from_slice(&self.cut.term.to_be_bytes());
        bytes.extend_from_slice(&self.cut.index.to_be_bytes());
        bytes
    }

    fn decode_inner(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != MAX_EVIDENCE_BYTES || &bytes[..8] != EVD_MAGIC {
            return Err(invalid("invalid install evidence encoding"));
        }
        let evidence = Self {
            root: RootDigest::from_bytes(bytes[8..40].try_into().unwrap()),
            operation: bytes[40..56].try_into().unwrap(),
            task: u64::from_be_bytes(bytes[56..64].try_into().unwrap()),
            migration_task: u64::from_be_bytes(bytes[64..72].try_into().unwrap()),
            region: RegionId(u64::from_be_bytes(bytes[72..80].try_into().unwrap())),
            destination: InitialReplica {
                node: NodeId(u64::from_be_bytes(bytes[80..88].try_into().unwrap())),
                incarnation: StoreIncarnation::from_bytes(bytes[88..104].try_into().unwrap()),
            },
            generation: StoreIncarnation::from_bytes(bytes[104..120].try_into().unwrap()),
            image_digest: RootDigest::from_bytes(bytes[120..152].try_into().unwrap()),
            subject: bytes[152..184].try_into().unwrap(),
            cut: AppliedPosition {
                term: u64::from_be_bytes(bytes[184..192].try_into().unwrap()),
                index: u64::from_be_bytes(bytes[192..200].try_into().unwrap()),
            },
        };
        if evidence.root.as_bytes() == &[0; 32]
            || evidence.operation == [0; 16]
            || evidence.migration_task < FIRST_DYNAMIC_ID
            || evidence.region.0 < FIRST_DYNAMIC_ID
            || evidence.destination.node.0 == 0
            || evidence.destination.incarnation.as_bytes() == &[0; 16]
            || evidence.generation.as_bytes() == &[0; 16]
            || evidence.image_digest.as_bytes() == &[0; 32]
            || evidence.subject == [0; 32]
            || evidence.cut.term == 0
            || evidence.cut.index == 0
        {
            return Err(invalid("invalid install evidence identities"));
        }
        Ok(evidence)
    }

    /// Decode one committed row payload. This does not mint evidence authority.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let evidence = Self::decode_inner(bytes)?;
        if evidence.task < FIRST_DYNAMIC_ID || evidence.task == evidence.migration_task {
            return Err(invalid("invalid install evidence identities"));
        }
        Ok(evidence)
    }

    /// Decode a destination-emitted receipt: identical layout with the task
    /// still zero, because the catalog allocates it at planning time.
    pub fn decode_receipt(bytes: &[u8]) -> Result<Self> {
        let receipt = Self::decode_inner(bytes)?;
        if receipt.task != 0 {
            return Err(invalid("an evidence receipt cannot supply its own task"));
        }
        Ok(receipt)
    }

    /// Canonical receipt bytes emitted by the destination from its own durable
    /// adopted state plus the committed migration it reconciled against.
    #[allow(clippy::too_many_arguments)]
    pub fn receipt(
        migration: &super::migration::MigrationIntent,
        generation: StoreIncarnation,
        image_digest: RootDigest,
        subject: [u8; 32],
        cut: AppliedPosition,
    ) -> Result<Vec<u8>> {
        let receipt = Self {
            root: migration.root(),
            operation: migration.operation(),
            task: 0,
            migration_task: migration.task(),
            region: migration.region(),
            destination: migration.destination(),
            generation,
            image_digest,
            subject,
            cut,
        };
        let bytes = receipt.encode();
        Self::decode_receipt(&bytes)?;
        Ok(bytes)
    }
}

fn from_evidence_row(row: &Row, root: RootDigest) -> Result<Option<InstallEvidence>> {
    if row.value.get(ColumnId(2)) != Some(&ColumnValue::Uint(INSTALL_EVIDENCE)) {
        return Ok(None);
    }
    let Some(ColumnValue::Bytes(bytes)) = row.value.get(ColumnId(3)) else {
        return Err(invalid("install evidence payload is missing"));
    };
    let evidence = InstallEvidence::decode(bytes)?;
    if evidence.root != root
        || row.pk != [memcmp_uint(evidence.task)]
        || row.value.get(ColumnId(1)) != Some(&ColumnValue::Uint(evidence.task))
        || row.value.get(ColumnId(4)) != Some(&ColumnValue::Uint(0))
        || row.value.get(ColumnId(5)) != Some(&ColumnValue::Uint(0))
    {
        return Err(invalid("install evidence row binding differs"));
    }
    Ok(Some(evidence))
}

fn matches_migration(
    evidence: &InstallEvidence,
    migration: &super::migration::MigrationIntent,
) -> Result<()> {
    if migration.task() != evidence.migration_task
        || migration.operation() != evidence.operation
        || migration.root() != evidence.root
        || migration.region() != evidence.region
        || migration.destination() != evidence.destination
    {
        return Err(invalid("evidence differs from the committed migration"));
    }
    Ok(())
}

/// Stage one idempotent evidence row in the serialized, term-fenced catalog
/// transaction. `false` means the exact evidence already exists and this
/// commit is a confirmation receipt. The committed migration row is the
/// authority the receipt must match; nothing here quiesces or releases a pin.
pub fn plan_install_evidence<E: Engine>(
    txn: &mut MetaTxn<'_, E>,
    receipt: &[u8],
) -> Result<(InstallEvidence, bool)> {
    let receipt = InstallEvidence::decode_receipt(receipt)?;
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("evidence requires a certified metadata root"))?;
    if receipt.root != root.digest() {
        return Err(invalid("evidence root differs from the certified root"));
    }
    let rows = txn.scan(&TASKS_DESC, MAX_CREATION_TASKS + 1)?;
    if rows.len() > MAX_CREATION_TASKS {
        return Err(invalid("bounded evidence task scan exceeded"));
    }
    let migration = super::migration::committed_migrations_in_txn(txn, &rows, root.digest())?
        .into_iter()
        .find(|m| m.operation() == receipt.operation)
        .ok_or_else(|| invalid("evidence requires the committed migration"))?;
    matches_migration(&receipt, &migration)?;
    let mut existing = None;
    for row in &rows {
        if let Some(previous) = from_evidence_row(row, root.digest())? {
            if previous.operation == receipt.operation {
                let expected = InstallEvidence {
                    task: previous.task,
                    ..receipt.clone()
                };
                if expected != previous {
                    return Err(invalid("evidence operation binding conflicts"));
                }
                existing = Some(previous);
            } else if previous.region == receipt.region {
                return Err(invalid("a committed evidence row already binds this group"));
            }
        }
    }
    if let Some(previous) = existing {
        return Ok((previous, false));
    }
    if rows.len() == MAX_CREATION_TASKS {
        return Err(invalid("evidence task capacity reached"));
    }
    let evidence = InstallEvidence {
        task: txn.allocate_id(SequenceKind::Task)?,
        ..receipt
    };
    InstallEvidence::decode(&evidence.encode())?;
    let mut row = RowValue::new();
    for (column, value) in [
        (ColumnId(1), ColumnValue::Uint(evidence.task)),
        (ColumnId(2), ColumnValue::Uint(INSTALL_EVIDENCE)),
        (ColumnId(3), ColumnValue::Bytes(evidence.encode())),
        (ColumnId(4), ColumnValue::Uint(0)),
        (ColumnId(5), ColumnValue::Uint(0)),
    ] {
        row.set(column, value);
    }
    txn.insert(&TASKS_DESC, &[memcmp_uint(evidence.task)], row)?;
    Ok((evidence, true))
}

/// Read every committed evidence row from one fresh applied snapshot with the
/// exact migration rows re-validated. Evidence is immutable and never expires.
pub fn committed_install_evidence<E: Engine>(store: &MetaStore<E>) -> Result<Vec<InstallEvidence>> {
    let txn = store.begin()?;
    let root = crate::root::certified_root(&txn)?
        .ok_or_else(|| invalid("evidence readback requires a certified metadata root"))?;
    let rows = txn.scan(&TASKS_DESC, MAX_CREATION_TASKS + 1)?;
    if rows.len() > MAX_CREATION_TASKS {
        return Err(invalid("bounded evidence task scan exceeded"));
    }
    let migrations = super::migration::committed_migrations_in_txn(&txn, &rows, root.digest())?;
    let mut out: Vec<InstallEvidence> = Vec::new();
    for row in &rows {
        if let Some(evidence) = from_evidence_row(row, root.digest())? {
            let migration = migrations
                .iter()
                .find(|m| m.operation() == evidence.operation)
                .ok_or_else(|| invalid("evidence requires the committed migration"))?;
            matches_migration(&evidence, migration)?;
            if out.iter().any(|prior| {
                prior.task == evidence.task
                    || prior.operation == evidence.operation
                    || prior.region == evidence.region
            }) {
                return Err(invalid("duplicate evidence identity"));
            }
            out.push(evidence);
        }
    }
    Ok(out)
}

/// The retention owner operation digest derived from one committed migration
/// operation. Server-side owner derivation and the ledger fence must agree on
/// this exact construction, so it lives here once.
pub fn migration_operation_digest(root: RootDigest, operation: [u8; 16]) -> [u8; 32] {
    let mut identity = b"kv9-migration-operation-v1".to_vec();
    identity.extend(root.as_bytes());
    identity.extend(operation);
    *RootDigest::sha256(&identity).as_bytes()
}

/// The ledger fence's view-level probe: does a committed evidence row exist
/// whose derived retention operation digest and subject match this owner
/// pair? This reads raw catalog rows from the SAME applied view the ledger
/// plans against — commitment is exactly local applied visibility, and
/// absence refuses in the safe direction. Row bindings are re-checked, but
/// the full migration cross-validation belongs to the catalog readers above;
/// a forged row cannot match, because the operation digest binds the root
/// and the committed operation identity that produced the owners.
pub fn evidence_matches_owner_pair(
    view: &dyn kv9_engine::ReadView,
    root: RootDigest,
    operation_digest: [u8; 32],
    subject: [u8; 32],
) -> Result<bool> {
    use kv9_engine::ColumnFamily;
    let (start, end) = crate::codec::row_range(TASKS_DESC.id)?;
    let entries = view.scan(ColumnFamily::Default, &start, &end, MAX_CREATION_TASKS + 1)?;
    if entries.len() > MAX_CREATION_TASKS {
        return Err(invalid("bounded evidence task scan exceeded"));
    }
    for (key, value) in entries {
        let row_value = RowValue::decode(&value)?;
        if row_value.get(ColumnId(2)) != Some(&ColumnValue::Uint(INSTALL_EVIDENCE)) {
            continue;
        }
        let Some(ColumnValue::Bytes(bytes)) = row_value.get(ColumnId(3)) else {
            return Err(invalid("install evidence payload is missing"));
        };
        let evidence = InstallEvidence::decode(bytes)?;
        if evidence.root != root
            || key != crate::codec::encode_row_key(TASKS_DESC.id, &[memcmp_uint(evidence.task)])?
            || row_value.get(ColumnId(1)) != Some(&ColumnValue::Uint(evidence.task))
        {
            return Err(invalid("install evidence row binding differs"));
        }
        if migration_operation_digest(root, evidence.operation) == operation_digest
            && evidence.subject == subject
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

    fn receipt_bytes(migration: &super::super::migration::MigrationIntent) -> Vec<u8> {
        InstallEvidence::receipt(
            migration,
            StoreIncarnation::from_bytes([41; 16]),
            RootDigest::sha256(b"image"),
            [42; 32],
            AppliedPosition { term: 3, index: 12 },
        )
        .unwrap()
    }

    #[test]
    fn evidence_requires_the_committed_migration_and_matches_it_exactly() {
        let store = super::super::tests::fixture();
        let migration = committed_migration(&store);
        let receipt = receipt_bytes(&migration);
        {
            // A receipt for an uncommitted operation refuses.
            let mut wrong = receipt.clone();
            wrong[40] ^= 1; // operation
            let mut txn = store.begin().unwrap();
            assert!(plan_install_evidence(&mut txn, &wrong).is_err());
            // Cross-bound fields must match the committed migration exactly.
            for at in [64, 72, 80, 88] {
                let mut wrong = receipt.clone();
                wrong[at] ^= 1; // migration task / region / node / incarnation
                assert!(plan_install_evidence(&mut txn, &wrong).is_err(), "{at}");
            }
            // A receipt cannot supply its own task id.
            let mut wrong = receipt.clone();
            wrong[63] = 200;
            assert!(plan_install_evidence(&mut txn, &wrong).is_err());
        }
        let mut txn = store.begin().unwrap();
        let (planned, changed) = plan_install_evidence(&mut txn, &receipt).unwrap();
        assert!(changed);
        assert_eq!(planned.operation(), migration.operation());
        assert_eq!(planned.migration_task(), migration.task());
        assert_eq!(planned.destination(), migration.destination());
        assert_eq!(planned.subject(), [42; 32]);
        assert_eq!(planned.cut(), AppliedPosition { term: 3, index: 12 });
        // The uncommitted overlay plans, but readback must stay empty.
        assert!(committed_install_evidence(&store).unwrap().is_empty());
        txn.commit().unwrap();
        let committed = committed_install_evidence(&store).unwrap();
        assert_eq!(committed, vec![planned.clone()]);
        // An exact resubmission is a confirmation; any divergence refuses.
        let mut txn = store.begin().unwrap();
        let (retry, changed) = plan_install_evidence(&mut txn, &receipt).unwrap();
        assert!(!changed);
        assert_eq!(retry, planned);
        let divergent = InstallEvidence::receipt(
            &migration,
            StoreIncarnation::from_bytes([41; 16]),
            RootDigest::sha256(b"image"),
            [43; 32],
            AppliedPosition { term: 3, index: 12 },
        )
        .unwrap();
        assert!(
            plan_install_evidence(&mut txn, &divergent).is_err(),
            "one operation must never name a second image"
        );
    }

    #[test]
    fn evidence_readback_refuses_row_and_migration_rebinding() {
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
            let (planned, _) = plan_install_evidence(&mut txn, &receipt_bytes(&migration)).unwrap();
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
                    assert!(committed_install_evidence(&store).is_err(), "{defect}");
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
            assert!(
                committed_install_evidence(&store).is_err(),
                "accepted {defect}"
            );
        }
    }

    #[test]
    fn the_view_probe_matches_only_the_exact_owner_pair_derivation() {
        let store = super::super::tests::fixture();
        let migration = committed_migration(&store);
        let root = migration.root();
        let digest = migration_operation_digest(root, migration.operation());
        let view = store.engine().snapshot().unwrap();
        assert!(!evidence_matches_owner_pair(view.as_ref(), root, digest, [42; 32]).unwrap());
        let mut txn = store.begin().unwrap();
        plan_install_evidence(&mut txn, &receipt_bytes(&migration)).unwrap();
        txn.commit().unwrap();
        let view = store.engine().snapshot().unwrap();
        assert!(evidence_matches_owner_pair(view.as_ref(), root, digest, [42; 32]).unwrap());
        assert!(!evidence_matches_owner_pair(view.as_ref(), root, digest, [43; 32]).unwrap());
        assert!(!evidence_matches_owner_pair(
            view.as_ref(),
            root,
            migration_operation_digest(root, [9; 16]),
            [42; 32]
        )
        .unwrap());
    }
}
