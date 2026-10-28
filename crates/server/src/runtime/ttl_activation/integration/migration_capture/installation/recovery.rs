use super::*;
use crate::recovery_retention::RecoveryRetention;
use crate::region_manager::{RecoveryDecisions, RegionManager};
use kv9_raft::grpc::GrpcTransport;

fn manager(s: &Setup) -> RegionManager {
    let mut m = RegionManager::new(s.guard.directory(), s.identity, 1);
    m.uploader = Some(s.uploader.clone());
    m.data_read_fences = Some(s.ledger.data_read_fences.clone());
    m.recovery_retention = Some(Arc::new(RecoveryRetention::new(
        s.root.clone(),
        s.identity,
        s.ledger.clone(),
    )));
    m
}
fn attempt(
    s: &Setup,
    mut m: RegionManager,
    uploader: Arc<RemoteUploader>,
) -> tokio::task::JoinHandle<(RegionManager, Result<()>)> {
    let migration = kv9_meta::data_groups::migration::committed_migrations(
        &s.f.backends[1].node.meta_raft.store,
    )
    .unwrap()
    .remove(0);
    let guard = s.guard.clone();
    let transport = GrpcTransport::new(
        s.identity.node_id,
        None,
        tokio::runtime::Handle::current(),
        s.root.digest(),
    );
    tokio::task::spawn_blocking(move || {
        let result = m.adopt_installed(
            &guard,
            &migration,
            &transport,
            Duration::from_millis(10),
            &uploader,
            RecoveryDecisions {
                truncations: &[],
                compactions: &[],
            },
        );
        (m, result)
    })
}
async fn installed(s: &Setup) {
    s.install(s.uploader.clone(), vec![s.source.descriptor.id])
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn runtime_adoption_and_restart_admit_actual_reads_before_starting_driver() {
    let s = Setup::new().await;
    installed(&s).await;
    let before = s.observed(&s.reader()).unwrap().binding.generation;
    let puts = s.objects.puts.load(Ordering::SeqCst);
    let mut m = manager(&s);
    for _ in 0..2 {
        let (mut next, result) = attempt(&s, m, s.uploader.clone()).await.unwrap();
        result.unwrap();
        let facts = next.recovery_status(s.f.range.region).unwrap();
        assert!(facts.driver_started());
        assert!(facts.at_open().unwrap().used_original_install_image());
        assert!(next.status(s.f.range.region).is_ok());
        let reader = s.observed(&s.reader()).unwrap();
        assert_eq!(reader.phase, PinPhase::Released);
        assert!(reader.binding.generation > before);
        assert_eq!(s.observed(&s.source).unwrap().phase, PinPhase::Published);
        assert!(!s.ledger.outbox.has_pending());
        next.shutdown();
        drop(next);
        m = manager(&s);
    }
    assert_eq!(s.objects.puts.load(Ordering::SeqCst), puts);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn adoption_missing_receipts_block_driver_and_retry_uses_persisted_drain() {
    let s = Setup::new().await;
    installed(&s).await;
    let gets = s.objects.gets.load(Ordering::SeqCst);
    let calls = s.f.hooks[1].retention_calls.load(Ordering::SeqCst);
    s.f.hooks[1]
        .retention_refuse_from
        .store(calls + 1, Ordering::SeqCst);
    let (m, result) = attempt(&s, manager(&s), s.uploader.clone()).await.unwrap();
    assert!(matches!(result, Err(Error::MetaNotReady(_))));
    assert!(m.status(s.f.range.region).is_err());
    assert_eq!(s.objects.gets.load(Ordering::SeqCst), gets);
    s.f.hooks[1]
        .retention_refuse_from
        .store(0, Ordering::SeqCst);
    if s.ledger.outbox.has_pending() {
        let ledger = s.ledger.clone();
        tokio::task::spawn_blocking(move || ledger.outbox.replay_one(ledger.as_ref()))
            .await
            .unwrap()
            .unwrap();
    }
    s.f.hooks[1].retention_read_fault.store(2, Ordering::SeqCst);
    let (m, result) = attempt(&s, m, s.uploader.clone()).await.unwrap();
    assert!(matches!(result, Err(Error::MetaNotReady(_))));
    assert!(m.status(s.f.range.region).is_err());
    assert!(s.ledger.outbox.has_pending());
    let gets = s.objects.gets.load(Ordering::SeqCst);
    let (m, result) = attempt(&s, m, s.uploader.clone()).await.unwrap();
    assert!(result.is_err());
    assert_eq!(s.objects.gets.load(Ordering::SeqCst), gets);
    s.f.hooks[1].retention_read_fault.store(0, Ordering::SeqCst);
    let ledger = s.ledger.clone();
    tokio::task::spawn_blocking(move || ledger.outbox.replay_one(ledger.as_ref()))
        .await
        .unwrap()
        .unwrap();
    let (mut m, result) = attempt(&s, m, s.uploader.clone()).await.unwrap();
    result.unwrap();
    assert!(m.status(s.f.range.region).is_ok());
    assert!(!s.ledger.outbox.has_pending());
    m.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn adoption_get_timeout_holds_pin_and_group_lock_until_real_worker_exits() {
    let s = Setup::new().await;
    installed(&s).await;
    let (store, pause) =
        kv9_engine::MinioObjectStore::with_paused_memory_worker(Duration::from_millis(100))
            .unwrap();
    let task = attempt(
        &s,
        manager(&s),
        Arc::new(RemoteUploader::new(Arc::new(store))),
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
    drop(pause);
    let (m, result) = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result, Err(Error::MetaNotReady(_))));
    assert!(m.status(s.f.range.region).is_err());
    assert_eq!(s.observed(&s.reader()).unwrap().phase, PinPhase::Released);
    assert_eq!(s.observed(&s.source).unwrap().phase, PinPhase::Published);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn adopted_v2_new_checkpoint_reads_exact_pending_or_version_without_original_sst() {
    for version in [false, true] {
        let s = Setup::new().await;
        installed(&s).await;
        let root = s.root.clone();
        let guard = s.guard.clone();
        let identity = s.identity;
        let range = s.f.range.clone();
        let uploader = s.uploader.clone();
        let ledger = s.ledger.clone();
        let objects = s.objects.clone();
        let original_owner = s.source.descriptor.id;
        let expected = tokio::task::spawn_blocking(move || {
            use kv9_common::raw_ttl::{ClockSample, ClockWatermark, CLOCK_KEY};
            use kv9_engine::checkpoint::FlushScope;
            use kv9_engine::{ColumnFamily, Engine, ObjectKey, ObjectStore};
            use kv9_meta::retention::{
                checkpoint_intent::CheckpointIntent, checkpoint_reader, OwnerKind,
            };
            use kv9_raft::command::{RawTtlAction, RawTtlCommand};
            use kv9_raft::rawnode::PersistentRaftStorage;
            use kv9_raft::state_machine::MemStateMachine;
            use kv9_raft::{ApplyOutcome, ManifestChangePayload, ManifestVerdict};
            use raft::{prelude::Entry, Storage};
            let retention = RecoveryRetention::new(root.clone(), identity, ledger.clone());
            let reader = retention.reader(&uploader, Some(original_owner));
            let adopted = kv9_raft::snapshot_install::adopt_for_runtime_with_reader(
                &guard,
                identity,
                range.region,
                &reader,
            )
            .unwrap();
            let mut storage = DiskRaftStorage::recover(&adopted.raft_directory).unwrap();
            let engine = Arc::new(
                adopted
                    .open_engine_with_retained_reader(&mut storage, &uploader, &reader)
                    .unwrap()
                    .0,
            );
            let original = WalEngine::checkpoint_reference(&adopted.engine_wal)
                .unwrap()
                .unwrap();
            let watermark = ClockWatermark::decode(
                &engine
                    .get(ColumnFamily::Default, CLOCK_KEY)
                    .unwrap()
                    .unwrap(),
            )
            .unwrap();
            let mut state = MemStateMachine::with_engine(engine.clone()).unwrap();
            state
                .set_data_group(range.root, range.region, range.creation)
                .unwrap();
            let mut hs = storage.initial_state().unwrap().hard_state;
            let cut = kv9_common::AppliedPosition {
                term: hs.term,
                index: hs.commit + 1,
            };
            let observe = Command::RawTtl(
                RawTtlCommand::new(
                    &range,
                    RawTtlAction::Observe {
                        observed: ClockSample::with_uncertainty(watermark.unix_ms() + 1000, 0)
                            .unwrap()
                            .advance(watermark)
                            .unwrap(),
                    },
                )
                .unwrap(),
            );
            hs.commit = cut.index;
            storage
                .persist_ready(
                    &[Entry {
                        term: cut.term,
                        index: cut.index,
                        data: observe.encode().into(),
                        ..Default::default()
                    }],
                    Some(&hs),
                )
                .unwrap();
            state.apply_at(cut, &observe).unwrap();
            let plan = RemoteUploader::plan(
                engine
                    .freeze(FlushScope {
                        cluster: root.cluster_id.to_string(),
                        region: range.region.0,
                        conf_ver: range.conf_ver,
                        version: range.version,
                    })
                    .unwrap(),
            )
            .unwrap();
            let manifest = plan.manifest().clone();
            let intent = CheckpointIntent::new(&root, &manifest, 0).unwrap();
            ledger
                .apply(
                    encode_request(
                        root.digest(),
                        &LedgerRequest::BeginCheckpoint(intent.clone()),
                    )
                    .unwrap(),
                )
                .unwrap();
            let command = Command::ManifestChange(
                ManifestChangePayload::from_prepared(uploader.upload_planned(plan).unwrap(), 0)
                    .unwrap(),
            );
            hs.commit += 1;
            let at = kv9_common::AppliedPosition {
                term: hs.term,
                index: hs.commit,
            };
            storage
                .persist_ready(
                    &[Entry {
                        term: at.term,
                        index: at.index,
                        data: command.encode().into(),
                        ..Default::default()
                    }],
                    Some(&hs),
                )
                .unwrap();
            assert!(matches!(
                state.apply_at(at, &command).unwrap().outcome,
                ApplyOutcome::Manifest(ManifestVerdict::Applied { .. })
            ));
            engine.checkpoint_applied(&manifest).unwrap();
            let kind = if version {
                OwnerKind::Version
            } else {
                OwnerKind::Pending
            };
            let source = intent.binding(kind).unwrap();
            if version {
                ledger
                    .apply(
                        encode_request(
                            root.digest(),
                            &LedgerRequest::Share {
                                from: intent.binding(OwnerKind::Pending).unwrap().token(),
                                to: source.clone(),
                            },
                        )
                        .unwrap(),
                    )
                    .unwrap();
                ledger
                    .apply(
                        encode_request(root.digest(), &LedgerRequest::Publish(source.token()))
                            .unwrap(),
                    )
                    .unwrap();
            }
            guard
                .data_read_fences(&identity)
                .unwrap()
                .advance(kv9_common::data_read_fence::ReadFloor {
                    identity,
                    creation: range.creation,
                    image: crate::checkpoint_retention::data_read_image(&manifest)
                        .unwrap()
                        .unwrap(),
                    generation: 1,
                })
                .unwrap();
            let gets = objects.gets.load(Ordering::SeqCst);
            let old = RetainedImages::new(&uploader, ledger.as_ref(), &root, &[original_owner])
                .unwrap()
                .restore(&original);
            assert!(old.is_err(), "original image bypassed DATA read floor");
            assert_eq!(objects.gets.load(Ordering::SeqCst), gets);
            // Deliberate object-loss control, not production GC authorization.
            let mut removed = 0;
            for file in &original.files {
                if !manifest.files.iter().any(|new| new.key == file.key) {
                    objects
                        .memory
                        .delete(&ObjectKey::new(file.key.clone()).unwrap())
                        .unwrap();
                    removed += 1;
                }
            }
            assert!(removed > 0);
            checkpoint_reader::binding(&source, ledger.lifetime).unwrap()
        })
        .await
        .unwrap();
        let old_reads = s.observed(&s.reader()).unwrap().binding.generation;
        let (mut m, result) = attempt(&s, manager(&s), s.uploader.clone()).await.unwrap();
        result.unwrap();
        assert!(!m
            .recovery_status(s.f.range.region)
            .unwrap()
            .at_open()
            .unwrap()
            .used_original_install_image());
        assert_eq!(s.observed(&expected).unwrap().phase, PinPhase::Released);
        assert_eq!(
            s.observed(&s.reader()).unwrap().binding.generation,
            old_reads
        );
        m.shutdown();
    }
}
