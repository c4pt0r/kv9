//! Immutable, replicated creation intents for initially empty data groups.
//!
//! An intent reserves IDs and initial replica identities, but publishes no
//! routing row. Planning uses the existing serialized, term-fenced catalog
//! transaction path. Local preparation consumes a separate committed readback.

use kv9_common::{Error, NodeId, RegionId, Result, RootDigest, StoreIncarnation};
use kv9_engine::Engine;

use crate::codec::{memcmp_uint, ColumnValue, RowValue};
use crate::schema::{ColumnId, TASKS_DESC};
use crate::store::{MetaStore, MetaTxn, Row, SequenceKind, FIRST_DYNAMIC_ID};

pub mod abort;
pub mod activation;
pub mod addition;
pub mod capture;
pub mod compaction;
pub mod drain;
pub mod evidence;
pub mod merge;
pub mod migration;
pub mod ranges;
pub mod removal;
pub mod split;
pub mod split_evidence;
pub mod store_history;
pub mod truncation;

/// Stable task kind and encoding. This uses the existing tasks table, not a
/// new table/schema version. No other task may reuse this kind.
const CREATE_EMPTY_GROUP: u64 = 100;
const MAGIC: &[u8; 8] = b"KV9GRP01";
/// Transient row-page budget, independent of the amount of retained history.
/// Immutable history is required for old operation retries and recovery.
const TASK_PAGE_ROWS: usize = 128;
pub const MAX_INTENT_BYTES: usize = 8 + 32 + 16 + 8 + 8 + 1 + 7 * 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InitialReplica {
    pub node: NodeId,
    pub incarnation: StoreIncarnation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreationIntent {
    root: RootDigest,
    operation: [u8; 16],
    task: u64,
    region: RegionId,
    replicas: Vec<InitialReplica>,
}

impl CreationIntent {
    pub fn operation(&self) -> [u8; 16] {
        self.operation
    }
    pub fn root(&self) -> RootDigest {
        self.root
    }
    pub fn task(&self) -> u64 {
        self.task
    }
    pub fn region(&self) -> RegionId {
        self.region
    }
    pub fn replicas(&self) -> &[InitialReplica] {
        &self.replicas
    }
    pub fn digest(&self) -> RootDigest {
        RootDigest::sha256(&self.encode())
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(MAX_INTENT_BYTES);
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(self.root.as_bytes());
        bytes.extend_from_slice(&self.operation);
        bytes.extend_from_slice(&self.task.to_be_bytes());
        bytes.extend_from_slice(&self.region.0.to_be_bytes());
        bytes.push(self.replicas.len() as u8);
        for replica in &self.replicas {
            bytes.extend_from_slice(&replica.node.0.to_be_bytes());
            bytes.extend_from_slice(replica.incarnation.as_bytes());
        }
        bytes
    }

    /// Decode data only. This does not mint committed creation authority.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 73 || bytes.len() > MAX_INTENT_BYTES || &bytes[..8] != MAGIC {
            return Err(invalid("invalid group intent encoding"));
        }
        let count = usize::from(bytes[72]);
        if ![3, 5, 7].contains(&count) || bytes.len() != 73 + count * 24 {
            return Err(invalid("group intent needs 3, 5 or 7 exact replicas"));
        }
        let intent = Self {
            root: RootDigest::from_bytes(bytes[8..40].try_into().unwrap()),
            operation: bytes[40..56].try_into().unwrap(),
            task: u64::from_be_bytes(bytes[56..64].try_into().unwrap()),
            region: RegionId(u64::from_be_bytes(bytes[64..72].try_into().unwrap())),
            replicas: bytes[73..]
                .chunks_exact(24)
                .map(|chunk| InitialReplica {
                    node: NodeId(u64::from_be_bytes(chunk[..8].try_into().unwrap())),
                    incarnation: StoreIncarnation::from_bytes(chunk[8..].try_into().unwrap()),
                })
                .collect(),
        };
        if intent.root.as_bytes() == &[0; 32]
            || intent.operation == [0; 16]
            || intent.task < FIRST_DYNAMIC_ID
            || intent.region.0 < FIRST_DYNAMIC_ID
            || intent
                .replicas
                .iter()
                .any(|r| r.node.0 == 0 || r.incarnation.as_bytes() == &[0; 16])
            || intent.replicas.windows(2).any(|r| r[0].node >= r[1].node)
            || intent.replicas.iter().enumerate().any(|(i, r)| {
                intent.replicas[..i]
                    .iter()
                    .any(|other| other.incarnation == r.incarnation)
            })
        {
            return Err(invalid("invalid group intent identities"));
        }
        Ok(intent)
    }
}

