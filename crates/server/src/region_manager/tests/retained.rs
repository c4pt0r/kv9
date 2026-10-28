//! Deferred-open component fixture: real data Raft/WAL, local atomic metadata
//! ledger batches. Runtime adoption separately exercises routed metadata Raft.
use super::*;
use crate::checkpoint_retention::{drain_outbox::DrainOutbox, CheckpointLedger};
use crate::recovery_retention::RecoveryRetention;
use kv9_common::retention::OwnerId;
use kv9_engine::{MemoryObjectStore, ObjectKey, ObjectStore};
use kv9_meta::retention::{decode_request, encode_owner_observation, LedgerRequest, LocalLifetime};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Debug, Default)]
pub(super) struct Objects {
    inner: MemoryObjectStore,
    pub(super) gets: AtomicUsize,
}
impl ObjectStore for Objects {
    fn put(&self, k: &ObjectKey, bytes: &[u8]) -> Result<()> {
        self.inner.put(k, bytes)
    }
    fn get(&self, k: &ObjectKey) -> Result<Option<Vec<u8>>> {
        self.gets.fetch_add(1, Ordering::SeqCst);
        self.inner.get(k)
    }
    fn delete(&self, k: &ObjectKey) -> Result<()> {
        self.inner.delete(k)
    }
    fn list(&self, p: &str) -> Result<Vec<ObjectKey>> {
        self.inner.list(p)
    }
}

pub(super) struct Ledger {
    root: RootDescriptor,
    engine: Arc<WalEngine>,
    pub(super) outbox: Arc<DrainOutbox>,
    pub(super) lifetime: LocalLifetime,
    pub(super) refuse: AtomicBool,
    pub(super) finish_fault: AtomicBool,
    pub(super) reads: AtomicUsize,
}
impl CheckpointLedger for Ledger {
    fn apply(&self, bytes: Vec<u8>) -> Result<()> {
        let request = decode_request(&bytes, self.root.digest())?;
        if matches!(request, LedgerRequest::FinishCheckpointRead { .. })
            && self.finish_fault.load(Ordering::SeqCst)
        {
            return Err(Error::MetaNotReady("fixture finish unavailable".into()));
        }
        let plan = kv9_meta::retention::plan_retention(
            self.engine.snapshot()?.as_ref(),
            &self.root,
            &request,
        )?;
        self.engine.write(plan.into_batch())
    }
    fn owner(&self, root: RootDigest, id: OwnerId) -> Result<Option<Vec<u8>>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.refuse.load(Ordering::SeqCst) {
            return Err(Error::MetaNotReady("fixture quorum unavailable".into()));
        }
        assert_eq!(root, self.root.digest());
        kv9_meta::retention::retention_owner(self.engine.snapshot()?.as_ref(), &self.root, id)?
            .as_ref()
            .map(encode_owner_observation)
            .transpose()
    }
    fn local_lifetime(&self) -> Result<LocalLifetime> {
        Ok(self.lifetime)
    }
    fn drain_outbox(&self) -> Result<Option<Arc<DrainOutbox>>> {
        Ok(Some(self.outbox.clone()))
    }
}
impl Ledger {
    pub(super) fn new(f: &Fixture, path: &Path, identity: StoreIdentity) -> Arc<Self> {
        let root = kv9_meta::root::certified_root(&f.store.begin().unwrap())
            .unwrap()
            .unwrap();
        Arc::new(Self {
            root,
            engine: f.store.engine().clone(),
            outbox: DrainOutbox::open(path, identity).unwrap(),
            lifetime: LocalLifetime {
                node: identity.node_id.0,
                store: *identity.store_incarnation.as_bytes(),
                process: [64; 16],
            },
            refuse: AtomicBool::new(false),
            finish_fault: AtomicBool::new(false),
            reads: AtomicUsize::new(0),
        })
    }
    pub(super) fn retention(self: &Arc<Self>, identity: StoreIdentity) -> Arc<RecoveryRetention> {
        Arc::new(RecoveryRetention::new(
            self.root.clone(),
            identity,
            self.clone(),
        ))
    }
}

pub(super) fn initialize_ledger_identity(f: &Fixture) {
    use kv9_common::{KeyspaceId, META_REGION_0};
    use kv9_meta::schema::{REGIONS_DESC, SCHEMA_VERSION, SCHEMA_VERSION_DESC};
    let mut txn = f.store.begin().unwrap();
    let root = kv9_meta::root::certified_root(&txn).unwrap().unwrap();
    kv9_meta::admission::initialize_cluster(&mut txn, root.cluster_id, 1).unwrap();
    let mut version = RowValue::new();
    version.set(ColumnId(2), ColumnValue::Uint(SCHEMA_VERSION as u64));
    txn.insert(&SCHEMA_VERSION_DESC, &[memcmp_uint(0)], version)
        .unwrap();
    let mut row = RowValue::new();
    for (id, value) in [
        (1, META_REGION_0.0),
        (2, KeyspaceId::SYSTEM.0 as u64),
        (5, 1),
        (6, 1),
        (7, 1),
    ] {
        row.set(ColumnId(id), ColumnValue::Uint(value));
    }
    row.set(ColumnId(3), ColumnValue::Bytes(Vec::new()));
    row.set(ColumnId(4), ColumnValue::Bytes(Vec::new()));
    // These four identity records are the ledger planner's component premise;
    // this fixture is not a fully initialized serving metadata catalog.
    let mut batch = txn.into_batch();
    batch.put(
        ColumnFamily::Default,
        kv9_meta::codec::encode_row_key(REGIONS_DESC.id, &[memcmp_uint(META_REGION_0.0)]).unwrap(),
        row.encode(),
    );
    f.store.engine().write(batch).unwrap();
}

