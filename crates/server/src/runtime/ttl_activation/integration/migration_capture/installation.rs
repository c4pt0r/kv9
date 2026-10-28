use super::*;
mod recovery;
use crate::checkpoint_retention::{drain_outbox::DrainOutbox, CheckpointLedger};
use crate::installation_retention::RetainedImages;
use crate::runtime::checkpoint_retention::RoutedLedger;
use kv9_common::retention::OwnerId;
use kv9_engine::checkpoint::CheckpointRestore;
use kv9_meta::retention::{
    encode_request, image_reader, LedgerOwner, LedgerRequest, LocalLifetime, OwnerBinding,
};

struct Ledger {
    routed: RoutedLedger,
    lifetime: LocalLifetime,
    outbox: Arc<DrainOutbox>,
    data_read_fences: Arc<kv9_common::data_read_fence::DataReadFences>,
}
impl CheckpointLedger for Ledger {
    fn data_read_fences(&self) -> Result<Option<Arc<kv9_common::data_read_fence::DataReadFences>>> {
        Ok(Some(self.data_read_fences.clone()))
    }

    fn apply(&self, bytes: Vec<u8>) -> Result<()> {
        self.routed.apply(bytes)
    }
    fn owner(&self, root: RootDigest, owner: OwnerId) -> Result<Option<Vec<u8>>> {
        self.routed.owner(root, owner)
    }
    fn local_lifetime(&self) -> Result<LocalLifetime> {
        Ok(self.lifetime)
    }
    fn drain_outbox(&self) -> Result<Option<Arc<DrainOutbox>>> {
        Ok(Some(self.outbox.clone()))
    }
}
struct Setup {
    f: Fixture,
    root: RootDescriptor,
    identity: StoreIdentity,
    guard: Arc<StoreGuard>,
    objects: Arc<Objects>,
    uploader: Arc<RemoteUploader>,
    ledger: Arc<Ledger>,
    record: Vec<u8>,
    source: OwnerBinding,
}
impl Setup {
    async fn new() -> Self {
        let f = Fixture::new().await;
        let root = f.backends[0].ttl_context().unwrap().root.clone();
        let digest = root.digest();
        let operation = [96; 16];
        assert!(matches!(
            f.client(0).activate().await.outcome,
            TtlOutcome::Success { .. }
        ));
        assert!(matches!(
            f.client(0)
                .call(Operation::Put {
                    pairs: vec![(b"retained-install".to_vec(), b"ttl-value".to_vec())],
                    ttl_secs: Some(60),
                })
                .await
                .outcome,
            TtlOutcome::Success { .. }
        ));
        let (identity, guard, _) = prepare_migration(&f, &root, operation).await;
        let objects = Arc::new(Objects::default());
        enable_payload_replicas(&f, &objects);
        let uploader = Arc::new(RemoteUploader::for_harness(objects.clone()));
        let source = with_uploader(&f.backends[0], uploader.clone());
        *f.hooks[0].capture_backend.lock().unwrap() = Some(source.clone());
        let backend = source.clone();
        let plan = tokio::task::spawn_blocking(move || {
            backend.plan_migration_image("test", digest, operation)
        })
        .await
        .unwrap()
        .unwrap();
        let backend = f.backends[1].clone();
        let pins = tokio::task::spawn_blocking(move || {
            backend.bind_migration_image("test", digest, operation, &plan.manifest)
        })
        .await
        .unwrap()
        .unwrap();
        let record = tokio::task::spawn_blocking(move || {
            source.capture_migration_image("test", digest, operation)
        })
        .await
        .unwrap()
        .unwrap()
        .record;
        let source = kv9_meta::retention::retention_owner(
            f.backends[1]
                .node
                .meta_raft
                .store
                .engine()
                .snapshot()
                .unwrap()
                .as_ref(),
            &root,
            pins.destination_owner,
        )
        .unwrap()
        .unwrap()
        .binding;
        let ledger = Arc::new(Ledger {
            routed: RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current()),
            lifetime: LocalLifetime {
                node: identity.node_id.0,
                store: *identity.store_incarnation.as_bytes(),
                process: [42; 16],
            },
            outbox: DrainOutbox::open(guard.directory(), identity).unwrap(),
            data_read_fences: guard.data_read_fences(&identity).unwrap(),
        });
        Self {
            f,
            root,
            identity,
            guard: Arc::new(guard),
            objects,
            uploader,
            ledger,
            record,
            source,
        }
    }
    fn reader(&self) -> OwnerBinding {
        image_reader::binding(&self.source, self.ledger.lifetime).unwrap()
    }
    fn observed(&self, binding: &OwnerBinding) -> Option<LedgerOwner> {
        let leader = self
            .f
            .meta
            .iter()
            .position(|d| d.status().role == Role::Leader)
            .unwrap();
        kv9_meta::retention::retention_owner(
            self.f.backends[leader]
                .node
                .meta_raft
                .store
                .engine()
                .snapshot()
                .unwrap()
                .as_ref(),
            &self.root,
            binding.descriptor.id,
        )
        .unwrap()
    }
    fn install(
        &self,
        uploader: Arc<RemoteUploader>,
        sources: Vec<OwnerId>,
    ) -> tokio::task::JoinHandle<Result<kv9_raft::snapshot_install::InstalledImage>> {
        let guard = self.guard.clone();
        let identity = self.identity;
        let root = self.root.clone();
        let ledger = self.ledger.clone();
        let record = self.record.clone();
        tokio::task::spawn_blocking(move || {
            let reader = RetainedImages::new(&uploader, ledger.as_ref(), &root, &sources)?;
            kv9_raft::snapshot_install::install_captured_record_with_reader(
                &guard, identity, &record, &reader,
            )
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn retained_install_and_selected_retry_read_exact_ttl_image_and_release_only_local_reader() {
    let s = Setup::new().await;
    let puts = s.objects.puts.load(Ordering::SeqCst);
    let installed = s
        .install(s.uploader.clone(), vec![s.source.descriptor.id])
        .await
        .unwrap()
        .unwrap();
    assert!(installed.ttl_watermark.is_some());
    let reader = s.observed(&s.reader()).unwrap();
    assert_eq!(reader.phase, PinPhase::Released);
    assert_eq!(reader.binding.generation.get(), 2); // prepared and final verification
    let repeated = s
        .install(s.uploader.clone(), vec![s.source.descriptor.id])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(repeated.generation, installed.generation);
    assert_eq!(s.observed(&s.reader()).unwrap().binding.generation.get(), 3);
    assert_eq!(s.observed(&s.source).unwrap().phase, PinPhase::Published);
    assert!(!s.ledger.outbox.has_pending());
    assert_eq!(s.objects.puts.load(Ordering::SeqCst), puts);
    // A requested manifest not covered by the supplied owner is refused before GET.
    let root = s.root.clone();
    let ledger = s.ledger.clone();
    let uploader = s.uploader.clone();
    let owners = vec![s.source.descriptor.id];
    let descriptor = kv9_meta::data_groups::capture::committed_capture(
        s.f.backends[1]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap()
            .as_ref(),
        &root,
        [96; 16],
    )
    .unwrap()
    .unwrap()
    .descriptor()
    .clone();
    let mut wrong = descriptor.manifest().clone();
    wrong.index += 1;
    let gets = s.objects.gets.load(Ordering::SeqCst);
    assert!(tokio::task::spawn_blocking(move || RetainedImages::new(
        &uploader,
        ledger.as_ref(),
        &root,
        &owners
    )?
    .restore(&wrong))
    .await
    .unwrap()
    .is_err());
    assert_eq!(s.objects.gets.load(Ordering::SeqCst), gets);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replacement_requires_independent_exact_coverage_for_old_and_new_selected_images() {
    let mut s = Setup::new().await;
    let first = s
        .install(s.uploader.clone(), vec![s.source.descriptor.id])
        .await
        .unwrap()
        .unwrap();
    let engine = s.f.backends[0]
        .raw_directory
        .by_region(s.f.range.region)
        .unwrap()
        .capture_parts()
        .1
        .clone();
    let driver = s.f.data[0].clone();
    let uploader = s.uploader.clone();
    let ledger = s.ledger.clone();
    let mut source = s.source.clone();
    let root = s.root.clone();
    // A separately Published image fixture exercises replacement. This is
    // not a new production capture or a portable migration authority receipt.
    let (record, next_source) = tokio::task::spawn_blocking(move || {
        let at = driver.propose(&Command::Noop).unwrap();
        driver.wait_applied(at, Duration::from_secs(3)).unwrap();
        let planned =
            kv9_raft::snapshot_install::capture::plan_capture(&driver, &engine, root.cluster_id)
                .unwrap();
        let manifest = planned.manifest().clone();
        source.descriptor.id = OwnerId::new([98; 16]).unwrap();
        source.descriptor.kind = kv9_meta::retention::OwnerKind::Snapshot;
        source.descriptor.subject = *RootDigest::sha256(&manifest.encode().unwrap()).as_bytes();
        source.resources =
            crate::checkpoint_retention::manifest_resources(root.digest(), &manifest).unwrap();
        for request in [
            LedgerRequest::Register(source.resources.clone()),
            LedgerRequest::Acquire(source.clone()),
            LedgerRequest::Publish(source.token()),
        ] {
            ledger
                .apply(encode_request(root.digest(), &request).unwrap())
                .unwrap();
        }
        (planned.upload(&uploader).unwrap().record, source)
    })
    .await
    .unwrap();
    s.record = record;
    let gets = s.objects.gets.load(Ordering::SeqCst);
    let puts = s.objects.puts.load(Ordering::SeqCst);
    let refused = s
        .install(s.uploader.clone(), vec![next_source.descriptor.id])
        .await
        .unwrap()
        .unwrap_err();
    assert!(refused.to_string().contains("no exact owner"), "{refused}");
    assert_eq!(s.objects.gets.load(Ordering::SeqCst), gets);
    let next = s
        .install(
            s.uploader.clone(),
            vec![s.source.descriptor.id, next_source.descriptor.id],
        )
        .await
        .unwrap()
        .unwrap();
    assert_ne!(next.generation, first.generation);
    assert!(next.position.index > first.position.index);
    assert!(next.ttl_watermark.is_some());
    let next_reader = image_reader::binding(&next_source, s.ledger.lifetime).unwrap();
    assert_eq!(s.observed(&next_reader).unwrap().phase, PinPhase::Released);
    assert_eq!(s.observed(&s.reader()).unwrap().phase, PinPhase::Released);
    assert_eq!(s.observed(&s.source).unwrap().phase, PinPhase::Published);
    assert_eq!(s.observed(&next_source).unwrap().phase, PinPhase::Published);
    assert_eq!(s.objects.puts.load(Ordering::SeqCst), puts);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stale_reader_observation_cannot_leave_conflicting_report_after_concurrent_read() {
    let s = Setup::new().await;
    let descriptor = kv9_meta::data_groups::capture::committed_capture(
        s.f.backends[1]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap()
            .as_ref(),
        &s.root,
        [96; 16],
    )
    .unwrap()
    .unwrap()
    .descriptor()
    .clone();
    let manifest = descriptor.manifest().clone();
    let (notify, seen) = std::sync::mpsc::channel();
    let (resume, paused) = std::sync::mpsc::channel();
    *s.f.hooks[1].publisher_owner_gate.lock().unwrap() = Some(PublisherOwnerGate {
        owner: s.reader().descriptor.id,
        observed: notify,
        resume: paused,
    });
    let read = || {
        let root = s.root.clone();
        let ledger = s.ledger.clone();
        let uploader = s.uploader.clone();
        let manifest = manifest.clone();
        let source = s.source.descriptor.id;
        tokio::task::spawn_blocking(move || {
            RetainedImages::new(&uploader, ledger.as_ref(), &root, &[source])?.restore(&manifest)
        })
    };
    let slow = read();
    tokio::task::spawn_blocking(move || seen.recv_timeout(Duration::from_secs(10)))
        .await
        .unwrap()
        .unwrap();
    read().await.unwrap().unwrap();
    let gets = s.objects.gets.load(Ordering::SeqCst);
    resume.send(()).unwrap();
    let error = slow.await.unwrap().expect_err("stale reader must refuse");
    assert!(
        error.to_string().contains("advanced before reservation"),
        "{error}"
    );
    assert_eq!(s.objects.gets.load(Ordering::SeqCst), gets);
    assert!(!s.ledger.outbox.has_pending());
    read().await.unwrap().unwrap();
    let finished = s.observed(&s.reader()).unwrap();
    assert_eq!(finished.phase, PinPhase::Released);
    assert_eq!(finished.binding.generation.get(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn timed_out_install_get_keeps_reader_and_group_lock_until_actual_worker_drains() {
    let s = Setup::new().await;
    let (store, pause) =
        kv9_engine::MinioObjectStore::with_paused_memory_worker(Duration::from_millis(100))
            .unwrap();
    let task = s.install(
        Arc::new(RemoteUploader::new(Arc::new(store))),
        vec![s.source.descriptor.id],
    );
    let pause = tokio::task::spawn_blocking(move || {
        assert!(pause.wait_for_closed_execution(Duration::from_secs(10)));
        pause
    })
    .await
    .unwrap();
    assert!(!task.is_finished());
    assert_eq!(s.observed(&s.reader()).unwrap().phase, PinPhase::Published);
    assert!(StoreGuard::lock(s.guard.directory()).is_err());
    assert!(kv9_raft::snapshot_install::JointInstaller::open(
        &s.guard,
        s.identity,
        s.f.range.clone()
    )
    .is_err());
    assert_eq!(
        serde_json::to_value(s.ledger.outbox.snapshot()).unwrap()["reserved"],
        1
    );
    drop(pause);
    assert!(tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    assert_eq!(s.observed(&s.reader()).unwrap().phase, PinPhase::Released);
    assert!(!s.ledger.outbox.has_pending());
    assert!(!s
        .guard
        .directory()
        .join(format!(
            "data-groups/{}/installed-generation",
            s.f.range.region.0
        ))
        .exists());
    assert_eq!(s.observed(&s.source).unwrap().phase, PinPhase::Published);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn uncertain_install_entry_and_lost_finish_replay_original_report_after_new_context_and_leader(
) {
    let mut s = Setup::new().await;
    let gets = s.objects.gets.load(Ordering::SeqCst);
    let calls = s.f.hooks[1].retention_calls.load(Ordering::SeqCst);
    s.f.hooks[1]
        .retention_refuse_from
        .store(calls + 2, Ordering::SeqCst);
    assert!(s
        .install(s.uploader.clone(), vec![s.source.descriptor.id])
        .await
        .unwrap()
        .is_err());
    assert_eq!(s.objects.gets.load(Ordering::SeqCst), gets);
    assert_eq!(s.observed(&s.reader()).unwrap().phase, PinPhase::Held);
    assert!(s.ledger.outbox.has_pending());
    s.f.hooks[1]
        .retention_refuse_from
        .store(0, Ordering::SeqCst);
    let ledger = s.ledger.clone();
    tokio::task::spawn_blocking(move || ledger.outbox.replay_one(ledger.as_ref()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s.observed(&s.reader()).unwrap().phase, PinPhase::Released);
    s.f.hooks[1].retention_read_fault.store(2, Ordering::SeqCst);
    assert!(s
        .install(s.uploader.clone(), vec![s.source.descriptor.id])
        .await
        .unwrap()
        .is_err());
    let old = s.observed(&s.reader()).unwrap();
    assert_eq!(old.phase, PinPhase::Released);
    assert_eq!(old.binding.generation.get(), 2);
    let gets = s.objects.gets.load(Ordering::SeqCst);
    assert!(s
        .install(s.uploader.clone(), vec![s.source.descriptor.id])
        .await
        .unwrap()
        .is_err());
    assert_eq!(s.objects.gets.load(Ordering::SeqCst), gets);
    let saved: Vec<_> = std::fs::read_dir(s.guard.directory().join("checkpoint-drains"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(saved.len(), 1);
    let original = std::fs::read(&saved[0]).unwrap();
    assert_eq!(
        original,
        encode_request(
            s.root.digest(),
            &LedgerRequest::FinishImageRead {
                successor: s.source.token(),
                reader: old.binding.clone()
            }
        )
        .unwrap()
    );
    s.f.meta[1].peer().transfer_leader_for_tests(NodeId(3));
    leader(&s.f.meta[2]).await;
    let reopened = Arc::new(Ledger {
        routed: RoutedLedger::new(&s.f.backends[0], tokio::runtime::Handle::current()),
        lifetime: LocalLifetime {
            process: [43; 16],
            ..s.ledger.lifetime
        },
        outbox: DrainOutbox::open(s.guard.directory(), s.identity).unwrap(),
        data_read_fences: s.guard.data_read_fences(&s.identity).unwrap(),
    });
    let next = image_reader::binding(&s.source, reopened.lifetime).unwrap();
    let original_reader = s.reader();
    let retired_context = std::mem::replace(&mut s.ledger, reopened.clone());
    assert_eq!(Arc::strong_count(&retired_context), 1);
    drop(retired_context);
    let copy = reopened.clone();
    tokio::task::spawn_blocking(move || copy.outbox.replay_one(copy.as_ref()))
        .await
        .unwrap()
        .unwrap();
    assert!(!reopened.outbox.has_pending());
    assert_eq!(s.observed(&original_reader), Some(old.clone()));
    assert!(s.observed(&next).is_none());
    assert_eq!(s.observed(&s.source).unwrap().phase, PinPhase::Published);
    assert_eq!(s.objects.gets.load(Ordering::SeqCst), gets);
    let path = s.f.shutdown_keep().await;
    let (engine, _) = WalEngine::open(path.join("n3/meta.wal")).unwrap();
    let view = engine.snapshot().unwrap();
    assert_eq!(
        kv9_meta::retention::retention_owner(view.as_ref(), &s.root, original_reader.descriptor.id)
            .unwrap(),
        Some(old)
    );
    assert_eq!(
        kv9_meta::retention::retention_owner(view.as_ref(), &s.root, s.source.descriptor.id)
            .unwrap()
            .unwrap()
            .phase,
        PinPhase::Published
    );
    drop(view);
    drop(engine);
    drop(s.guard);
    std::fs::remove_dir_all(path).unwrap();
}
