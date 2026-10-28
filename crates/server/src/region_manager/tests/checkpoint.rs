use super::*;
use kv9_common::data_range::DataRange;
use kv9_common::raw_ttl::{ClockSample, ClockWatermark, ExpiryPlan, ExpiryRecord};
use kv9_common::{AppliedPosition, KeyspaceId, TenantId};
use kv9_engine::checkpoint::{FlushScope, RemoteUploader};
use kv9_engine::ReplicatedEngine;
use kv9_raft::command::{RawTtlAction, RawTtlCommand, RawTtlMutation};
use kv9_raft::transport::{InProcHub, RaftTransport};
use kv9_raft::{Command, ManifestChangePayload, RaftGroup};

type Driver = Arc<NodeDriver<DiskRaftStorage, WalEngine>>;
fn pump(drivers: &[Driver]) {
    for _ in 0..30 {
        for driver in drivers {
            driver.step().unwrap();
        }
    }
}
fn commit(drivers: &[Driver], command: Command) -> AppliedPosition {
    let at = drivers[0].propose(&command).unwrap();
    pump(drivers);
    for driver in drivers {
        assert!(driver.driver_applied().unwrap().index >= at.index.0);
    }
    AppliedPosition {
        term: at.term,
        index: at.index.0,
    }
}
fn bytes(path: &Path) -> u64 {
    fs::read_dir(path)
        .unwrap()
        .map(|e| {
            let p = e.unwrap().path();
            let meta = fs::metadata(&p).unwrap();
            if meta.is_dir() {
                bytes(&p)
            } else {
                meta.len()
            }
        })
        .sum()
}

#[test]
fn all_three_group_replicas_recover_ttl_checkpoint_and_tail_after_segment_reclamation() {
    recover_replicas(false);
}

#[test]
fn retained_startup_defers_io_rotates_retries_retires_unopened_groups_and_replays_drains() {
    recover_replicas(true);
}