pub(super) fn extra_active(
    f: &Fixture,
    path: &Path,
    identity: StoreIdentity,
    creation: &CommittedCreation,
) {
    let mut manager = RegionManager::new(path, identity, 1);
    manager.prepare(creation).unwrap();
    publish(
        &manager
            .directory
            .join(creation.intent().region().0.to_string()),
        &Record {
            phase: Phase::Active,
            node: identity.node_id,
            incarnation: identity.store_incarnation,
            intent: creation.intent().clone(),
        },
        &mut |_, _| Ok(()),
    )
    .unwrap();
    assert_eq!(f.identity.root_digest, identity.root_digest);
}

pub(super) fn extra_creations(f: &Fixture) -> [CommittedCreation; 2] {
    let mut txn = f.store.begin().unwrap();
    let first = kv9_meta::data_groups::plan_empty_group(
        &mut txn,
        [80; 16],
        &[NodeId(1), NodeId(2), NodeId(3)],
    )
    .unwrap();
    let second = kv9_meta::data_groups::plan_empty_group(
        &mut txn,
        [81; 16],
        &[NodeId(1), NodeId(2), NodeId(3)],
    )
    .unwrap();
    txn.commit().unwrap();
    [first, second].map(|i| {
        kv9_meta::data_groups::committed_creation(&f.store, i.task())
            .unwrap()
            .unwrap()
    })
}

#[test]
fn migration_reconciliation_rotates_past_missing_destinations_and_terminal_groups() {
    let f = Fixture::new();
    let extras = extra_creations(&f);
    let mut txn = f.store.begin().unwrap();
    let mut row = RowValue::new();
    row.set(ColumnId(2), ColumnValue::Text("127.0.0.1:19444".into()));
    row.set(ColumnId(3), ColumnValue::Uint(2));
    row.set(ColumnId(4), ColumnValue::Uint(0));
    row.set(ColumnId(5), ColumnValue::Bytes(vec![4; 16]));
    txn.insert(&NODES_DESC, &[memcmp_uint(4)], row).unwrap();
    for (i, creation) in [&f.creation, &extras[0], &extras[1]].iter().enumerate() {
        kv9_meta::data_groups::activation::plan_activation(&mut txn, creation.intent()).unwrap();
        kv9_meta::data_groups::migration::plan_migration(
            &mut txn,
            [90 + i as u8; 16],
            creation.intent().task(),
            NodeId(4),
        )
        .unwrap();
    }
    txn.commit().unwrap();
    let candidates = kv9_meta::data_groups::migration::committed_migrations(&f.store).unwrap();
    let mut manager = f.manager();
    let regions = [
        f.creation.intent().region(),
        extras[0].intent().region(),
        extras[1].intent().region(),
    ];
    manager
        .groups
        .insert(regions[0], LocalGroup::Failed("not installed".into()));
    for expected in regions.into_iter().cycle().take(6) {
        assert_eq!(
            manager
                .next_adoption(candidates.clone().into_iter())
                .unwrap()
                .intent()
                .region(),
            expected
        );
    }
    manager
        .groups
        .insert(regions[0], LocalGroup::Retired { draining: None });
    manager.groups.insert(regions[1], LocalGroup::Reclaimed {});
    for _ in 0..2 {
        assert_eq!(
            manager
                .next_adoption(candidates.clone().into_iter())
                .unwrap()
                .intent()
                .region(),
            regions[2]
        );
    }
}

#[test]
fn failed_retirement_of_deferred_group_fences_retries_before_any_get() {
    let f = Fixture::new();
    extra_active(&f, &f.path, f.identity, &f.creation);
    let ledger = Ledger::new(&f, &f.path, f.identity);
    let mut manager = f.manager();
    manager.recovery_retention = Some(ledger.retention(f.identity));
    manager.recover(&f.store).unwrap();
    let region = f.creation.intent().region();
    assert!(matches!(manager.groups[&region], LocalGroup::Recovering(_)));
    let result = manager.retire_observed(region, &mut |step, after| {
        if step == PrepareStep::RetiredFileSync && !after {
            Err(Error::Engine("injected retirement failure".into()))
        } else {
            Ok(())
        }
    });
    assert!(result.is_err());
    manager.resume_recovery_one();
    assert!(matches!(manager.groups[&region], LocalGroup::Failed(_)));
    assert_eq!(ledger.reads.load(Ordering::SeqCst), 0);
    assert!(manager.group_handles.lock().unwrap().is_empty());
}