/// Only `committed_creation` constructs this capability from an engine
/// snapshot; a planner's uncommitted overlay cannot construct it.
#[derive(Debug, Clone)]
pub struct CommittedCreation(CreationIntent);
impl CommittedCreation {
    pub fn intent(&self) -> &CreationIntent {
        &self.0
    }
}

fn invalid(message: &str) -> Error {
    Error::Config(message.into())
}

/// Walk all task history on one fixed transaction view. Only one decoded page
/// is buffered; callers retain their required protocol results, not a copy of
/// every task payload. No row is removed and IDs are never recycled here.
fn task_rows<'t, E: Engine>(txn: &'t MetaTxn<'_, E>) -> impl Iterator<Item = Result<Row>> + 't {
    let mut after = None::<Vec<crate::codec::PkComponent>>;
    let mut page = Vec::new().into_iter();
    let mut done = false;
    std::iter::from_fn(move || {
        if let Some(row) = page.next() {
            return Some(Ok(row));
        }
        if done {
            return None;
        }
        match txn.scan_after(&TASKS_DESC, after.as_deref(), TASK_PAGE_ROWS) {
            Err(error) => {
                done = true;
                Some(Err(error))
            }
            Ok(rows) => {
                done = rows.len() < TASK_PAGE_ROWS;
                after = rows.last().map(|row| row.pk.clone());
                page = rows.into_iter();
                page.next().map(Ok)
            }
        }
    })
}

fn from_row(row: &Row, root: RootDigest) -> Result<Option<CreationIntent>> {
    if row.value.get(ColumnId(2)) != Some(&ColumnValue::Uint(CREATE_EMPTY_GROUP)) {
        return Ok(None);
    }
    let Some(ColumnValue::Bytes(bytes)) = row.value.get(ColumnId(3)) else {
        return Err(invalid("group intent payload is missing"));
    };
    let intent = CreationIntent::decode(bytes)?;
    if intent.root != root
        || row.pk != [memcmp_uint(intent.task)]
        || row.value.get(ColumnId(1)) != Some(&ColumnValue::Uint(intent.task))
        || row.value.get(ColumnId(4)) != Some(&ColumnValue::Uint(0))
        || row.value.get(ColumnId(5)) != Some(&ColumnValue::Uint(0))
    {
        return Err(invalid("group intent row binding differs"));
    }
    Ok(Some(intent))
}