fn recover_replicas(retained: bool) {
    let f = Fixture::new();
    let extras = retained.then(|| {
        super::retained::initialize_ledger_identity(&f);
        super::retained::extra_creations(&f)
    });
    let root = kv9_meta::root::certified_root(&f.store.begin().unwrap())
        .unwrap()
        .unwrap();
    let region = f.creation.intent().region();
    let range = DataRange {
        root: root.digest(),
        creation: f.creation.intent().digest(),
        region,
        keyspace: KeyspaceId(100),
        tenant: TenantId(1),
        conf_ver: 1,
        version: 1,
        start: vec![],
        end: vec![],
        sealed: false,
    };
    let scope = FlushScope {
        cluster: root.cluster_id.to_string(),
        region: region.0,
        conf_ver: 1,
        version: 1,
    };
    let hub = InProcHub::new();
    let mut managers = Vec::new();
    let mut drivers = Vec::new();
    let mut replicas = Vec::new();
    let mut engines = Vec::new();
    for id in 1..=3 {
        let identity = StoreIdentity::for_voter(&root, NodeId(id)).unwrap();
        let path = f.path.join(format!("replica-{id}"));
        fs::create_dir_all(&path).unwrap();
        let mut manager = RegionManager::new(&path, identity, 2);
        manager.prepare(&f.creation).unwrap();
        assert_eq!(
            manager
                .recovery_status(region)
                .unwrap()
                .selection_relation(),
            RecoverySelectionRelation::NoCompletedOpen
        );
        let LocalGroup::Ready(prepared) = manager.groups.get_mut(&region).unwrap() else {
            panic!("prepared group");
        };
        // Exercise the real durable Active record; use in-process transports
        // for the three actual disk-backed voters rather than binding TCP.
        publish(
            &manager.directory.join(region.0.to_string()),
            &Record {
                phase: Phase::Active,
                node: identity.node_id,
                incarnation: identity.store_incarnation,
                intent: f.creation.intent().clone(),
            },
            &mut |_, _| Ok(()),
        )
        .unwrap();
        prepared.phase = Phase::Active;
        let engine = prepared.engine.clone();
        let storage = prepared.storage.take().unwrap();
        let mut sm = MemStateMachine::with_engine(engine.clone()).unwrap();
        sm.set_data_group(range.root, region, range.creation)
            .unwrap();
        let driver = NodeDriver::new(
            Arc::new(RaftPeer::with_storage(NodeId(id), region, storage).unwrap()),
            Arc::new(hub.endpoint(NodeId(id))) as Arc<dyn RaftTransport>,
            sm,
        )
        .unwrap();
        engines.push(engine);
        drivers.push(driver);
        managers.push(manager);
        replicas.push((path, identity));
    }
    drivers[0].peer().campaign().unwrap();
    pump(&drivers);
    commit(
        &drivers,
        Command::DataRange {
            expected: None,
            next: range.clone(),
        },
    );
    // TTL activation is an explicit fixture injection in this recovery test.
    drivers[0]
        .peer()
        .propose_raw_for_harness(
            Command::RawTtl(
                RawTtlCommand::new(
                    &range,
                    RawTtlAction::Enable {
                        observed: ClockWatermark::default(),
                    },
                )
                .unwrap(),
            )
            .encode(),
        )
        .unwrap();
    pump(&drivers);
    let plan = ExpiryPlan::resolve(
        Some(60),
        ClockSample::with_uncertainty(1000, 10).unwrap(),
        ClockWatermark::default(),
    )
    .unwrap();
    for byte in 0..18 {
        commit(
            &drivers,
            Command::RawTtl(
                RawTtlCommand::new(
                    &range,
                    RawTtlAction::Write {
                        plan,
                        mutations: vec![RawTtlMutation::Put {
                            key: b"bulk".to_vec(),
                            value: vec![byte; 1024 * 1024],
                        }],
                    },
                )
                .unwrap(),
            ),
        );
    }
    let written = commit(
        &drivers,
        Command::RawTtl(
            RawTtlCommand::new(
                &range,
                RawTtlAction::Write {
                    plan,
                    mutations: vec![RawTtlMutation::Put {
                        key: b"bulk".to_vec(),
                        value: b"last TTL value".to_vec(),
                    }],
                },
            )
            .unwrap(),
        ),
    );
    let objects = Arc::new(super::retained::Objects::default());
    let uploader = Arc::new(RemoteUploader::for_harness(objects.clone()));
    let planned = RemoteUploader::plan(engines[0].freeze(scope.clone()).unwrap()).unwrap();
    let manifest = planned.manifest().clone();
    let prepared = uploader.upload_planned(planned).unwrap();
    let published = commit(
        &drivers,
        Command::ManifestChange(ManifestChangePayload::from_prepared(prepared, 0).unwrap()),
    );
    for (i, engine) in engines.iter().enumerate() {
        assert_eq!(
            drivers[i].manifest_at(region.0, 1).unwrap().unwrap().2,
            manifest.encode().unwrap()
        );
        let segments = engine.path().with_extension("segments");
        let before = bytes(&segments);
        assert!(engine.checkpoint_applied(&manifest).unwrap());
        let after = bytes(&segments);
        assert!(
            before > 16 * 1024 * 1024 && after < before / 2,
            "replica {i}: {before} -> {after}"
        );
        println!(
            "replica={} engine_segment_bytes_before={before} engine_segment_bytes_after={after}",
            i + 1
        );
    }
    let tail = commit(
        &drivers,
        Command::RawTtl(
            RawTtlCommand::new(
                &range,
                RawTtlAction::Write {
                    plan: ExpiryPlan::permanent(plan.observed()),
                    mutations: vec![RawTtlMutation::Put {
                        key: b"tail".to_vec(),
                        value: b"newer than image".to_vec(),
                    }],
                },
            )
            .unwrap(),
        ),
    );
    // The actual driver's engine restriction covers leader truncation and
    // confirmed follower compaction, not just the caller's preflight checks.
    for engine in &engines {
        engine.sync_applied_now().unwrap();
    }
    assert!(drivers[0]
        .peer()
        .truncate_retained_log(tail, b"test committed compaction decision")
        .is_err());
    drivers[0]
        .peer()
        .truncate_retained_log(manifest.position(), b"test committed compaction decision")
        .unwrap();
    let final_position = commit(
        &drivers,
        Command::CompactionConfirmed {
            region: region.0,
            floor: manifest.position(),
        },
    );
    for (i, driver) in drivers.iter().enumerate() {
        engines[i].sync_applied_now().unwrap();
        let before = driver.peer().log_file_bytes();
        assert!(driver
            .peer()
            .compact_confirmed_prefix(tail, b"test confirmation above selected cut")
            .is_err());
        driver
            .peer()
            .compact_confirmed_prefix(manifest.position(), b"replicated test confirmation")
            .unwrap();
        assert!(driver.peer().reclaim_log().unwrap());
        let after = driver.peer().log_file_bytes();
        assert!(before > 16 * 1024 * 1024 && after < before / 2);
        println!(
            "replica={} raft_log_bytes_before={before} raft_log_bytes_after={after}",
            i + 1
        );
    }
    for driver in &drivers {
        driver.stop();
    }
    drop(drivers);
    drop(engines);
    drop(managers);
    for (replica, (path, identity)) in replicas.iter().enumerate() {
        let mut recovered = RegionManager::new(path, *identity, 2);
        recovered.uploader = Some(uploader.clone());
        let ledger = extras.as_ref().map(|extras| {
            use crate::checkpoint_retention::CheckpointLedger;
            use kv9_meta::retention::{
                checkpoint_intent::CheckpointIntent, encode_request, LedgerRequest, OwnerKind,
            };
            for creation in extras {
                super::retained::extra_active(&f, path, *identity, creation);
            }
            let ledger = super::retained::Ledger::new(&f, path, *identity);
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
            if replica == 1 {
                let version = intent.binding(OwnerKind::Version).unwrap();
                ledger
                    .apply(
                        encode_request(
                            root.digest(),
                            &LedgerRequest::Share {
                                from: intent.binding(OwnerKind::Pending).unwrap().token(),
                                to: version.clone(),
                            },
                        )
                        .unwrap(),
                    )
                    .unwrap();
                ledger
                    .apply(
                        encode_request(root.digest(), &LedgerRequest::Publish(version.token()))
                            .unwrap(),
                    )
                    .unwrap();
            }
            recovered.recovery_retention = Some(ledger.retention(*identity));
            ledger
        });
        let gets = objects.gets.load(std::sync::atomic::Ordering::SeqCst);
        recovered.recover(&f.store).unwrap();
        if let (Some(ledger), Some(extras)) = (&ledger, &extras) {
            use kv9_common::retention::PinPhase;
            use kv9_meta::retention::{
                checkpoint_intent::CheckpointIntent, checkpoint_reader, OwnerKind,
            };
            use std::sync::atomic::Ordering::SeqCst;
            assert!(matches!(
                recovered.groups[&region],
                LocalGroup::Recovering(_)
            ));
            assert!(recovered.recovery_status(region).is_err());
            assert_eq!(objects.gets.load(SeqCst), gets);
            assert_eq!(
                ledger.reads.load(SeqCst),
                0,
                "bootstrap must not wait for metadata"
            );
            recovered
                .retire_published_parent(extras[1].intent().region())
                .unwrap();
            assert!(matches!(
                recovered.groups[&extras[1].intent().region()],
                LocalGroup::Retired { draining: None }
            ));
            assert_eq!(ledger.reads.load(SeqCst), 0);
            ledger.refuse.store(true, SeqCst);
            recovered.resume_recovery_one();
            assert!(matches!(
                recovered.groups[&region],
                LocalGroup::Recovering(_)
            ));
            assert_eq!(objects.gets.load(SeqCst), gets);
            recovered.resume_recovery_one();
            assert!(
                matches!(
                    recovered.groups[&extras[0].intent().region()],
                    LocalGroup::Ready(_)
                ),
                "a refused Reader must not starve another group"
            );
            ledger.refuse.store(false, SeqCst);
            ledger.finish_fault.store(true, SeqCst);
            recovered.resume_recovery_one();
            assert!(matches!(
                recovered.groups[&region],
                LocalGroup::Recovering(_)
            ));
            assert!(ledger.outbox.has_pending());
            let gets = objects.gets.load(SeqCst);
            recovered.resume_recovery_one();
            assert_eq!(
                objects.gets.load(SeqCst),
                gets,
                "old drain must be replayed before another GET"
            );
            ledger.finish_fault.store(false, SeqCst);
            assert!(ledger.outbox.replay_one(ledger.as_ref()).unwrap());
            recovered.resume_recovery_one();
            assert!(!ledger.outbox.has_pending());
            let kind = if replica == 0 {
                OwnerKind::Pending
            } else {
                OwnerKind::Version
            };
            let source = CheckpointIntent::new(&root, &manifest, 0)
                .unwrap()
                .binding(kind)
                .unwrap();
            let reader = checkpoint_reader::binding(&source, ledger.lifetime).unwrap();
            let observed = kv9_meta::retention::retention_owner(
                f.store.engine().snapshot().unwrap().as_ref(),
                &root,
                reader.descriptor.id,
            )
            .unwrap()
            .unwrap();
            assert_eq!(observed.phase, PinPhase::Released);
            assert_eq!(observed.binding.generation.get(), 2);
        }
        let Some(LocalGroup::Ready(local)) = recovered.groups.get(&region) else {
            panic!("{}", recovered.status_json());
        };
        assert_eq!(local.phase, Phase::Active);
        let status = recovered.recovery_status(region).unwrap();
        assert_eq!(status.store(), *identity);
        assert!(!status.driver_started());
        assert_eq!(
            status.selection_relation(),
            RecoverySelectionRelation::SameImageAsOpen
        );
        let facts = status.at_open().unwrap();
        assert!(facts.installed().is_none());
        assert_eq!(
            facts.recovered_through(),
            DurableAppliedPosition::AppliedThrough(final_position)
        );
        assert_eq!(
            facts.selected().unwrap().publication().unwrap().at(),
            published
        );
        assert_eq!(facts.selected().unwrap().cut(), manifest.position());
        assert!(
            local.driver.is_none(),
            "recovery alone must not authorize serving/voting"
        );
        let key =
            kv9_common::codec::encode_key(kv9_common::codec::KeyMode::Raw, range.keyspace, b"bulk")
                .unwrap();
        assert_eq!(
            local.engine.get(ColumnFamily::Default, &key).unwrap(),
            Some(b"last TTL value".to_vec())
        );
        assert_eq!(
            local
                .engine
                .get(ColumnFamily::Write, &key)
                .unwrap()
                .unwrap(),
            ExpiryRecord::new(range.creation, written, 61010)
                .unwrap()
                .encode()
        );
        let key =
            kv9_common::codec::encode_key(kv9_common::codec::KeyMode::Raw, range.keyspace, b"tail")
                .unwrap();
        assert_eq!(
            local.engine.get(ColumnFamily::Default, &key).unwrap(),
            Some(b"newer than image".to_vec())
        );
        assert_eq!(
            local.engine.applied_position().unwrap(),
            DurableAppliedPosition::AppliedThrough(final_position)
        );
        // A locally selected successor must not inherit the old successful-open
        // observation, even before anyone has validated its publication history.
        let successor = uploader
            .upload(local.engine.freeze(scope.clone()).unwrap())
            .unwrap()
            .into_manifest();
        local.engine.checkpoint_applied(&successor).unwrap();
        let changed = recovered.recovery_status(region).unwrap();
        assert_eq!(
            changed.selection_relation(),
            RecoverySelectionRelation::ChangedSinceOpen
        );
        assert_eq!(changed.at_open().unwrap().selected(), facts.selected());
        assert_eq!(
            changed.selected(),
            Some((
                successor.position(),
                RootDigest::sha256(&successor.encode().unwrap())
            ))
        );
        let json: serde_json::Value = serde_json::from_str(&recovered.status_json()).unwrap();
        assert_eq!(
            json[0]["recovery_at_open"]["selected"]["cut"]["index"],
            manifest.index
        );
        assert!(local
            .engine
            .checkpoint_history()
            .unwrap()
            .with_compaction(successor.position(), || Err::<(), _>(Error::Engine(
                "test uncertain compaction".into()
            )))
            .is_err());
        assert!(recovered.recovery_status(region).is_err());
    }
    // Missing remote access quarantines the replica; the surviving local tail
    // is never mistaken for a complete engine after prefix reclamation.
    let mut unavailable = RegionManager::new(&replicas[0].0, replicas[0].1, 2);
    unavailable.recover(&f.store).unwrap();
    assert!(matches!(
        unavailable.groups.get(&region),
        Some(LocalGroup::Failed(_))
    ));
    assert!(unavailable.recovery_status(region).is_err());
}
