use super::*;
use crate::runtime::data_checkpoint::Maintenance;
use kv9_engine::checkpoint::{CheckpointManifest, FlushJournal, RemoteUploader};
use kv9_engine::{MemoryObjectStore, ObjectKey, ObjectStore};
use std::sync::{Condvar, Mutex};

mod admission;
mod bootstrap;
mod data_fence;
mod execution;
mod membership;
mod pending_disk;
mod read_floor;
mod readers;
mod selection;

async fn until(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("checkpoint progress deadline");
}
fn engine(f: &Fixture, i: usize) -> Arc<WalEngine> {
    f.backends[i]
        .raw_directory
        .by_region(f.range.region)
        .unwrap()
        .capture_parts()
        .1
        .clone()
}
fn selected(f: &Fixture, i: usize) -> Option<CheckpointManifest> {
    engine(f, i).selected_checkpoint().unwrap()
}
fn bytes(path: &std::path::Path) -> u64 {
    if path.is_file() {
        return path.metadata().unwrap().len();
    }
    std::fs::read_dir(path)
        .unwrap()
        .map(|e| bytes(&e.unwrap().path()))
        .sum()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn automatic_data_checkpoints_resume_lost_owner_receipt_and_adopt_on_all_three_replicas() {
    let f = Fixture::new().await;
    assert!(matches!(
        f.client(0).activate().await.outcome,
        TtlOutcome::Success { .. }
    ));
    for n in 0..36 {
        // Seeding: the same put again is harmless, and under a loaded test run one large
        // write can time out; retry it rather than fail on the setup.
        let mut last = None;
        for _ in 0..5 {
            let pairs = (0..8).map(|i| (vec![b'a' + i], vec![n; 65536])).collect();
            let outcome = f
                .client(0)
                .call(Operation::Put {
                    pairs,
                    ttl_secs: Some(60),
                })
                .await
                .outcome;
            if matches!(outcome, TtlOutcome::Success { .. }) {
                last = None;
                break;
            }
            last = Some(format!("{outcome:?}"));
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        assert!(last.is_none(), "seed put {n} failed: {last:?}");
    }
    let before: Vec<_> = (0..3)
        .map(|i| bytes(&engine(&f, i).path().with_extension("segments")))
        .collect();
    f.hooks[1].retention_fail_after.store(3, Ordering::SeqCst);
    let store = Arc::new(MemoryObjectStore::new());
    let uploader = Arc::new(RemoteUploader::for_harness(store.clone()));
    let tasks: Vec<_> = f
        .backends
        .iter()
        .map(|b| {
            Maintenance::start(
                b.clone(),
                uploader.clone(),
                true,
                Duration::from_millis(100),
                &tokio::runtime::Handle::current(),
            )
        })
        .collect();
    until(|| tasks[0].snapshot().failures > 0).await;
    let journal = FlushJournal::new(engine(&f, 0).path().with_extension("pending"));
    let pending = journal.load().unwrap().unwrap();
    let original = pending.manifest().clone();
    assert_eq!(pending.expected_generation(), 0);
    assert!(
        store.is_empty(),
        "unknown owner acquisition cannot reach object I/O"
    );
    // A renewal races the durable older candidate; the journal must not be
    // replaced with an easier, newer cut merely because registration failed.
    assert!(matches!(
        f.client(0)
            .call(Operation::Put {
                pairs: vec![(b"a".to_vec(), b"renewed".to_vec())],
                ttl_secs: Some(90)
            })
            .await
            .outcome,
        TtlOutcome::Success { .. }
    ));
    until(|| {
        f.data[0]
            .manifest_pair(f.range.region.0)
            .unwrap()
            .generation
            >= 1
    })
    .await;
    assert_eq!(
        f.data[0]
            .manifest_at(f.range.region.0, 1)
            .unwrap()
            .unwrap()
            .2,
        original.encode().unwrap()
    );
    until(|| (0..3).all(|i| selected(&f, i).is_some_and(|m| m.index > original.index))).await;
    let root = f.backends[0].ttl_context().unwrap().root.clone();
    until(|| {
        let view = f.backends[1]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap();
        (0..3).all(|i| {
            let identity = StoreIdentity::for_voter(&root, NodeId(i + 1)).unwrap();
            kv9_meta::retention::checkpoint_selection::selection(
                view.as_ref(),
                &root,
                identity.node_id,
                identity.store_incarnation,
                f.range.region,
            )
            .is_ok_and(|s| s.is_some())
        })
    })
    .await;
    for task in &tasks {
        task.cancel();
    }
    for task in tasks {
        task.drain().await;
    }
    let after: Vec<_> = (0..3)
        .map(|i| bytes(&engine(&f, i).path().with_extension("segments")))
        .collect();
    for i in 0..3 {
        assert!(
            after[i] < before[i] / 2,
            "replica {i}: {} -> {}",
            before[i],
            after[i]
        );
        let key =
            kv9_common::codec::encode_key(kv9_common::codec::KeyMode::Raw, f.range.keyspace, b"a")
                .unwrap();
        assert_eq!(
            engine(&f, i).get(ColumnFamily::Default, &key).unwrap(),
            Some(b"renewed".to_vec())
        );
        assert!(engine(&f, i)
            .get(ColumnFamily::Write, &key)
            .unwrap()
            .is_some());
    }
    eprintln!("automatic data checkpoint engine bytes: {before:?} -> {after:?}");
    f.close().await;
}

struct ExtraGroup {
    path: PathBuf,
    range: DataRange,
    drivers: Vec<Driver>,
    owners: Vec<std::thread::JoinHandle<()>>,
}
impl ExtraGroup {
    async fn new(f: &Fixture) -> Self {
        let backend = f.backends[1].clone();
        let root = backend.ttl_context().unwrap().root.digest();
        let range = tokio::task::spawn_blocking(move || {
            let mut txn = backend.node.meta_raft.store.begin().unwrap();
            let creation = kv9_meta::data_groups::plan_empty_group(
                &mut txn,
                [42; 16],
                &[NodeId(1), NodeId(2), NodeId(3)],
            )
            .unwrap();
            let (range, _) = kv9_meta::data_groups::ranges::plan_keyspace(
                &mut txn,
                root,
                creation.task(),
                "checkpoint-extra",
                TenantId::DEFAULT,
            )
            .unwrap();
            propose_and_wait(
                &backend.driver,
                &Command::from_batch(&txn.into_batch()),
                Duration::from_secs(3),
            )
            .unwrap();
            range
        })
        .await
        .unwrap();
        let path = f.path.join("extra");
        let hub = InProcHub::new();
        let mut drivers = Vec::new();
        for id in 1..=3 {
            let path = path.join(format!("n{id}"));
            std::fs::create_dir_all(&path).unwrap();
            let engine = Arc::new(WalEngine::open(path.join("data.wal")).unwrap().0);
            engine.enable_segmentation().unwrap();
            let storage = DiskRaftStorage::open(&path.join("raft"), &[1, 2, 3])
                .unwrap()
                .0;
            drivers.push(Self::driver(id, &range, engine.clone(), storage, &hub));
            f.backends[id as usize - 1]
                .raw_directory
                .insert(range_api::RawGroup::new(
                    range.clone(),
                    engine,
                    drivers.last().unwrap().clone(),
                    f.backends[id as usize - 1].raw_directory.clock(),
                ));
        }
        drivers[0].peer().campaign().unwrap();
        pump(&drivers);
        drivers[0]
            .propose(&Command::DataRange {
                expected: None,
                next: range.clone(),
            })
            .unwrap();
        pump(&drivers);
        let owners = drivers
            .iter()
            .map(|d| d.spawn(Duration::from_millis(5)).unwrap())
            .collect();
        Self {
            path,
            range,
            drivers,
            owners,
        }
    }
    fn wal(&self) -> PathBuf {
        self.path.join("n1/data.wal")
    }
    fn driver(
        id: u64,
        range: &DataRange,
        engine: Arc<WalEngine>,
        storage: DiskRaftStorage,
        hub: &Arc<InProcHub>,
    ) -> Driver {
        let peer = Arc::new(RaftPeer::with_storage(NodeId(id), range.region, storage).unwrap());
        let mut state = MemStateMachine::with_engine(engine).unwrap();
        state
            .set_data_group(range.root, range.region, range.creation)
            .unwrap();
        NodeDriver::new(
            peer,
            Arc::new(hub.endpoint(NodeId(id))) as Arc<dyn RaftTransport>,
            state,
        )
        .unwrap()
    }
    fn close_owner(&mut self, f: &Fixture) {
        for backend in &f.backends {
            backend.raw_directory.remove(self.range.region);
        }
        for driver in &self.drivers {
            driver.stop();
        }
        for owner in self.owners.drain(..) {
            owner.join().unwrap();
        }
        self.drivers.clear();
    }
    fn reopen(&mut self, f: &Fixture, uploader: &RemoteUploader) {
        let root = f.backends[0].ttl_context().unwrap().root.clone();
        let hub = InProcHub::new();
        for id in 1..=3 {
            let path = self.path.join(format!("n{id}"));
            let mut storage = DiskRaftStorage::recover(&path.join("raft")).unwrap();
            let (engine, _, _) = kv9_raft::state_machine::checkpoint_recovery::data_group::open(
                &mut storage,
                path.join("data.wal"),
                Some(uploader),
                kv9_raft::state_machine::checkpoint_recovery::data_group::DataCheckpointIdentity {
                    cluster: root.cluster_id,
                    root: root.digest(),
                    region: self.range.region,
                    creation: self.range.creation,
                },
            )
            .unwrap();
            let engine = Arc::new(engine);
            let driver = Self::driver(id, &self.range, engine.clone(), storage, &hub);
            f.backends[id as usize - 1]
                .raw_directory
                .insert(range_api::RawGroup::new(
                    self.range.clone(),
                    engine,
                    driver.clone(),
                    f.backends[id as usize - 1].raw_directory.clock(),
                ));
            self.drivers.push(driver);
        }
        self.drivers[0].peer().campaign().unwrap();
        pump(&self.drivers);
        self.owners = self
            .drivers
            .iter()
            .map(|d| d.spawn(Duration::from_millis(5)).unwrap())
            .collect();
    }
    fn client(&self, f: &Fixture) -> ScopedTtlClient {
        ScopedTtlClient::with_test_transport(
            self.range.clone(),
            TransportKind::TonicUnary,
            f.public[0].clone(),
            5000,
        )
    }
}
impl Drop for ExtraGroup {
    fn drop(&mut self) {
        for d in &self.drivers {
            d.stop();
        }
        for t in self.owners.drain(..) {
            let _ = t.join();
        }
    }
}
#[derive(Debug, Default)]
struct FailingStore {
    memory: MemoryObjectStore,
    fail: AtomicBool,
}
impl ObjectStore for FailingStore {
    fn put(&self, key: &ObjectKey, bytes: &[u8]) -> Result<()> {
        self.memory.put(key, bytes)?;
        if self.fail.load(Ordering::SeqCst) {
            return Err(Error::Engine("injected lost PUT response".into()));
        }
        Ok(())
    }
    fn get(&self, key: &ObjectKey) -> Result<Option<Vec<u8>>> {
        self.memory.get(key)
    }
    fn delete(&self, key: &ObjectKey) -> Result<()> {
        self.memory.delete(key)
    }
    fn list(&self, prefix: &str) -> Result<Vec<ObjectKey>> {
        self.memory.list(prefix)
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn automatic_pending_upload_restarts_same_disk_identity_even_when_new_triggers_disabled() {
    for handoff_failure in [false, true] {
        let f = Fixture::new().await;
        let mut single = ExtraGroup::new(&f).await;
        f.backends[0].raw_directory.remove(f.range.region);
        assert!(matches!(
            single.client(&f).activate().await.outcome,
            TtlOutcome::Success { .. }
        ));
        assert!(matches!(
            single
                .client(&f)
                .call(Operation::Put {
                    pairs: vec![(b"k".to_vec(), b"before crash".to_vec())],
                    ttl_secs: Some(60)
                })
                .await
                .outcome,
            TtlOutcome::Success { .. }
        ));
        let store = Arc::new(FailingStore::default());
        store.fail.store(!handoff_failure, Ordering::SeqCst);
        if handoff_failure {
            // Acquire (4), archive (1), executor (2), archive retry (1), then
            // the original post-manifest positive handoff Share loses its receipt.
            f.hooks[1].retention_fail_after.store(9, Ordering::SeqCst);
        }
        let uploader = Arc::new(RemoteUploader::for_harness(store.clone()));
        let task = Maintenance::start(
            f.backends[0].clone(),
            uploader.clone(),
            true,
            Duration::from_millis(100),
            &tokio::runtime::Handle::current(),
        );
        until(|| task.snapshot().failures > 0 && !store.memory.is_empty()).await;
        task.cancel();
        task.drain().await;
        let journal = FlushJournal::new(single.wal().with_extension("pending"));
        let pending = journal.load().unwrap().unwrap();
        let manifest = pending.manifest().clone();
        assert_eq!(pending.requires_upload(), !handoff_failure);
        assert_eq!(
            single.drivers[0]
                .manifest_pair(single.range.region.0)
                .unwrap()
                .generation,
            u64::from(handoff_failure)
        );
        single.close_owner(&f);
        single.reopen(&f, &uploader);
        store.fail.store(false, Ordering::SeqCst);
        let task = Maintenance::start(
            f.backends[0].clone(),
            uploader,
            false,
            Duration::from_millis(100),
            &tokio::runtime::Handle::current(),
        );
        until(|| {
            WalEngine::checkpoint_reference(single.wal())
                .unwrap()
                .is_some()
                && journal.load().unwrap().is_none()
        })
        .await;
        assert_eq!(
            single
                .drivers
                .first()
                .unwrap()
                .manifest_pair(single.range.region.0)
                .unwrap()
                .generation,
            1
        );
        assert_eq!(
            WalEngine::checkpoint_reference(single.wal()).unwrap(),
            Some(manifest)
        );
        let get = single
            .client(&f)
            .call(Operation::Get {
                keys: vec![b"k".to_vec()],
            })
            .await;
        assert_eq!(
            get.outcome,
            TtlOutcome::Success {
                reply: Reply::Values {
                    values: vec![Some(b"before crash".to_vec())]
                }
            }
        );
        // Time observation creates fresh data revision, but disabled new triggers
        // cannot start a second attempt once the recovered one completes.
        tokio::time::sleep(Duration::from_millis(350)).await;
        assert_eq!(
            single
                .drivers
                .first()
                .unwrap()
                .manifest_pair(single.range.region.0)
                .unwrap()
                .generation,
            1
        );
        task.cancel();
        task.drain().await;
        single.close_owner(&f);
        drop(single);
        f.close().await;
    }
}

#[derive(Debug)]
struct GatedStore {
    memory: MemoryObjectStore,
    region: u64,
    entered: AtomicBool,
    blocked_puts: std::sync::atomic::AtomicUsize,
    blocked: Mutex<bool>,
    wake: Condvar,
}
impl GatedStore {
    fn release(&self) {
        *self.blocked.lock().unwrap() = false;
        self.wake.notify_all();
    }
}
impl ObjectStore for GatedStore {
    fn put(&self, key: &ObjectKey, bytes: &[u8]) -> Result<()> {
        if self.region == 0 || key.as_str().contains(&format!("/regions/{}/", self.region)) {
            self.entered.store(true, Ordering::SeqCst);
            self.blocked_puts.fetch_add(1, Ordering::SeqCst);
            let mut blocked = self.blocked.lock().unwrap();
            while *blocked {
                blocked = self.wake.wait(blocked).unwrap();
            }
        }
        self.memory.put(key, bytes)
    }
    fn get(&self, key: &ObjectKey) -> Result<Option<Vec<u8>>> {
        self.memory.get(key)
    }
    fn delete(&self, key: &ObjectKey) -> Result<()> {
        self.memory.delete(key)
    }
    fn list(&self, prefix: &str) -> Result<Vec<ObjectKey>> {
        self.memory.list(prefix)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shared_node_budget_keeps_ttl_progress_with_two_blocked_uploads_and_drains_real_permits() {
    let f = Fixture::new().await;
    let mut extra = ExtraGroup::new(&f).await;
    assert!(matches!(
        f.client(0).activate().await.outcome,
        TtlOutcome::Success { .. }
    ));
    assert!(matches!(
        extra.client(&f).activate().await.outcome,
        TtlOutcome::Success { .. }
    ));
    for client in [f.client(0), extra.client(&f)] {
        assert!(matches!(
            client
                .call(Operation::Put {
                    pairs: vec![(b"k".to_vec(), b"expire".to_vec())],
                    ttl_secs: Some(0),
                })
                .await
                .outcome,
            TtlOutcome::Success { .. }
        ));
    }
    let store = Arc::new(GatedStore {
        memory: MemoryObjectStore::new(),
        region: 0,
        entered: AtomicBool::new(false),
        blocked_puts: std::sync::atomic::AtomicUsize::new(0),
        blocked: Mutex::new(true),
        wake: Condvar::new(),
    });
    let release = Release(store.clone());
    let uploader = Arc::new(RemoteUploader::for_harness(store.clone()));
    let backend = &f.backends[0];
    let budget = backend.maintenance.clone();
    let checkpoint = Maintenance::start(
        backend.clone(),
        uploader.clone(),
        true,
        Duration::from_millis(100),
        &tokio::runtime::Handle::current(),
    );
    until(|| store.blocked_puts.load(Ordering::SeqCst) == 2).await;
    let image_bytes = uploader.image_memory_snapshot().retained_bytes;
    assert!(image_bytes > 0);
    assert_eq!(uploader.image_memory_snapshot().active_images, 2);
    // A real metadata worker uses the SAME node budget. It cannot dispatch
    // while both checkpoint I/O slots are held, even though it has its own thread.
    let root = backend.ttl_context().unwrap().root.clone();
    let metadata_uploader = uploader.clone();
    let remote = crate::remote_storage::RemoteStorage::start(
        backend.node.clone(),
        backend.driver.clone(),
        crate::remote_storage::PreparedRemote::for_test(
            &f.path.join("n1"),
            &root,
            metadata_uploader,
        ),
        budget.clone(),
        Arc::new(crate::runtime::checkpoint_retention::RoutedLedger::new(
            backend,
            tokio::runtime::Handle::current(),
        )),
    )
    .unwrap();
    let ttl = crate::runtime::ttl_maintenance::Maintenance::start(
        backend.clone(),
        &tokio::runtime::Handle::current(),
    );
    until(|| ttl.snapshot().candidates_applied == 2).await;
    until(|| budget.snapshot().waiting_classes > 0).await;
    // The leader applied both TTL candidates; a follower may still be one entry behind
    // on a loaded run, so wait for every replica to drop the expired key.
    until(|| {
        [&f.range, &extra.range].into_iter().all(|range| {
            let key =
                kv9_common::codec::encode_key(kv9_common::codec::KeyMode::Raw, range.keyspace, b"k")
                    .unwrap();
            f.backends.iter().all(|b| {
                let group = b.raw_directory.by_region(range.region).unwrap();
                [ColumnFamily::Default, ColumnFamily::Write]
                    .into_iter()
                    .all(|cf| group.capture_parts().1.get(cf, &key).unwrap().is_none())
            })
        })
    })
    .await;
    assert_eq!(budget.snapshot().heavy_active, 2);
    assert!(budget.snapshot().peak <= 4);
    assert!(
        budget.snapshot().peak >= 3,
        "TTL ran beside blocked checkpoint I/O"
    );
    ttl.cancel();
    ttl.drain().await;
    // Dropping the waiting dispatcher removes its demand, not the data jobs.
    tokio::task::spawn_blocking(move || drop(remote))
        .await
        .unwrap();
    assert_eq!(budget.snapshot().active, 2);
    assert_eq!(budget.snapshot().waiting_classes, 0);
    checkpoint.cancel();
    let drain = tokio::spawn(checkpoint.drain());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!drain.is_finished());
    assert_eq!(uploader.image_memory_snapshot().retained_bytes, image_bytes);
    assert_eq!(uploader.image_memory_snapshot().active_images, 2);
    assert_eq!(budget.snapshot().active, 2);
    drop(release);
    tokio::time::timeout(Duration::from_secs(5), drain)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(budget.snapshot().active, 0);
    assert_eq!(uploader.image_memory_snapshot().retained_bytes, 0);
    assert_eq!(uploader.image_memory_snapshot().active_images, 0);
    assert_eq!(budget.snapshot().heavy_peak, 2);
    extra.close_owner(&f);
    drop(extra);
    f.close().await;
}
struct Release(Arc<GatedStore>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn metadata_worker_waits_for_shared_capacity_then_really_publishes_and_adopts() {
    let f = Fixture::new().await;
    let backend = &f.backends[1]; // The actual metadata leader is node 2.
    let budget = backend.maintenance.clone();
    let mut occupied = budget.lane(crate::maintenance::Class::DataCheckpoint);
    let a = occupied.try_acquire().unwrap();
    let b = occupied.try_acquire().unwrap();
    let root = backend.ttl_context().unwrap().root.clone();
    let objects = Arc::new(MemoryObjectStore::new());
    let uploader = Arc::new(RemoteUploader::for_harness(objects.clone()));
    let remote = crate::remote_storage::RemoteStorage::start(
        backend.node.clone(),
        backend.driver.clone(),
        crate::remote_storage::PreparedRemote::for_test(&f.path.join("n2"), &root, uploader),
        budget.clone(),
        Arc::new(crate::runtime::checkpoint_retention::RoutedLedger::new(
            backend,
            tokio::runtime::Handle::current(),
        )),
    )
    .unwrap();
    until(|| budget.snapshot().waiting_classes == 1).await;
    assert!(objects.is_empty());
    assert_eq!(
        backend
            .driver
            .manifest_pair(kv9_common::META_REGION_0.0)
            .unwrap()
            .generation,
        0
    );
    drop(a);
    let engine = backend.node.meta_raft.store.engine();
    until(|| {
        WalEngine::checkpoint_reference(engine.path())
            .unwrap()
            .is_some()
    })
    .await;
    let journal = FlushJournal::new(f.path.join("n2/catalog.pending"));
    until(|| journal.load().unwrap().is_none()).await;
    assert!(!objects.is_empty());
    assert!(
        backend
            .driver
            .manifest_pair(kv9_common::META_REGION_0.0)
            .unwrap()
            .generation
            >= 1
    );
    tokio::task::spawn_blocking(move || drop(remote))
        .await
        .unwrap();
    drop((b, occupied));
    assert_eq!(budget.snapshot().active, 0);
    assert_eq!(budget.snapshot().heavy_peak, 2);
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn blocked_upload_does_not_starve_another_group_and_retirement_waits_for_actual_job() {
    let mut f = Fixture::new().await;
    let mut single = ExtraGroup::new(&f).await;
    let source = f.backends[0]
        .raw_directory
        .by_region(f.range.region)
        .unwrap();
    let source_engine = Arc::downgrade(source.capture_parts().1);
    drop(source);
    let store = Arc::new(GatedStore {
        memory: MemoryObjectStore::new(),
        region: f.range.region.0,
        entered: AtomicBool::new(false),
        blocked_puts: std::sync::atomic::AtomicUsize::new(0),
        blocked: Mutex::new(true),
        wake: Condvar::new(),
    });
    let release = Release(store.clone());
    let uploader = Arc::new(RemoteUploader::for_harness(store.clone()));
    let task = Maintenance::start(
        f.backends[0].clone(),
        uploader,
        true,
        Duration::from_millis(100),
        &tokio::runtime::Handle::current(),
    );
    until(|| store.entered.load(Ordering::SeqCst)).await;
    until(|| {
        WalEngine::checkpoint_reference(single.wal())
            .unwrap()
            .is_some()
    })
    .await;
    assert_eq!(task.snapshot().peak_in_flight, 2);
    assert!(task.snapshot().tracked_groups <= 2);
    // Remove the source while its upload is blocked. Cancellation cannot release
    // its engine or the task's actual slot before the blocking call returns.
    f.backends[0].raw_directory.remove(f.range.region);
    let source_driver = f.data.remove(0);
    source_driver.stop();
    f.owners.remove(1).join().unwrap();
    drop(source_driver);
    task.cancel();
    let draining = tokio::spawn(task.drain());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!draining.is_finished());
    assert!(source_engine.upgrade().is_some());
    drop(release);
    tokio::time::timeout(Duration::from_secs(5), draining)
        .await
        .unwrap()
        .unwrap();
    assert!(
        WalEngine::checkpoint_reference(f.path.join("n1/data.wal"))
            .unwrap()
            .is_none(),
        "a retired capture must not select a checkpoint after I/O completes"
    );
    assert!(
        source_engine.upgrade().is_none(),
        "actual upload completion releases the retired engine"
    );
    single.close_owner(&f);
    drop(single);
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn metadata_image_budget_defers_before_staging_then_really_publishes_after_release() {
    use kv9_engine::checkpoint::{FlushScope, RemoteUploader};
    let f = Fixture::new().await;
    let backend = &f.backends[1];
    let root = backend.ttl_context().unwrap().root.clone();
    let meta_engine = backend.node.meta_raft.store.engine();
    let sample = RemoteUploader::plan(
        meta_engine
            .freeze_with_scope(|view| {
                Ok(
                    kv9_meta::checkpoint::inspect_initial_checkpoint_base(view, &root)?
                        .flush_scope(),
                )
            })
            .unwrap(),
    )
    .unwrap();
    let bytes = sample.manifest().files.iter().map(|f| f.size).sum::<u64>();
    drop(sample);
    let limit = (bytes * 2).max(64 * 1024);
    let memory = Arc::new(kv9_engine::checkpoint_memory::ImageBudget::new(limit).unwrap());
    let objects = Arc::new(MemoryObjectStore::new());
    let uploader =
        Arc::new(RemoteUploader::for_harness(objects.clone()).with_image_budget(memory.clone()));
    // Hold a real image from another engine against the same node budget.
    // One key encodes to value length + 33 bytes including both footer keys.
    let held_engine = WalEngine::open(f.path.join("held-image.wal")).unwrap().0;
    let mut batch = kv9_engine::WriteBatch::new();
    batch.put(
        ColumnFamily::Default,
        b"k".to_vec(),
        vec![7; limit as usize - 33],
    );
    held_engine
        .write_applied(batch, kv9_common::AppliedPosition { term: 1, index: 1 })
        .unwrap();
    let held = uploader
        .plan_checkpoint(
            held_engine
                .freeze(FlushScope {
                    cluster: root.cluster_id.to_string(),
                    region: 999,
                    conf_ver: 1,
                    version: 1,
                })
                .unwrap(),
        )
        .unwrap();
    assert_eq!(memory.snapshot().retained_bytes, limit);
    let remote = crate::remote_storage::RemoteStorage::start(
        backend.node.clone(),
        backend.driver.clone(),
        crate::remote_storage::PreparedRemote::for_test(
            &f.path.join("n2"),
            &root,
            uploader.clone(),
        ),
        backend.maintenance.clone(),
        Arc::new(crate::runtime::checkpoint_retention::RoutedLedger::new(
            backend,
            tokio::runtime::Handle::current(),
        )),
    )
    .unwrap();
    until(|| memory.snapshot().refused_images > 0).await;
    assert!(objects.is_empty());
    assert!(!f.path.join("n2/catalog.pending").exists());
    assert_eq!(
        backend
            .driver
            .manifest_pair(kv9_common::META_REGION_0.0)
            .unwrap()
            .generation,
        0
    );
    assert_eq!(memory.snapshot().retained_bytes, limit);
    drop(held);
    until(|| {
        WalEngine::checkpoint_reference(meta_engine.path())
            .unwrap()
            .is_some()
    })
    .await;
    until(|| {
        uploader
            .journal(f.path.join("n2/catalog.pending"))
            .load()
            .unwrap()
            .is_none()
    })
    .await;
    assert!(!objects.is_empty());
    assert!(
        backend
            .driver
            .manifest_pair(kv9_common::META_REGION_0.0)
            .unwrap()
            .generation
            >= 1
    );
    tokio::task::spawn_blocking(move || drop(remote))
        .await
        .unwrap();
    assert_eq!(memory.snapshot().retained_bytes, 0);
    assert_eq!(memory.snapshot().active_images, 0);
    assert!(memory.snapshot().peak_bytes <= limit);
    drop(held_engine);
    f.close().await;
}
