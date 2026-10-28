use super::*;
use crate::bootstrap_retention::BootstrapReader;
use crate::checkpoint_retention::CheckpointLedger;
use crate::remote_storage::{PreparedRemote, RemoteStorage};
use crate::runtime::checkpoint_retention::RoutedLedger;
use kv9_common::retention::{OwnerId, PinPhase};
use kv9_common::{RootDigest, META_REGION_0};
use kv9_meta::retention::{
    bootstrap_reader, decode_owner_observation, decode_request, LedgerRequest, LocalLifetime,
    OwnerKind,
};
mod closure;
mod enrollment;
mod image_gate;
mod read_gate;
mod retirement;
mod selection;
mod unentered;

#[derive(Debug, Default)]
struct Objects {
    memory: MemoryObjectStore,
    gets: AtomicUsize,
    puts: AtomicUsize,
}
impl ObjectStore for Objects {
    fn get(&self, key: &ObjectKey) -> Result<Option<Vec<u8>>> {
        self.gets.fetch_add(1, Ordering::SeqCst);
        self.memory.get(key)
    }
    fn put(&self, key: &ObjectKey, bytes: &[u8]) -> Result<()> {
        self.puts.fetch_add(1, Ordering::SeqCst);
        self.memory.put(key, bytes)
    }
    fn delete(&self, _: &ObjectKey) -> Result<()> {
        panic!("no GC")
    }
    fn list(&self, _: &str) -> Result<Vec<ObjectKey>> {
        panic!("no listing")
    }
}