/// Stage one immutable intent, with operation-ID retry deduplication. The
/// caller holds the catalog planner lock through same-term committed apply.
/// A changed replica set or a replacement disk cannot reuse an operation ID.
pub fn plan_empty_group<E: Engine>(
    txn: &mut MetaTxn<'_, E>,
    operation: [u8; 16],
    voters: &[NodeId],
) -> Result<CreationIntent> {
    if operation == [0; 16] || ![3, 5, 7].contains(&voters.len()) {
        return Err(invalid(
            "group creation needs a nonzero operation and 3, 5 or 7 voters",
        ));
    }
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("group creation requires a certified metadata root"))?;
    let mut voters = voters.to_vec();
    voters.sort();
    if voters.windows(2).any(|v| v[0] == v[1]) {
        return Err(invalid("duplicate group voter"));
    }
    let replicas = voters
        .into_iter()
        .map(|node| {
            let endpoint = crate::endpoint::node_endpoint(txn, node)?
                .filter(|endpoint| endpoint.active)
                .ok_or_else(|| invalid("group voter is not an active registered store"))?;
            Ok(InitialReplica {
                node,
                incarnation: endpoint.incarnation,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut existing = None;
    for row in task_rows(txn) {
        let row = row?;
        if let Some(previous) = from_row(&row, root.digest())? {
            if previous.operation == operation {
                if previous.replicas != replicas {
                    return Err(invalid("group operation binding conflicts"));
                }
                if existing.replace(previous).is_some() {
                    return Err(invalid("duplicate group operation"));
                }
            }
        }
    }
    if let Some(previous) = existing {
        return Ok(previous);
    }
    // No partial allocation is exposed: caller commits or drops the entire txn.
    let intent = CreationIntent {
        root: root.digest(),
        operation,
        task: txn.allocate_id(SequenceKind::Task)?,
        region: RegionId(txn.allocate_id(SequenceKind::Region)?),
        replicas,
    };
    CreationIntent::decode(&intent.encode())?;
    let mut row = RowValue::new();
    for (column, value) in [
        (ColumnId(1), ColumnValue::Uint(intent.task)),
        (ColumnId(2), ColumnValue::Uint(CREATE_EMPTY_GROUP)),
        (ColumnId(3), ColumnValue::Bytes(intent.encode())),
        (ColumnId(4), ColumnValue::Uint(0)),
        (ColumnId(5), ColumnValue::Uint(0)),
    ] {
        row.set(column, value);
    }
    txn.insert(&TASKS_DESC, &[memcmp_uint(intent.task)], row)?;
    Ok(intent)
}

/// Whether a migration operation is **settled** for the purpose of admitting the
/// group's next one (docs/REPLICA-ADDITION.md §1): aborted, or evidenced and — when a
/// replica plan drives it as a move — decided for removal. One operation at a time
/// binds a group; a settled one leaves the group free for the next.
pub(crate) fn operation_settled<E: Engine>(
    txn: &MetaTxn<'_, E>,
    root: RootDigest,
    operation: [u8; 16],
) -> Result<bool> {
    let (mut aborted, mut evidenced, mut decided, mut move_plan) = (false, false, false, false);
    for row in task_rows(txn) {
        let row = row?;
        if abort::from_abort_row(&row, root)?.is_some_and(|a| a.operation() == operation) {
            aborted = true;
        }
        if evidence::from_row_for_siblings(&row, root)?.is_some_and(|e| e.operation() == operation) {
            evidenced = true;
        }
        if removal::from_removal_row(&row, root)?.is_some_and(|d| d.operation() == operation) {
            decided = true;
        }
        if addition::from_plan_row(&row, root)?
            .is_some_and(|p| p.operation() == operation && p.mode() == addition::PlanMode::Move)
        {
            move_plan = true;
        }
    }
    Ok(aborted || (evidenced && (!move_plan || decided)))
}

/// The group's committed membership: the creation's initial replicas, plus every
/// migration destination with committed install evidence, minus every replica a
/// committed removal decision retired. Each member carries the task that introduced
/// it (0 for an initial replica, the evidence task otherwise).
pub fn committed_members<E: Engine>(
    txn: &MetaTxn<'_, E>,
    root: RootDigest,
    creation: &CreationIntent,
) -> Result<Vec<(InitialReplica, u64)>> {
    let mut members: Vec<(InitialReplica, u64)> =
        creation.replicas().iter().map(|r| (*r, 0)).collect();
    let mut removed = Vec::new();
    for row in task_rows(txn) {
        let row = row?;
        if let Some(e) = evidence::from_row_for_siblings(&row, root)? {
            if e.region() == creation.region() {
                members.push((e.destination(), e.task()));
            }
        }
        if let Some(d) = removal::from_removal_row(&row, root)? {
            if d.region() == creation.region() {
                removed.push(d.source());
            }
        }
    }
    members.retain(|(replica, _)| !removed.contains(replica));
    Ok(members)
}

/// Read only from the local applied metadata engine, never a caller's overlay.
/// Intents are immutable and have no cancel/reuse transition, so local committed
/// readback need not establish current leadership to prepare its exact disk.
/// Production metadata engine mutation remains exclusively Raft-ordered.
pub fn committed_creation<E: Engine>(
    store: &MetaStore<E>,
    task: u64,
) -> Result<Option<CommittedCreation>> {
    let txn = store.begin()?;
    let root = crate::root::certified_root(&txn)?
        .ok_or_else(|| invalid("group readback requires a certified metadata root"))?;
    txn.get(&TASKS_DESC, &[memcmp_uint(task)])?
        .as_ref()
        .map(|row| from_row(row, root.digest()))
        .transpose()
        .map(|intent| intent.flatten().map(CommittedCreation))
}

/// Bounded replay of immutable intents, including a preparation interrupted
/// before its first local record was published. No local file can mint an intent.
pub fn committed_creations<E: Engine>(store: &MetaStore<E>) -> Result<Vec<CommittedCreation>> {
    let txn = store.begin()?;
    let root = crate::root::certified_root(&txn)?
        .ok_or_else(|| invalid("group readback requires a certified metadata root"))?;
    let mut intents = Vec::new();
    for row in task_rows(&txn) {
        let row = row?;
        if let Some(intent) = from_row(&row, root.digest())? {
            if intents.iter().any(|previous: &CommittedCreation| {
                previous.intent().region() == intent.region()
                    || previous.intent().operation == intent.operation
            }) {
                return Err(invalid("duplicate committed group creation identity"));
            }
            intents.push(CommittedCreation(intent));
        }
    }
    Ok(intents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{NODES_DESC, REGIONS_DESC};
    use kv9_common::{BootstrapGeneration, ClusterId, RootDescriptor, RootVoter};
    use kv9_engine::MemEngine;
    use std::sync::Arc;

    /// Retain unrelated task kinds before the authority under test. This makes
    /// the protocol tests exercise later pages without changing their group set.
    pub(super) fn fill_task_history(store: &MetaStore<MemEngine>, count: usize) {
        let mut txn = store.begin().unwrap();
        for _ in 0..count {
            let id = txn.allocate_id(SequenceKind::Task).unwrap();
            let mut row = RowValue::new();
            row.set(ColumnId(1), ColumnValue::Uint(id));
            row.set(ColumnId(2), ColumnValue::Uint(999));
            row.set(ColumnId(3), ColumnValue::Bytes(vec![]));
            row.set(ColumnId(4), ColumnValue::Uint(1));
            row.set(ColumnId(5), ColumnValue::Uint(0));
            txn.insert(&TASKS_DESC, &[memcmp_uint(id)], row).unwrap();
        }
        txn.commit().unwrap();
    }

    #[test]
    fn task_history_surfaces_late_corruption_and_fuses_after_error() {
        use kv9_engine::{ColumnFamily, WriteBatch};
        let store = fixture();
        let mut txn = store.begin().unwrap();
        let first =
            plan_empty_group(&mut txn, [8; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        txn.commit().unwrap();
        fill_task_history(&store, TASK_PAGE_ROWS * 2 - 1);
        let mut txn = store.begin().unwrap();
        let intent =
            plan_empty_group(&mut txn, [9; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        txn.commit().unwrap();
        let mut batch = WriteBatch::new();
        batch.put(
            ColumnFamily::Default,
            crate::codec::encode_row_key(TASKS_DESC.id, &[memcmp_uint(intent.task())]).unwrap(),
            vec![255],
        );
        store.engine().write(batch).unwrap();
        let txn = store.begin().unwrap();
        let mut rows = task_rows(&txn);
        for _ in 0..TASK_PAGE_ROWS * 2 {
            assert!(rows.next().unwrap().is_ok());
        }
        assert!(rows.next().unwrap().is_err());
        assert!(rows.next().is_none());
        assert!(rows.next().is_none());
        assert!(committed_creations(&store).is_err());
        assert!(activation::committed_activations(&store).is_err());
        let mut retry = store.begin().unwrap();
        assert!(plan_empty_group(
            &mut retry,
            first.operation(),
            &[NodeId(1), NodeId(2), NodeId(3)]
        )
        .is_err());
        assert!(retry.into_batch().is_empty());
    }

    #[test]
    fn task_history_detects_duplicate_operations_across_pages() {
        let store = fixture();
        let mut txn = store.begin().unwrap();
        let first =
            plan_empty_group(&mut txn, [8; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        txn.commit().unwrap();
        fill_task_history(&store, TASK_PAGE_ROWS * 2);
        let mut txn = store.begin().unwrap();
        let mut duplicate =
            plan_empty_group(&mut txn, [9; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        duplicate.operation = first.operation;
        txn.update(
            &TASKS_DESC,
            &[memcmp_uint(duplicate.task())],
            vec![(ColumnId(3), ColumnValue::Bytes(duplicate.encode()))],
        )
        .unwrap();
        txn.commit().unwrap();
        assert!(committed_creations(&store).is_err());
        assert!(plan_empty_group(
            &mut store.begin().unwrap(),
            first.operation(),
            &[NodeId(1), NodeId(2), NodeId(3)]
        )
        .is_err());
    }

    #[test]
    fn task_history_preserves_creation_and_activation_beyond_255_rows() {
        let store = fixture();
        let mut expected = Vec::new();
        for sequence in 1u64..=270 {
            let mut operation = [31; 16];
            operation[..8].copy_from_slice(&sequence.to_be_bytes());
            let mut txn = store.begin().unwrap();
            let intent =
                plan_empty_group(&mut txn, operation, &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
            assert!(activation::plan_activation(&mut txn, &intent).unwrap());
            txn.commit().unwrap();
            expected.push(intent);
        }
        assert_eq!(committed_creations(&store).unwrap().len(), expected.len());
        assert_eq!(
            activation::committed_activations(&store).unwrap().len(),
            expected.len()
        );
        for original in &expected {
            let mut txn = store.begin().unwrap();
            assert_eq!(
                plan_empty_group(
                    &mut txn,
                    original.operation(),
                    &[NodeId(3), NodeId(1), NodeId(2)]
                )
                .unwrap(),
                *original
            );
            assert!(!activation::plan_activation(&mut txn, original).unwrap());
            assert!(
                txn.into_batch().is_empty(),
                "a retry allocated new catalog state"
            );
            assert_eq!(
                committed_creation(&store, original.task())
                    .unwrap()
                    .unwrap()
                    .intent(),
                original
            );
        }
        assert_eq!(
            store
                .begin()
                .unwrap()
                .scan(&TASKS_DESC, 1000)
                .unwrap()
                .len(),
            540
        );
    }

    #[test]
    fn task_history_repeated_compaction_keeps_old_receipts_and_newest_floor() {
        let store = fixture();
        let mut txn = store.begin().unwrap();
        let creation =
            plan_empty_group(&mut txn, [21; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        txn.commit().unwrap();
        let mut first = None;
        for index in 1..=300 {
            let mut txn = store.begin().unwrap();
            let (decision, fresh) = compaction::plan_compaction(
                &mut txn,
                creation.region(),
                kv9_common::AppliedPosition { term: 1, index },
            )
            .unwrap();
            assert!(fresh);
            if index == 1 {
                first = Some(decision);
            }
            txn.commit().unwrap();
        }
        let floors = compaction::committed_compaction_floors(&store).unwrap();
        assert_eq!(floors.len(), 1);
        assert_eq!(floors[0].floor().index, 300);
        let mut txn = store.begin().unwrap();
        let (old, fresh) = compaction::plan_compaction(
            &mut txn,
            creation.region(),
            kv9_common::AppliedPosition { term: 1, index: 1 },
        )
        .unwrap();
        assert!(!fresh);
        assert_eq!(Some(old), first);
        assert!(txn.into_batch().is_empty());
        assert_eq!(
            store
                .begin()
                .unwrap()
                .scan(&TASKS_DESC, 1000)
                .unwrap()
                .len(),
            301
        );
    }

    pub(super) fn fixture() -> MetaStore<MemEngine> {
        let store = MetaStore::new(Arc::new(MemEngine::new()));
        initialize_fixture(&store);
        store
    }

    fn initialize_fixture<E: Engine>(store: &MetaStore<E>) {
        let voters: Vec<_> = (1..=3)
            .map(|id| RootVoter {
                node_id: NodeId(id),
                addr: format!("127.0.0.1:{}", 18000 + id).parse().unwrap(),
                store_incarnation: StoreIncarnation::from_bytes([id as u8; 16]),
            })
            .collect();
        let root = RootDescriptor::new(
            ClusterId::from_bytes([11; 16]),
            BootstrapGeneration::from_bytes([12; 16]),
            voters.clone(),
            b"group-test",
        )
        .unwrap();
        let mut txn = store.begin().unwrap();
        crate::root::initialize_root(&mut txn, &root).unwrap();
        for voter in voters {
            let mut row = RowValue::new();
            row.set(ColumnId(2), ColumnValue::Text(voter.addr.to_string()));
            row.set(ColumnId(3), ColumnValue::Uint(2));
            row.set(ColumnId(4), ColumnValue::Uint(0));
            row.set(
                ColumnId(5),
                ColumnValue::Bytes(voter.store_incarnation.as_bytes().to_vec()),
            );
            txn.insert(&NODES_DESC, &[memcmp_uint(voter.node_id.0)], row)
                .unwrap();
        }
        txn.commit().unwrap();
    }

    #[test]
    fn task_history_wal_reopen_preserves_old_retries_and_allocation() {
        use kv9_engine::WalEngine;
        let dir = std::env::temp_dir().join(format!(
            "kv9-task-history-{}",
            StoreIncarnation::mint().unwrap()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("metadata.wal");
        let store = MetaStore::new(Arc::new(WalEngine::open(&path).unwrap().0));
        initialize_fixture(&store);
        let mut original = Vec::new();
        for sequence in 1u64..=130 {
            let mut operation = [71; 16];
            operation[..8].copy_from_slice(&sequence.to_be_bytes());
            let mut txn = store.begin().unwrap();
            let intent =
                plan_empty_group(&mut txn, operation, &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
            activation::plan_activation(&mut txn, &intent).unwrap();
            txn.commit().unwrap();
            original.push(intent);
        }
        drop(store);
        let recovered = MetaStore::new(Arc::new(WalEngine::open(&path).unwrap().0));
        assert_eq!(committed_creations(&recovered).unwrap().len(), 130);
        assert_eq!(
            activation::committed_activations(&recovered).unwrap().len(),
            130
        );
        for intent in &original {
            let mut txn = recovered.begin().unwrap();
            assert_eq!(
                plan_empty_group(
                    &mut txn,
                    intent.operation(),
                    &[NodeId(1), NodeId(2), NodeId(3)]
                )
                .unwrap(),
                *intent
            );
            assert!(!activation::plan_activation(&mut txn, intent).unwrap());
            assert!(txn.into_batch().is_empty());
        }
        let mut txn = recovered.begin().unwrap();
        let next =
            plan_empty_group(&mut txn, [72; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        assert!(next.task() > original.last().unwrap().task());
        assert_eq!(next.region().0, original.last().unwrap().region().0 + 1);
        txn.commit().unwrap();
        drop(recovered);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn group_preparation_requires_committed_readback_and_deduplicates_operation() {
        let store = fixture();
        let mut txn = store.begin().unwrap();
        let planned =
            plan_empty_group(&mut txn, [9; 16], &[NodeId(3), NodeId(1), NodeId(2)]).unwrap();
        assert!(
            committed_creation(&store, planned.task())
                .unwrap()
                .is_none(),
            "an uncommitted overlay must not authorize local preparation"
        );
        assert!(
            txn.get(&REGIONS_DESC, &[memcmp_uint(planned.region().0)])
                .unwrap()
                .is_none(),
            "creation must not publish a routable region"
        );
        txn.commit().unwrap();
        let committed = committed_creation(&store, planned.task()).unwrap().unwrap();
        assert_eq!(committed.intent(), &planned);
        let mut retry = store.begin().unwrap();
        assert_eq!(
            plan_empty_group(&mut retry, [9; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap(),
            planned,
            "same operation must reserve the same group"
        );
        retry.commit().unwrap();
        assert_eq!(committed_creations(&store).unwrap().len(), 1);
        let mut next = store.begin().unwrap();
        let next =
            plan_empty_group(&mut next, [10; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        assert_eq!(
            next.task(),
            planned.task() + 1,
            "retry must not consume a task ID"
        );
        assert_eq!(
            next.region().0,
            planned.region().0 + 1,
            "retry must not consume a region ID"
        );
    }

    #[test]
    fn group_preparation_rejects_changed_incarnation_and_invalid_members() {
        let store = fixture();
        let mut txn = store.begin().unwrap();
        plan_empty_group(&mut txn, [9; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        txn.commit().unwrap();
        for voters in [
            vec![NodeId(1)],
            vec![NodeId(1), NodeId(1), NodeId(2)],
            vec![NodeId(1), NodeId(2), NodeId(4)],
        ] {
            assert!(plan_empty_group(&mut store.begin().unwrap(), [10; 16], &voters).is_err());
        }
        let mut change = store.begin().unwrap();
        change
            .update(
                &NODES_DESC,
                &[memcmp_uint(3)],
                vec![(ColumnId(5), ColumnValue::Bytes(vec![8; 16]))],
            )
            .unwrap();
        change.commit().unwrap();
        assert!(
            matches!(plan_empty_group(&mut store.begin().unwrap(), [9;16], &[NodeId(1),NodeId(2),NodeId(3)]), Err(Error::Config(s)) if s == "group operation binding conflicts"),
            "an operation must not silently authorize a replacement disk"
        );
    }

    #[test]
    fn group_preparation_codec_refuses_truncation_and_row_rebinding() {
        let store = fixture();
        let mut txn = store.begin().unwrap();
        let intent =
            plan_empty_group(&mut txn, [9; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        let bytes = intent.encode();
        assert_eq!(CreationIntent::decode(&bytes).unwrap(), intent);
        for n in 0..bytes.len() {
            assert!(
                CreationIntent::decode(&bytes[..n]).is_err(),
                "truncated intent accepted at {n}"
            );
        }
        let mut extra = bytes.clone();
        extra.push(0);
        assert!(CreationIntent::decode(&extra).is_err());
        for offset in [0, 72] {
            let mut bad = bytes.clone();
            bad[offset] = 255;
            assert!(CreationIntent::decode(&bad).is_err());
        }
        txn.commit().unwrap();
        let mut change = store.begin().unwrap();
        change
            .update(
                &TASKS_DESC,
                &[memcmp_uint(intent.task())],
                vec![(ColumnId(4), ColumnValue::Uint(1))],
            )
            .unwrap();
        change.commit().unwrap();
        assert!(
            committed_creation(&store, intent.task()).is_err(),
            "unknown task phase must not grant creation authority"
        );
    }
}
