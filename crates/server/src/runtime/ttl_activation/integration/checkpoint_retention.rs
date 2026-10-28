//! Real metadata/data Raft groups, distinct leaders and authenticated duplex
//! HTTP/2. This drives the publication steps explicitly, not an auto scheduler.
use super::*;
mod catalog_admission;
use crate::checkpoint_retention::{CheckpointLedger, CheckpointOwners};
use crate::runtime::checkpoint_retention::RoutedLedger;
use kv9_common::retention::PinPhase;
use kv9_engine::checkpoint::{FlushJournal, FlushScope, RemoteUploader};
use kv9_engine::{MemoryObjectStore, ObjectStore};
use kv9_meta::retention::{decode_owner_observation, encode_request, LedgerRequest};
use kv9_raft::grpc::checkpoint_retention::{on_channel, LedgerQuery, LedgerReply};
use kv9_region::manifest::{ManifestAttempt, ManifestProposalState, ManifestSeam, SettledManifest};

fn ledger(f: &Fixture) -> RoutedLedger {
    RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current())
}
fn root(f: &Fixture) -> RootDescriptor {
    f.backends[0].ttl_context().unwrap().root.clone()
}
fn phase(ledger: &RoutedLedger, root: RootDigest, id: kv9_common::retention::OwnerId) -> PinPhase {
    decode_owner_observation(&ledger.owner(root, id).unwrap().unwrap(), root, id)
        .unwrap()
        .phase
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn old_losing_pending_settles_from_exact_history_without_releasing_its_global_pin() {
    use kv9_raft::driver::ApplyWaitOutcome;
    use kv9_raft::{Command, ManifestChangePayload, ManifestVerdict};
    let mut f = Fixture::new().await;
    assert!(matches!(
        f.client(0).activate().await.outcome,
        TtlOutcome::Success { .. }
    ));
    assert!(matches!(
        f.client(0)
            .call(Operation::Put {
                pairs: vec![(b"old-pending".to_vec(), b"ttl-data".to_vec())],
                ttl_secs: Some(60),
            })
            .await
            .outcome,
        TtlOutcome::Success { .. }
    ));
    let root = root(&f);
    let range = f.range.clone();
    let engine = f.backends[0]
        .raw_directory
        .by_region(range.region)
        .unwrap()
        .capture_parts()
        .1
        .clone();
    let driver = f.data[0].clone();
    let ledger = ledger(&f);
    let path = f.path.join("losing.pending");
    let objects = Arc::new(MemoryObjectStore::new());
    let uploader = RemoteUploader::for_harness(objects.clone());
    let (root, original, pending_id) = tokio::task::spawn_blocking(move || {
        let scope = FlushScope {
            cluster: root.cluster_id.to_string(),
            region: range.region.0,
            conf_ver: range.conf_ver,
            version: range.version,
        };
        let seam = ManifestSeam::mint(driver.mint_seam_handle().unwrap());
        let (frozen, mut candidate) = engine
            .freeze_checkpoint_candidate(|_| Ok(scope.clone()))
            .unwrap();
        let plan = uploader.plan_checkpoint(frozen).unwrap();
        let original = plan.manifest().clone();
        let owners = CheckpointOwners::new(&root, &original, 0).unwrap();
        let mut journal = FlushJournal::new(&path);
        candidate.stage(&mut journal, &plan, 0).unwrap();
        drop(plan);
        owners.before_io(&ledger).unwrap();
        let (prepared, predecessor) = journal.load().unwrap().unwrap().recover(&uploader).unwrap();
        journal.stage(&prepared, predecessor).unwrap();
        let old = ManifestAttempt::from_prepared(prepared, predecessor).unwrap();
        // Establish a different first winner, then move far beyond its window.
        // Every descriptor is uploaded and published through real data Raft;
        // object owners use the distinct metadata leader through duplex HTTP/2.
        for generation in 0..9 {
            let observe = Command::RawTtl(
                kv9_raft::command::RawTtlCommand::new(
                    &range,
                    kv9_raft::command::RawTtlAction::Observe {
                        observed: kv9_common::raw_ttl::ClockSample::with_uncertainty(
                            2000 + generation,
                            0,
                        )
                        .unwrap()
                        .advance(kv9_common::raw_ttl::ClockWatermark::default())
                        .unwrap(),
                    },
                )
                .unwrap(),
            );
            let at = driver.propose(&observe).unwrap();
            driver.wait_applied(at, Duration::from_secs(3)).unwrap();
            let plan = uploader
                .plan_checkpoint(engine.freeze(scope.clone()).unwrap())
                .unwrap();
            let manifest = plan.manifest().clone();
            let winner_owners = CheckpointOwners::new(&root, &manifest, generation).unwrap();
            let mut winner_journal =
                FlushJournal::new(path.with_extension(format!("winner-{generation}")));
            winner_journal.stage_planned(&plan, generation).unwrap();
            drop(plan);
            winner_owners.before_io(&ledger).unwrap();
            let (prepared, expected) = winner_journal
                .load()
                .unwrap()
                .unwrap()
                .recover(&uploader)
                .unwrap();
            winner_journal.stage(&prepared, expected).unwrap();
            let command = Command::ManifestChange(
                ManifestChangePayload::from_prepared(prepared, expected).unwrap(),
            );
            let at = driver.propose(&command).unwrap();
            assert!(matches!(
                driver.wait_applied(at, Duration::from_secs(3)).unwrap(),
                ApplyWaitOutcome::Manifest {
                    verdict: ManifestVerdict::Applied { .. },
                    ..
                }
            ));
            winner_owners.after_positive_settlement(&ledger).unwrap();
            winner_journal.clear_settled().unwrap();
        }
        assert_eq!(driver.manifest_pair(range.region.0).unwrap().generation, 9);
        assert_eq!(journal.load().unwrap().unwrap().manifest(), &original);
        let state = seam.resume_in_flight(old, Duration::from_secs(3)).unwrap();
        let ManifestProposalState::Settled(SettledManifest::WindowRefused { winner_change_id }) =
            state
        else {
            panic!("actual exact-window history must settle the losing attempt: {state:?}");
        };
        let (_, _, winner) = driver.manifest_at(range.region.0, 1).unwrap().unwrap();
        assert_eq!(
            winner_change_id,
            kv9_engine::checkpoint::CheckpointManifest::decode(&winner)
                .unwrap()
                .change_id(0)
                .unwrap()
        );
        assert!(!seam.in_flight(range.region.0));
        candidate.finish(&mut journal).unwrap();
        assert!(journal.load().unwrap().is_none());
        let archived = ledger.intent(&root, owners.pending_id()).unwrap().unwrap();
        assert_eq!(archived.manifest_bytes(), original.encode().unwrap());
        assert_eq!(archived.predecessor(), 0);
        assert_eq!(
            phase(&ledger, root.digest(), owners.pending_id()),
            PinPhase::Published,
            "a local negative does not prove every uploader has drained"
        );
        assert!(ledger
            .owner(root.digest(), owners.version_id())
            .unwrap()
            .is_none());
        for file in &original.files {
            assert!(objects
                .get(&kv9_engine::ObjectKey::new(file.key.clone()).unwrap())
                .unwrap()
                .is_some());
        }
        (root, original, owners.pending_id())
    })
    .await
    .unwrap();
    for server in f.servers.drain(..) {
        server.abort();
        let _ = server.await;
    }
    for driver in f.meta.iter().chain(f.data.iter()) {
        driver.stop();
    }
    for thread in f.owners.drain(..) {
        thread.join().unwrap();
    }
    f.backends.clear();
    f.meta.clear();
    f.data.clear();
    f.hooks.clear();
    let (metadata, _) = WalEngine::open(f.path.join("n2/meta.wal")).unwrap();
    let view = metadata.snapshot().unwrap();
    let archived =
        kv9_meta::retention::checkpoint_intent::checkpoint_intent(view.as_ref(), &root, pending_id)
            .unwrap()
            .unwrap();
    assert_eq!(archived.manifest_bytes(), original.encode().unwrap());
    assert_eq!(archived.predecessor(), 0);
    assert_eq!(
        kv9_meta::retention::retention_owner(view.as_ref(), &root, pending_id)
            .unwrap()
            .unwrap()
            .phase,
        PinPhase::Published
    );
    drop(view);
    drop(metadata);
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn data_checkpoint_remote_owners_resume_all_seven_lost_receipt_prefixes_and_recover() {
    let mut f = Fixture::new().await;
    assert_eq!(f.meta[1].status().role, Role::Leader);
    assert_eq!(f.data[0].status().role, Role::Leader);
    assert!(matches!(
        f.client(0).activate().await.outcome,
        TtlOutcome::Success { .. }
    ));
    let root = root(&f);
    let objects = Arc::new(MemoryObjectStore::new());
    let uploader = Arc::new(RemoteUploader::for_harness(objects.clone()));
    let seam = Arc::new(ManifestSeam::mint(f.data[0].mint_seam_handle().unwrap()));
    let engine = f.backends[0]
        .raw_directory
        .by_region(f.range.region)
        .unwrap()
        .capture_parts()
        .1
        .clone();
    let mut last_manifest = None;
    for lost_after in 1..=7 {
        assert!(matches!(
            f.client(0)
                .call(Operation::Put {
                    pairs: vec![(b"k".to_vec(), vec![lost_after as u8])],
                    ttl_secs: Some(2)
                })
                .await
                .outcome,
            TtlOutcome::Success { .. }
        ));
        let root = root.clone();
        let range = f.range.clone();
        let engine = engine.clone();
        let backend = f.backends[0].clone();
        let hook = f.hooks[1].clone();
        let seam = seam.clone();
        let uploader = uploader.clone();
        let objects = objects.clone();
        let driver = f.data[0].clone();
        let path = f.path.join("data.pending");
        let ledger = ledger(&f);
        let runtime = tokio::runtime::Handle::current();
        let manifest = tokio::task::spawn_blocking(move || {
            hook.retention_calls.store(0, Ordering::SeqCst);
            hook.retention_commits.store(0, Ordering::SeqCst);
            hook.retention_fail_after
                .store(lost_after, Ordering::SeqCst);
            let generation = driver.manifest_pair(range.region.0).unwrap().generation;
            driver.read_barrier(Duration::from_secs(2)).unwrap();
            let frozen = engine
                .freeze_with_scope(|view| {
                    let actual = DataRange::decode(
                        &view
                            .get(ColumnFamily::Default, kv9_common::data_range::RANGE_KEY)?
                            .unwrap(),
                    )?;
                    assert_eq!(actual, range);
                    Ok(FlushScope {
                        cluster: root.cluster_id.to_string(),
                        region: actual.region.0,
                        conf_ver: actual.conf_ver,
                        version: actual.version,
                    })
                })
                .unwrap();
            let plan = RemoteUploader::plan(frozen).unwrap();
            let manifest = plan.manifest().clone();
            let owners = CheckpointOwners::new(&root, &manifest, generation).unwrap();
            let mut journal = FlushJournal::new(&path);
            journal.stage_planned(&plan, generation).unwrap();
            let before = objects.list("").unwrap().len();
            let owned = owners.before_io(&ledger);
            if lost_after == 1 {
                assert!(owned.is_err());
                let view = hook
                    .backend
                    .node
                    .meta_raft
                    .store
                    .engine()
                    .snapshot()
                    .unwrap();
                assert!(kv9_meta::retention::checkpoint_begin::atomic_birth(
                    view.as_ref(),
                    &root,
                    owners.pending_id()
                )
                .unwrap()
                .is_some());
                assert_eq!(
                    kv9_meta::retention::retention_owner(view.as_ref(), &root, owners.pending_id())
                        .unwrap()
                        .unwrap()
                        .phase,
                    PinPhase::Published
                );
                drop(view);
                assert_eq!(
                    hook.retention_calls.load(Ordering::SeqCst),
                    lost_after,
                    "no same-call retry after uncertain mutation"
                );
                assert_eq!(
                    objects.list("").unwrap().len(),
                    before,
                    "no PUT before ownership confirmed"
                );
                assert_eq!(journal.load().unwrap().unwrap().manifest(), &manifest);
                // Recreate the local caller and reload the actual durable plan.
                // The next pass reconciles the same identity, never mints a new cut.
                let restarted = RoutedLedger::new(&backend, runtime.clone());
                owners.before_io(&restarted).unwrap();
            } else {
                owned.unwrap();
            }
            assert_eq!(
                phase(&ledger, root.digest(), owners.pending_id()),
                PinPhase::Published
            );
            let intent = ledger.intent(&root, owners.pending_id()).unwrap().unwrap();
            assert_eq!(intent.manifest_bytes(), manifest.encode().unwrap());
            assert_eq!(intent.predecessor(), generation);
            let pending = FlushJournal::new(&path).load().unwrap().unwrap();
            assert_eq!(pending.manifest(), &manifest);
            assert_eq!(pending.expected_generation(), generation);
            let (prepared, predecessor) = pending.recover(&uploader).unwrap();
            journal.stage(&prepared, predecessor).unwrap();
            let state = seam
                .resume_in_flight(
                    ManifestAttempt::from_prepared(prepared, predecessor).unwrap(),
                    Duration::from_secs(3),
                )
                .unwrap();
            assert!(
                matches!(
                    state,
                    ManifestProposalState::Settled(
                        SettledManifest::MyChangeApplied { .. }
                            | SettledManifest::AlreadyApplied { .. }
                            | SettledManifest::EffectSettled
                    )
                ),
                "{state:?}"
            );
            let handoff = owners.after_positive_settlement(&ledger);
            if lost_after >= 2 {
                assert!(handoff.is_err());
                assert_eq!(hook.retention_calls.load(Ordering::SeqCst), lost_after);
                assert_eq!(journal.load().unwrap().unwrap().manifest(), &manifest);
                let restarted = RoutedLedger::new(&backend, runtime);
                owners.before_io(&restarted).unwrap();
                owners.after_positive_settlement(&restarted).unwrap();
            } else {
                handoff.unwrap();
            }
            assert_eq!(
                phase(&ledger, root.digest(), owners.pending_id()),
                PinPhase::Released
            );
            assert_eq!(
                phase(&ledger, root.digest(), owners.version_id()),
                PinPhase::Published
            );
            assert!(ledger
                .executors(root.digest(), owners.pending_id(), None, 1)
                .unwrap()
                .admission
                .is_closed());
            let delayed = CheckpointOwners::new(&root, &manifest, generation)
                .unwrap()
                .execution(&ledger)
                .unwrap();
            assert!(
                delayed.before_io(&ledger).is_err(),
                "a settled attempt cannot reopen uploads"
            );
            assert!(
                ledger
                    .apply(
                        encode_request(root.digest(), &LedgerRequest::RecordCheckpoint(intent))
                            .unwrap()
                    )
                    .is_err(),
                "legacy final authorization must not join an atomically born attempt"
            );
            // Only actual positive publication and completed owner handoff reach adoption.
            assert!(engine.checkpoint_applied(&manifest).unwrap());
            journal.clear_settled().unwrap();
            manifest
        })
        .await
        .unwrap();
        assert_eq!(f.hooks[0].retention_calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            f.hooks[2].retention_calls.load(Ordering::SeqCst),
            0,
            "unknown remote mutation never walks to another candidate"
        );
        last_manifest = Some(manifest);
    }
    assert_eq!(
        f.data[0]
            .manifest_pair(f.range.region.0)
            .unwrap()
            .generation,
        7
    );
    let manifest = last_manifest.unwrap();
    let final_owners = CheckpointOwners::new(&root, &manifest, 6).unwrap();
    // Stop owners and drop live engine/driver references before actual file reopen.
    for driver in f.meta.iter().chain(f.data.iter()) {
        driver.stop();
    }
    for thread in f.owners.drain(..) {
        thread.join().unwrap();
    }
    let path = f.path.clone();
    let range = f.range.clone();
    for server in f.servers.drain(..) {
        server.abort();
        let _ = server.await;
    }
    drop(seam);
    drop(engine);
    f.backends.clear();
    f.data.clear();
    f.meta.clear();
    f.hooks.clear();
    let (mut storage, _) = DiskRaftStorage::open(&path.join("n1/data-raft"), &[1, 2, 3]).unwrap();
    let (reopened, _, observed) = kv9_raft::state_machine::checkpoint_recovery::data_group::open(
        &mut storage,
        path.join("n1/data.wal"),
        Some(&uploader),
        kv9_raft::state_machine::checkpoint_recovery::data_group::DataCheckpointIdentity {
            cluster: root.cluster_id,
            root: root.digest(),
            region: range.region,
            creation: range.creation,
        },
    )
    .unwrap();
    assert_eq!(observed.unwrap().range(), Some(&range));
    assert_eq!(
        WalEngine::checkpoint_reference(path.join("n1/data.wal")).unwrap(),
        Some(manifest.clone())
    );
    let key = kv9_common::codec::encode_key(kv9_common::codec::KeyMode::Raw, range.keyspace, b"k")
        .unwrap();
    assert_eq!(
        reopened.get(ColumnFamily::Default, &key).unwrap(),
        Some(vec![7])
    );
    assert!(reopened.get(ColumnFamily::Write, &key).unwrap().is_some());
    assert!(reopened
        .get(ColumnFamily::Default, CLOCK_KEY)
        .unwrap()
        .is_some());
    drop(reopened);
    drop(storage);
    let (metadata, _) = WalEngine::open(path.join("n2/meta.wal")).unwrap();
    let view = metadata.snapshot().unwrap();
    let pending =
        kv9_meta::retention::retention_owner(view.as_ref(), &root, final_owners.pending_id())
            .unwrap()
            .unwrap();
    let version =
        kv9_meta::retention::retention_owner(view.as_ref(), &root, final_owners.version_id())
            .unwrap()
            .unwrap();
    let intent = kv9_meta::retention::checkpoint_intent::checkpoint_intent(
        view.as_ref(),
        &root,
        final_owners.pending_id(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(intent.manifest_bytes(), manifest.encode().unwrap());
    assert_eq!(intent.predecessor(), 6);
    assert!(kv9_meta::retention::checkpoint_begin::atomic_birth(
        view.as_ref(),
        &root,
        final_owners.pending_id()
    )
    .unwrap()
    .is_some());
    assert!(kv9_meta::retention::checkpoint_admission::executors(
        view.as_ref(),
        &root,
        final_owners.pending_id(),
        None,
        1
    )
    .unwrap()
    .admission
    .is_closed());
    assert_eq!(pending.phase, PinPhase::Released);
    assert_eq!(version.phase, PinPhase::Published);
    assert_eq!(pending.binding.resources, version.binding.resources);
    assert_eq!(
        pending.binding.descriptor.subject,
        version.binding.descriptor.subject
    );
    assert_eq!(pending.binding.descriptor.region, range.region.0);
    assert_eq!(pending.binding.generation.get(), 1);
    drop(view);
    drop(metadata);
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn checkpoint_ledger_routes_only_typed_leader_refusal_and_validates_node_auth() {
    let f = Fixture::new().await;
    let root = root(&f);
    let query =
        LedgerQuery::Apply(encode_request(root.digest(), &LedgerRequest::Initialize).unwrap());
    for (caller, token) in [
        (NodeId(0), "cluster-test"),
        (NodeId(99), "cluster-test"),
        (NodeId(1), "wrong-token"),
    ] {
        assert!(on_channel(
            f.public[1].clone(),
            root.digest(),
            &query,
            caller,
            Some(token.into()),
            Duration::from_secs(1)
        )
        .await
        .is_err());
    }
    assert_eq!(f.hooks[1].retention_calls.load(Ordering::SeqCst), 0);
    let refusal = on_channel(
        f.public[2].clone(),
        root.digest(),
        &query,
        NodeId(1),
        Some("cluster-test".into()),
        Duration::from_secs(1),
    )
    .await;
    assert!(matches!(
        refusal,
        Err(Error::NotLeader {
            leader: Some(NodeId(2))
        })
    ));
    // Stale routing: the first address reaches another admitted follower; the
    // next address reaches the actual leader. Root+receipt, not address, certify.
    let context = f.backends[0].ttl_context().unwrap();
    context
        .channels
        .lock()
        .unwrap()
        .insert(root.voters[1].addr, f.public[2].clone());
    context
        .channels
        .lock()
        .unwrap()
        .insert(root.voters[2].addr, f.public[1].clone());
    let ledger = ledger(&f);
    tokio::task::spawn_blocking(move || {
        ledger.apply(encode_request(root.digest(), &LedgerRequest::Initialize).unwrap())
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(f.hooks[2].retention_calls.load(Ordering::SeqCst), 2);
    assert_eq!(f.hooks[1].retention_commits.load(Ordering::SeqCst), 1);
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn checkpoint_ledger_busy_deadline_and_cross_root_do_not_append() {
    let f = Fixture::new().await;
    let root = root(&f);
    let ledger = ledger(&f);
    let backend = f.backends[1].clone();
    tokio::task::spawn_blocking(move || {
        let command = encode_request(root.digest(), &LedgerRequest::Initialize).unwrap();
        let before = backend.driver.driver_applied();
        assert!(backend
            .checkpoint_retention(
                root.digest(),
                &LedgerQuery::Apply(command.clone()),
                Instant::now()
            )
            .is_err());
        assert!(backend
            .checkpoint_retention(
                RootDigest::from_bytes([77; 32]),
                &LedgerQuery::Apply(command.clone()),
                Instant::now() + Duration::from_secs(1)
            )
            .is_err());
        let _busy = backend.node.meta_raft.lock_catalog_txn();
        let start = Instant::now();
        assert!(ledger.apply(command).is_err());
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "catalog wait must stay below the full RPC budget"
        );
        assert_eq!(backend.driver.driver_applied(), before);
    })
    .await
    .unwrap();
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn checkpoint_ledger_duplicate_confirmation_is_new_receipt_with_same_revision() {
    let f = Fixture::new().await;
    let root = root(&f);
    let query =
        LedgerQuery::Apply(encode_request(root.digest(), &LedgerRequest::Initialize).unwrap());
    let mut previous = None;
    for changed in [true, false] {
        let response = on_channel(
            f.public[1].clone(),
            root.digest(),
            &query,
            NodeId(1),
            Some("cluster-test".into()),
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        let LedgerReply::Applied {
            revision,
            changed: actual,
            applied,
        } = response
        else {
            panic!("wrong reply")
        };
        assert_eq!(actual, changed);
        assert_eq!(revision, 0);
        if let Some(old) = previous {
            assert!(applied.index > old);
        }
        previous = Some(applied.index);
    }
    let before = f.meta[1].driver_applied().unwrap();
    let local = RoutedLedger::new(&f.backends[1], tokio::runtime::Handle::current());
    let digest = root.digest();
    tokio::task::spawn_blocking(move || {
        local
            .apply(encode_request(digest, &LedgerRequest::Initialize).unwrap())
            .unwrap();
        assert!(local
            .owner(
                digest,
                kv9_common::retention::OwnerId::new([33; 16]).unwrap()
            )
            .unwrap()
            .is_none());
    })
    .await
    .unwrap();
    assert!(f.meta[1].driver_applied().unwrap().index > before.index);
    assert_eq!(
        f.hooks[1].retention_calls.load(Ordering::SeqCst),
        2,
        "local leader uses the same core without a loopback RPC"
    );
    f.close().await;
}