fn worker(
    f: &Fixture,
    i: usize,
    uploader: Arc<RemoteUploader>,
    ledger: Arc<dyn CheckpointLedger>,
) -> RemoteStorage {
    let b = &f.backends[i];
    RemoteStorage::start(
        b.node.clone(),
        b.driver.clone(),
        PreparedRemote::for_test(
            &f.path.join(format!("n{}", i + 1)),
            &b.ttl_context().unwrap().root,
            uploader,
        ),
        b.maintenance.clone(),
        ledger,
    )
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn all_metadata_replicas_recover_offline_with_exact_bootstrap_grants_and_no_live_ledger() {
    let f = Fixture::new().await;
    let root = f.backends[0].ttl_context().unwrap().root.clone();
    let objects = Arc::new(Objects::default());
    let uploader = Arc::new(RemoteUploader::for_harness(objects.clone()));
    let workers: Vec<_> = (0..3)
        .map(|i| {
            worker(
                &f,
                i,
                uploader.clone(),
                Arc::new(RoutedLedger::new(
                    &f.backends[i],
                    tokio::runtime::Handle::current(),
                )),
            )
        })
        .collect();
    until(|| {
        f.backends.iter().all(|b| {
            b.node
                .meta_raft
                .store
                .engine()
                .selected_checkpoint()
                .unwrap()
                .is_some()
        })
    })
    .await;
    tokio::task::spawn_blocking(move || drop(workers))
        .await
        .unwrap();
    let at = f.meta[1].driver_applied().unwrap();
    until(|| {
        f.meta
            .iter()
            .all(|d| d.driver_applied().is_some_and(|p| p.index >= at.index))
    })
    .await;
    let directory = f.shutdown_keep().await;
    let puts = objects.puts.load(Ordering::SeqCst);
    let r = root.clone();
    let u = uploader.clone();
    let o = objects.clone();
    let dir = directory.clone();
    tokio::task::spawn_blocking(move || {
        for i in 1..=3 {
            let path = dir.join(format!("n{i}"));
            let identity = StoreIdentity::for_voter(&r, NodeId(i)).unwrap();
            let _guard = StoreGuard::lock(&path).unwrap();
            let mut storage = DiskRaftStorage::recover(&path.join("meta-raft")).unwrap();
            let plan = kv9_raft::state_machine::checkpoint_recovery::inspect_checkpoint_read(
                &mut storage,
                path.join("meta.wal"),
                r.cluster_id.to_string(),
                META_REGION_0.0,
            )
            .unwrap();
            let manifest = plan.manifest().unwrap().clone();
            let reader = BootstrapReader {
                directory: &path,
                root: &r,
                identity,
                uploader: &u,
            };
            let receipt = path.join("catalog.bootstrap-reads");
            let saved = std::fs::read(&receipt).unwrap();
            std::fs::remove_file(&receipt).unwrap();
            let gets = o.gets.load(Ordering::SeqCst);
            assert!(
                kv9_meta::recovery::open_initial_checkpoint_engine_with_reader(
                    &mut storage,
                    path.join("meta.wal"),
                    Some(&reader),
                    &r
                )
                .is_err()
            );
            assert_eq!(o.gets.load(Ordering::SeqCst), gets);
            std::fs::write(&receipt, saved).unwrap();
            for _ in 0..2 {
                let (engine, _, anchor) =
                    kv9_meta::recovery::open_initial_checkpoint_engine_with_reader(
                        &mut storage,
                        path.join("meta.wal"),
                        Some(&reader),
                        &r,
                    )
                    .unwrap();
                assert_eq!(
                    anchor.unwrap().descriptor().manifest,
                    manifest.encode().unwrap()
                );
                let grant =
                    bootstrap_reader::binding(&r, &manifest, plan.predecessor().unwrap(), identity)
                        .unwrap();
                let owner = kv9_meta::retention::retention_owner(
                    engine.snapshot().unwrap().as_ref(),
                    &r,
                    grant.descriptor.id,
                )
                .unwrap()
                .unwrap();
                assert_eq!(owner.phase, PinPhase::Published);
                assert_eq!(owner.binding, grant);
                drop(engine);
            }
            assert!(o.gets.load(Ordering::SeqCst) > gets);
        }
    })
    .await
    .unwrap();
    assert_eq!(objects.puts.load(Ordering::SeqCst), puts);
    std::fs::remove_dir_all(directory).unwrap();
}

struct LoseGrantReceipt {
    ledger: RoutedLedger,
    root: RootDigest,
    lost: AtomicUsize,
    fault: AtomicBool,
}
impl CheckpointLedger for LoseGrantReceipt {
    fn apply(&self, bytes: Vec<u8>) -> Result<()> {
        let lose = if let LedgerRequest::Publish(token) = decode_request(&bytes, self.root)? {
            self.ledger
                .owner(self.root, token.owner)?
                .is_some_and(|bytes| {
                    decode_owner_observation(&bytes, self.root, token.owner)
                        .unwrap()
                        .binding
                        .descriptor
                        .kind
                        == OwnerKind::BootstrapReader
                })
        } else {
            false
        };
        self.ledger.apply(bytes)?;
        if lose && self.fault.load(Ordering::SeqCst) {
            self.lost.fetch_add(1, Ordering::SeqCst);
            return Err(Error::MetaNotReady(
                "lost committed bootstrap receipt".into(),
            ));
        }
        Ok(())
    }
    fn owner(&self, root: RootDigest, id: OwnerId) -> Result<Option<Vec<u8>>> {
        self.ledger.owner(root, id)
    }
    fn local_lifetime(&self) -> Result<LocalLifetime> {
        self.ledger.local_lifetime()
    }
    fn drain_outbox(
        &self,
    ) -> Result<Option<Arc<crate::checkpoint_retention::drain_outbox::DrainOutbox>>> {
        self.ledger.drain_outbox()
    }
    fn executors(
        &self,
        root: RootDigest,
        pending: OwnerId,
        after: Option<OwnerId>,
        limit: usize,
    ) -> Result<kv9_meta::retention::checkpoint_admission::ExecutorPage> {
        self.ledger.executors(root, pending, after, limit)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn metadata_worker_keeps_wal_selection_unchanged_until_grant_receipt_is_confirmed() {
    let f = Fixture::new().await;
    let b = &f.backends[1];
    let root = b.ttl_context().unwrap().root.clone();
    let ledger = Arc::new(LoseGrantReceipt {
        ledger: RoutedLedger::new(b, tokio::runtime::Handle::current()),
        root: root.digest(),
        lost: AtomicUsize::new(0),
        fault: AtomicBool::new(true),
    });
    let objects = Arc::new(Objects::default());
    let work = worker(
        &f,
        1,
        Arc::new(RemoteUploader::for_harness(objects.clone())),
        ledger.clone(),
    );
    until(|| ledger.lost.load(Ordering::SeqCst) >= 2).await;
    assert!(b
        .node
        .meta_raft
        .store
        .engine()
        .selected_checkpoint()
        .unwrap()
        .is_none());
    // Prospective registration is durable, but it does not authorize GET.
    let manifest =
        CheckpointManifest::decode(&b.driver.manifest_at(META_REGION_0.0, 1).unwrap().unwrap().2)
            .unwrap();
    let directory = f.path.join("n2");
    let identity = StoreIdentity::for_voter(&root, NodeId(2)).unwrap();
    let uploader = RemoteUploader::for_harness(objects.clone());
    let count = objects.gets.load(Ordering::SeqCst);
    use kv9_engine::checkpoint::CheckpointRestore;
    assert!(BootstrapReader {
        directory: &directory,
        root: &root,
        identity,
        uploader: &uploader
    }
    .restore(&manifest)
    .is_err());
    assert_eq!(objects.gets.load(Ordering::SeqCst), count);
    ledger.fault.store(false, Ordering::SeqCst);
    until(|| {
        b.node
            .meta_raft
            .store
            .engine()
            .selected_checkpoint()
            .unwrap()
            .is_some()
    })
    .await;
    tokio::task::spawn_blocking(move || drop(work))
        .await
        .unwrap();
    f.close().await;
}

mod image_census;

mod image_publishers;
