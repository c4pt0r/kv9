//! Full runtime TTL split on actual data/catalog Raft, V8 RPC confirmations,
//! a resumed child leader, empty-child clock inheritance and metadata publication.
use super::*;
use kv9_common::codec::{encode_key, KeyMode};
use kv9_common::data_range::RANGE_KEY;
use kv9_common::raw_ttl::split::{ImportState, IMPORT_KEY, SEAL_KEY};
use kv9_meta::data_groups::split::committed_split_plan;
use kv9_raft::state_machine::checkpoint_recovery::data_group::{self, DataCheckpointIdentity};

async fn applied(drivers: &[Driver], at: kv9_raft::ProposedAt) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while drivers
            .iter()
            .any(|d| d.status().applied_index < at.index.0)
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f).await.unwrap()
}

async fn split_fixture() -> (
    Fixture,
    Vec<kv9_meta::data_groups::CreationIntent>,
    Vec<Vec<Driver>>,
) {
    split_fixture_with_tick(Duration::from_millis(5)).await
}
async fn split_fixture_with_tick(
    tick: Duration,
) -> (
    Fixture,
    Vec<kv9_meta::data_groups::CreationIntent>,
    Vec<Vec<Driver>>,
) {
    let mut f = Fixture::new().await;
    let root = f.range.root;
    let mut txn = f.backends[1].node.meta_raft.store.begin().unwrap();
    let mut creations = Vec::new();
    for operation in [61, 62] {
        let creation = kv9_meta::data_groups::plan_empty_group(
            &mut txn,
            [operation; 16],
            &[NodeId(1), NodeId(2), NodeId(3)],
        )
        .unwrap();
        kv9_meta::data_groups::activation::plan_activation(&mut txn, &creation).unwrap();
        creations.push(creation);
    }
    let at = f.meta[1]
        .propose(&Command::from_batch(&txn.into_batch()))
        .unwrap();
    applied(&f.meta, at).await;
    let intent = f.backends[1]
        .record_split_intent(
            "test",
            root,
            [63; 16],
            f.range.region,
            b"m",
            creations[0].task(),
            creations[1].task(),
        )
        .unwrap();
    applied(
        &f.meta,
        kv9_raft::ProposedAt {
            term: intent.applied.term,
            index: kv9_raft::LogIndex(intent.applied.index),
        },
    )
    .await;

    for i in 0..3 {
        let engine = f.backends[i]
            .raw_directory
            .by_region(f.range.region)
            .unwrap()
            .capture_parts()
            .1
            .clone();
        f.backends[i]
            .group_handles
            .lock()
            .unwrap()
            .insert(f.range.region, (engine, f.data[i].clone()));
    }
    let mut children = Vec::new();
    for (half, creation) in creations.iter().enumerate() {
        let hub = InProcHub::new();
        let mut drivers = Vec::new();
        for i in 0..3 {
            let path = f
                .path
                .join(format!("n{}/split-{}", i + 1, creation.region().0));
            let engine = Arc::new(WalEngine::open(path.with_extension("wal")).unwrap().0);
            engine.enable_segmentation().unwrap();
            let storage = DiskRaftStorage::open(&path, &[1, 2, 3]).unwrap().0;
            let peer = Arc::new(
                RaftPeer::with_storage(NodeId(i as u64 + 1), creation.region(), storage).unwrap(),
            );
            let mut sm = MemStateMachine::with_engine(engine.clone()).unwrap();
            sm.set_data_group(root, creation.region(), creation.digest())
                .unwrap();
            let driver = NodeDriver::new(
                peer,
                Arc::new(hub.endpoint(NodeId(i as u64 + 1))) as Arc<dyn RaftTransport>,
                sm,
            )
            .unwrap();
            f.backends[i]
                .group_handles
                .lock()
                .unwrap()
                .insert(creation.region(), (engine, driver.clone()));
            drivers.push(driver);
        }
        drivers[if half == 0 { 0 } else { 2 }]
            .peer()
            .campaign()
            .unwrap();
        pump(&drivers);
        assert_eq!(
            drivers[if half == 0 { 0 } else { 2 }].status().role,
            Role::Leader
        );
        for driver in &drivers {
            f.owners.push(driver.spawn(tick).unwrap());
            f.data.push(driver.clone());
        }
        children.push(drivers);
    }

    (f, creations, children)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn runtime_ttl_split_requires_all_stores_resumes_copy_and_publishes_original_expiry() {
    split_resume(false, false, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn staging_checkpoint_reclaims_actual_import_wal_and_recovers_three_children_before_finish() {
    split_resume(true, false, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn import_compaction_reclaims_raft_and_recovers_three_unbound_children_before_finish() {
    split_resume(true, true, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn large_import_image_reclaims_and_recovers_three_unbound_children_with_v7() {
    split_resume(true, true, true).await;
}

async fn compact_import(f: &Fixture, children: &[Driver], region: RegionId) {
    use crate::runtime::group_compaction::{self, Thresholds};
    let before = children
        .iter()
        .map(|d| d.peer().log_file_bytes())
        .collect::<Vec<_>>();
    assert!(before.iter().all(|n| *n > 16 * 1024 * 1024));
    let thresholds = Thresholds {
        entries: 0,
        bytes: 8 * 1024 * 1024,
        age: Duration::ZERO,
    };
    let disabled = Thresholds {
        bytes: 0,
        ..thresholds
    };
    let backend = f.backends[1].clone();
    let follower = f.backends[0].clone();
    let decision = blocking(move || {
        let mut cursor = Some(region.0 - 1);
        let mut marks = BTreeMap::new();
        assert!(group_compaction::plan(&backend, disabled, &mut cursor, &mut marks)?.is_none());
        assert!(group_compaction::plan(&follower, thresholds, &mut cursor, &mut marks)?.is_none());
        assert!(marks.is_empty());
        group_compaction::plan(&backend, thresholds, &mut cursor, &mut marks)
    })
    .await
    .unwrap()
    .expect("actual auto-compaction decision");
    assert_eq!(decision.decision.region(), region);
    let floor = decision.decision.floor();
    applied(
        &f.meta,
        kv9_raft::ProposedAt {
            term: decision.applied.term,
            index: kv9_raft::LogIndex(decision.applied.index),
        },
    )
    .await;
    for (i, child) in children.iter().enumerate() {
        let engine = f.backends[i].group_handles.lock().unwrap()[&region]
            .0
            .clone();
        let selected = WalEngine::checkpoint_reference(engine.path())
            .unwrap()
            .unwrap();
        assert!(floor.index <= selected.index);
        // Even a purported confirmation cannot erase the selected image's
        // publication/configuration evidence. Refusal must leave storage intact.
        let status = child.status();
        assert!(status.applied_index > selected.index);
        let error = child
            .peer()
            .compact_confirmed_prefix(
                AppliedPosition {
                    term: status.applied_term,
                    index: status.applied_index,
                },
                b"negative test: beyond selected checkpoint",
            )
            .unwrap_err();
        assert!(error.to_string().contains("selected or pending"), "{error}");
        assert_eq!(child.status().log_first_index, 1);
    }
    // Catalog intent alone does not allow followers to truncate.
    let backend = f.backends[2].clone();
    blocking(move || group_compaction::execute(&backend, &mut None))
        .await
        .unwrap();
    assert_eq!(children[2].status().log_first_index, 1);
    // A newly committed, unreachable learner has not matched the floor. The
    // real planner decision and selected images must not bypass that gate.
    let child = children[0].clone();
    let added = blocking(move || {
        let at = child.add_learner(NodeId(4))?;
        child.wait_conf_applied(at, Duration::from_secs(5))?;
        Ok(at)
    })
    .await
    .unwrap();
    until_split(|| {
        children.iter().all(|d| {
            d.peer().status_snapshot().conf_applied >= added.index.0
                && d.status().learners.contains(&4)
        })
    })
    .await;
    let backend = f.backends[0].clone();
    let error = blocking(move || group_compaction::execute(&backend, &mut None))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("have not matched"), "{error}");
    assert!(children.iter().all(|d| d.status().log_first_index == 1));
    assert!(children[0]
        .get(
            ColumnFamily::Default,
            kv9_common::data_range::COMPACTION_CONFIRMED_KEY
        )
        .unwrap()
        .is_none());
    let child = children[0].clone();
    let removed = blocking(move || {
        let at = child.remove_voter(NodeId(4))?;
        child.wait_conf_applied(at, Duration::from_secs(5))?;
        Ok(at)
    })
    .await
    .unwrap();
    until_split(|| {
        children.iter().all(|d| {
            d.peer().status_snapshot().conf_applied >= removed.index.0
                && !d.status().learners.contains(&4)
        })
    })
    .await;
    let backend = f.backends[0].clone();
    blocking(move || group_compaction::execute(&backend, &mut None))
        .await
        .unwrap();
    let mut confirmed = floor.term.to_be_bytes().to_vec();
    confirmed.extend_from_slice(&floor.index.to_be_bytes());
    until_split(|| {
        children.iter().all(|d| {
            d.get(
                ColumnFamily::Default,
                kv9_common::data_range::COMPACTION_CONFIRMED_KEY,
            )
            .unwrap()
            .as_ref()
                == Some(&confirmed)
        })
    })
    .await;
    for (i, backend) in f.backends.iter().enumerate() {
        let backend = backend.clone();
        blocking(move || {
            group_compaction::execute(&backend, &mut None)?;
            group_compaction::reclaim(&backend, 0, &mut None)
        })
        .await
        .unwrap();
        assert_eq!(children[i].status().log_first_index, floor.index + 1);
        assert!(children[i].peer().log_file_bytes() >= before[i]);
        let backend = f.backends[i].clone();
        blocking(move || group_compaction::reclaim(&backend, 1, &mut Some(region.0 - 1)))
            .await
            .unwrap();
        let after = children[i].peer().log_file_bytes();
        assert!(
            after < before[i] / 4,
            "Raft replica {i}: {} -> {after}",
            before[i]
        );
        assert!(children[i]
            .get(ColumnFamily::Default, RANGE_KEY)
            .unwrap()
            .is_none());
        assert!(f.backends[i].raw_directory.by_region(region).is_none());
        eprintln!(
            "import_compaction_raft replica={i} before={} after={after} floor={}:{}",
            before[i], floor.term, floor.index
        );
    }
}

async fn split_resume(staging: bool, compact: bool, large: bool) {
    // Debug hashing of full pages must not use the tiny election interval of
    // the small-value fixture; this is a storage/recovery test, not a latency claim.
    let child_tick = Duration::from_millis(if staging { 100 } else { 5 });
    let (mut f, creations, mut children) = split_fixture_with_tick(child_tick).await;
    let root = f.range.root;
    assert!(matches!(
        f.client(0).activate().await.outcome,
        TtlOutcome::Success { .. }
    ));
    let batch_size = if staging { 10 } else { 100 };
    let rows = if large { 900 } else { 300 };
    for start in (0..rows).step_by(batch_size) {
        let pairs = (start..start + batch_size)
            .map(|i| {
                (
                    format!("a{i:04}").into_bytes(),
                    if staging {
                        vec![i as u8; 60 * 1024]
                    } else {
                        b"KV9TTL01 opaque original value".to_vec()
                    },
                )
            })
            .collect();
        let outcome = f
            .client(0)
            .call(Operation::Put {
                pairs,
                ttl_secs: Some(1),
            })
            .await
            .outcome;
        assert!(
            matches!(outcome, TtlOutcome::Success { .. }),
            "batch {start}: {outcome:?}"
        );
    }
    for (key, ttl_secs) in [(b"b-permanent".as_slice(), None), (b"c-expired", Some(0))] {
        assert!(matches!(
            f.client(0)
                .call(Operation::Put {
                    pairs: vec![(key.to_vec(), key.to_vec())],
                    ttl_secs
                })
                .await
                .outcome,
            TtlOutcome::Success { .. }
        ));
    }
    let key = |k: &[u8]| encode_key(KeyMode::Raw, f.range.keyspace, k).unwrap();
    let original_engine = f.backends[0].group_handles.lock().unwrap()[&f.range.region]
        .0
        .clone();
    let original = original_engine.snapshot().unwrap();
    let old_expiry = original
        .get(ColumnFamily::Write, &key(b"a0000"))
        .unwrap()
        .unwrap();
    let expired_bytes = original
        .get(ColumnFamily::Write, &key(b"c-expired"))
        .unwrap()
        .unwrap();

    // A writable quorum is insufficient even for source sealing.
    f.discoveries[2].raft_receive.store(false, Ordering::SeqCst);
    let backend = f.backends[0].clone();
    assert!(
        blocking(move || backend.seal_split_parent("test", root, [63; 16]))
            .await
            .is_err()
    );
    assert!(f.data[0]
        .get(ColumnFamily::Default, SEAL_KEY)
        .unwrap()
        .is_none());
    f.discoveries[2].authorize_raft();
    let backend = f.backends[0].clone();
    let sealed = blocking(move || backend.seal_split_parent("test", root, [63; 16]))
        .await
        .unwrap();
    applied(
        &f.data[..3],
        kv9_raft::ProposedAt {
            term: sealed.cut.term,
            index: kv9_raft::LogIndex(sealed.cut.index),
        },
    )
    .await;
    let backend = f.backends[0].clone();
    assert_eq!(
        blocking(move || backend.seal_split_parent("test", root, [63; 16]))
            .await
            .unwrap()
            .cut,
        sealed.cut
    );

    // Prepare just one page through the same production source/format gate.
    let backend = f.backends[0].clone();
    let child = children[0][0].clone();
    let parent = f.data[0].clone();
    blocking(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        let committed = committed_split_plan(&backend.node.meta_raft.store, [63; 16])?;
        let source = parent.capture_ttl_split_source(committed.plan(), deadline)?;
        let at = child.propose(&Command::Noop)?;
        settle_proposal(at, child.wait_applied(at, Duration::from_secs(5)), true)?;
        let probe = child.begin_ttl_split_import(&source, false, deadline)?;
        backend.submit_ttl_split(&committed, &child, probe)?;
        if !staging {
            // Even a fully confirmed, source-derived transfer must recheck
            // local capacity at its protected append; no RawGroup exists yet.
            let before = child.status();
            child.peer().set_max_write_log_entries(1);
            let probe = child.begin_ttl_split_copy(&source, false, deadline)?;
            assert!(matches!(
                backend.submit_ttl_split(&committed, &child, probe),
                Err(Error::WriteBackpressure { .. })
            ));
            assert_eq!(child.status().log_last_index, before.log_last_index);
            assert_eq!(
                child
                    .ttl_import_state(source.spec(false).target())?
                    .unwrap()
                    .copied,
                0
            );
            child
                .peer()
                .set_max_write_log_entries(before.max_write_log_entries);
        }
        let probe = child.begin_ttl_split_copy(&source, false, deadline)?;
        backend.submit_ttl_split(&committed, &child, probe)?;
        if staging {
            // Copy every page but deliberately stop before Finish.
            while child
                .ttl_import_state(source.spec(false).target())?
                .unwrap()
                .copied
                < source.spec(false).rows
            {
                let probe = child.begin_ttl_split_copy(&source, false, deadline)?;
                backend.submit_ttl_split(&committed, &child, probe)?;
            }
        }
        Ok(())
    })
    .await
    .unwrap();
    let partial = ImportState::decode(
        &children[0][0]
            .get(ColumnFamily::Default, IMPORT_KEY)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(partial.copied, if staging { rows as u64 + 2 } else { 256 });
    assert!(!partial.complete);
    if compact {
        let (engine, driver) =
            f.backends[1].group_handles.lock().unwrap()[&creations[0].region()].clone();
        assert!(crate::runtime::group_compaction::import_cut(
            &f.backends[1],
            creations[0].region(),
            &engine,
            &driver,
        )
        .unwrap()
        .is_none());
    }
    assert!(children[0][0]
        .get(ColumnFamily::Default, RANGE_KEY)
        .unwrap()
        .is_none());
    let at = children[0][0].propose(&Command::Noop).unwrap();
    applied(&children[0], at).await;
    let backend = f.backends[1].clone();
    assert!(
        blocking(move || backend.publish_split("test", root, [63; 16]))
            .await
            .is_err()
    );

    let uploader = staging.then(|| {
        Arc::new(
            kv9_engine::checkpoint::RemoteUploader::for_harness(Arc::new(
                kv9_engine::MemoryObjectStore::new(),
            ))
            .with_image_budget(Arc::new(
                kv9_engine::checkpoint_memory::ImageBudget::new(17 * 1024 * 1024).unwrap(),
            )),
        )
    });
    if let Some(uploader) = &uploader {
        let engines = f
            .backends
            .iter()
            .map(|b| {
                b.group_handles.lock().unwrap()[&creations[0].region()]
                    .0
                    .clone()
            })
            .collect::<Vec<_>>();
        fn tree_bytes(path: &std::path::Path) -> u64 {
            if path.is_file() {
                return path.metadata().unwrap().len();
            }
            std::fs::read_dir(path)
                .unwrap()
                .map(|e| tree_bytes(&e.unwrap().path()))
                .sum()
        }
        let size = |engine: &WalEngine| {
            tree_bytes(&engine.path()) + tree_bytes(&engine.path().with_extension("segments"))
        };
        let before = engines.iter().map(|e| size(e)).collect::<Vec<_>>();
        assert!(before.iter().all(|bytes| *bytes > 16 * 1024 * 1024));
        let tasks = f
            .backends
            .iter()
            .map(|b| {
                crate::runtime::data_checkpoint::Maintenance::start(
                    b.clone(),
                    uploader.clone(),
                    true,
                    Duration::from_millis(100),
                    &tokio::runtime::Handle::current(),
                )
            })
            .collect::<Vec<_>>();
        // Allow the production 30-second pass plus follower adoption for this
        // debug-build storage test; report real worker failures on timeout.
        let selected = tokio::time::timeout(Duration::from_secs(60), async {
            while !engines
                .iter()
                .all(|e| WalEngine::checkpoint_reference(e.path()).unwrap().is_some())
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        let stats = tasks.iter().map(|task| task.snapshot()).collect::<Vec<_>>();
        for task in &tasks {
            task.cancel();
        }
        for task in tasks {
            task.drain().await;
        }
        assert!(
            selected.is_ok(),
            "checkpoint workers: {}",
            serde_json::to_string(&stats).unwrap()
        );
        let memory = uploader.image_memory_snapshot();
        assert_eq!(memory.retained_bytes, 0);
        assert_eq!(memory.active_images, 0);
        assert!(memory.peak_bytes <= memory.limit_bytes);
        eprintln!(
            "staging_checkpoint_spool limit_bytes={} peak_reserved_bytes={}",
            memory.limit_bytes, memory.peak_bytes
        );
        for (i, engine) in engines.iter().enumerate() {
            let selected = WalEngine::checkpoint_reference(engine.path())
                .unwrap()
                .unwrap();
            assert!(selected.files.iter().map(|f| f.size).sum::<u64>() > memory.limit_bytes);
            assert_eq!(selected.requires_large_format().unwrap(), large);
            let restored = uploader.restore(&selected).unwrap();
            let identity = DataCheckpointIdentity {
                cluster: f.backends[0].ttl_context().unwrap().root.cluster_id,
                root,
                region: creations[0].region(),
                creation: creations[0].digest(),
            };
            let image = data_group::inspect_checkpoint(
                restored.snapshot().unwrap().as_ref(),
                &selected,
                identity,
            )
            .unwrap();
            assert_eq!(
                image.state(),
                &data_group::DataCheckpointState::Import(Box::new(partial.clone()))
            );
            assert!(data_group::inspect_base(
                restored.snapshot().unwrap().as_ref(),
                &selected,
                identity
            )
            .is_err());
            assert!(engine
                .get(ColumnFamily::Default, RANGE_KEY)
                .unwrap()
                .is_none());
            assert!(f.backends[i]
                .raw_directory
                .by_region(creations[0].region())
                .is_none());
            let after = size(engine);
            assert!(
                after < before[i] / 2,
                "staging WAL replica {i}: {} -> {after}",
                before[i]
            );
            eprintln!(
                "staging_checkpoint_wal replica={i} before={} after={after}",
                before[i]
            );
        }
    }

    if compact {
        compact_import(&f, &children[0], creations[0].region()).await;
    }

    // Real mid-copy WAL/Raft reopen of all child replicas, elect another node.
    let low_region = creations[0].region();
    for driver in &children[0] {
        driver.stop();
    }
    // Six initial metadata/parent owners, followed by the low child's three.
    for owner in f.owners.drain(6..9) {
        owner.join().unwrap();
    }
    f.data.retain(|d| d.peer().region_id() != low_region);
    children[0].clear();
    for backend in &f.backends {
        backend.group_handles.lock().unwrap().remove(&low_region);
    }
    let hub = InProcHub::new();
    for i in 0..3 {
        let path = f.path.join(format!("n{}/split-{}", i + 1, low_region.0));
        let mut storage = DiskRaftStorage::open(&path, &[1, 2, 3]).unwrap().0;
        let identity = DataCheckpointIdentity {
            cluster: f.backends[0].ttl_context().unwrap().root.cluster_id,
            root,
            region: low_region,
            creation: creations[0].digest(),
        };
        let (engine, _, dependencies) = data_group::open_with_dependencies(
            &mut storage,
            path.with_extension("wal"),
            uploader.as_deref(),
            identity,
        )
        .unwrap();
        assert!(dependencies.installed().is_none());
        if staging {
            let manifest = engine.selected_checkpoint().unwrap().unwrap();
            let restored = uploader.as_ref().unwrap().restore(&manifest).unwrap();
            let import = ImportState::decode(
                &restored
                    .get(ColumnFamily::Default, IMPORT_KEY)
                    .unwrap()
                    .unwrap(),
            )
            .unwrap();
            let selected = dependencies.selected().unwrap();
            assert_eq!(
                selected.manifest(),
                RootDigest::sha256(&manifest.encode().unwrap())
            );
            assert_eq!(
                selected.state(),
                data_group::RecoveryImageState::Import(RootDigest::sha256(&import.encode()))
            );
            assert!(selected.publication().unwrap().at().index > selected.cut().index);
        } else {
            assert!(dependencies.selected().is_none());
        }
        let engine = Arc::new(engine);
        assert_eq!(
            ImportState::decode(
                &engine
                    .get(ColumnFamily::Default, IMPORT_KEY)
                    .unwrap()
                    .unwrap()
            )
            .unwrap(),
            partial
        );
        let mut sm = MemStateMachine::with_engine(engine.clone()).unwrap();
        sm.set_data_group(root, low_region, creations[0].digest())
            .unwrap();
        let compactions = kv9_meta::data_groups::compaction::committed_compaction_floors(
            &f.backends[i].node.meta_raft.store,
        )
        .unwrap();
        let absent = crate::region_manager::RecoveryDecisions {
            truncations: &[],
            compactions: &[],
        };
        assert_eq!(absent.checked_base(low_region, &storage).is_err(), compact);
        let base = crate::region_manager::RecoveryDecisions {
            truncations: &[],
            compactions: &compactions,
        }
        .checked_base(low_region, &storage)
        .unwrap();
        assert_eq!(base.is_some(), compact);
        let transport = Arc::new(hub.endpoint(NodeId(i as u64 + 1))) as Arc<dyn RaftTransport>;
        let driver = match base {
            Some(base) => {
                let peer = Arc::new(
                    RaftPeer::with_installed_storage(
                        NodeId(i as u64 + 1),
                        low_region,
                        storage,
                        base,
                    )
                    .unwrap(),
                );
                NodeDriver::with_installed_base(peer, transport, sm, base).unwrap()
            }
            None => {
                let peer = Arc::new(
                    RaftPeer::with_storage(NodeId(i as u64 + 1), low_region, storage).unwrap(),
                );
                NodeDriver::new(peer, transport, sm).unwrap()
            }
        };
        f.backends[i]
            .group_handles
            .lock()
            .unwrap()
            .insert(low_region, (engine, driver.clone()));
        children[0].push(driver);
    }
    // Reapply the retained configuration suffix before campaigning. raft-rs
    // refuses campaigns while committed configuration entries remain unapplied;
    // otherwise a later timer election would choose an accidental test leader.
    pump(&children[0]);
    for driver in &children[0] {
        assert_eq!(driver.status().voters, vec![1, 2, 3]);
        assert!(driver.status().learners.is_empty());
        if compact {
            assert!(driver.status().conf_index > driver.status().log_first_index);
        }
    }
    children[0][1].peer().campaign().unwrap();
    pump(&children[0]);
    assert_eq!(children[0][1].status().role, Role::Leader);
    for driver in &children[0] {
        f.owners.push(driver.spawn(child_tick).unwrap());
        f.data.push(driver.clone());
    }
    let backend = f.backends[1].clone();
    let copied = blocking(move || backend.populate_split_child("test", root, [63; 16], false))
        .await
        .unwrap();
    assert_eq!(copied.rows_copied, rows as u64 + 2);
    assert_eq!(copied.parent_half_digest, copied.child_digest);
    let backend = f.backends[1].clone();
    assert!(
        blocking(move || backend.publish_split("test", root, [63; 16]))
            .await
            .is_err()
    );
    let backend = f.backends[2].clone();
    let empty = blocking(move || backend.populate_split_child("test", root, [63; 16], true))
        .await
        .unwrap();
    assert_eq!(empty.rows_copied, 0);
    for (half, leader) in [(0, 1), (1, 2)] {
        let at = children[half][leader].propose(&Command::Noop).unwrap();
        applied(&children[half], at).await;
    }
    for ((low_driver, high_driver), backend) in
        children[0].iter().zip(&children[1]).zip(&f.backends)
    {
        assert_eq!(
            low_driver
                .get(ColumnFamily::Write, &key(b"a0000"))
                .unwrap()
                .as_ref(),
            Some(&old_expiry)
        );
        assert_eq!(
            low_driver
                .get(ColumnFamily::Write, &key(b"c-expired"))
                .unwrap()
                .as_ref(),
            Some(&expired_bytes)
        );
        assert!(low_driver
            .get(ColumnFamily::Write, &key(b"b-permanent"))
            .unwrap()
            .is_none());
        assert_eq!(
            high_driver.get(ColumnFamily::Default, CLOCK_KEY).unwrap(),
            f.data[0].get(ColumnFamily::Default, CLOCK_KEY).unwrap()
        );
        assert!(backend.raw_directory.by_region(low_region).is_none());
    }
    let backend = f.backends[1].clone();
    let published = blocking(move || backend.publish_split("test", root, [63; 16]))
        .await
        .unwrap();
    applied(
        &f.meta,
        kv9_raft::ProposedAt {
            term: published.applied.term,
            index: kv9_raft::LogIndex(published.applied.index),
        },
    )
    .await;
    let bindings =
        kv9_meta::data_groups::ranges::committed_ranges(&f.backends[1].node.meta_raft.store)
            .unwrap();
    for backend in &f.backends {
        for binding in &bindings {
            let range = binding.range();
            let (engine, driver) = backend.group_handles.lock().unwrap()[&range.region].clone();
            backend.raw_directory.insert(range_api::RawGroup::new(
                range.clone(),
                engine,
                driver,
                Some(Arc::new(Clock)),
            ));
        }
    }
    let low = bindings
        .iter()
        .find(|b| b.range().region == low_region)
        .unwrap()
        .range()
        .clone();
    let client = ScopedTtlClient::with_test_transport(
        low,
        TransportKind::TonicUnary,
        f.public[1].clone(),
        5000,
    );
    assert_eq!(
        client
            .call(Operation::Get {
                keys: vec![b"c-expired".to_vec()]
            })
            .await
            .outcome,
        TtlOutcome::Success {
            reply: Reply::Values { values: vec![None] }
        }
    );
    assert!(matches!(
        client
            .call(Operation::Put {
                pairs: vec![(b"a0000".to_vec(), b"renewed after publication".to_vec())],
                ttl_secs: Some(20)
            })
            .await
            .outcome,
        TtlOutcome::Success { .. }
    ));
    let backend = f.backends[1].clone();
    assert!(matches!(
        blocking(move || backend.populate_split_child("test", root, [63; 16], false)).await,
        Err(Error::StaleEpoch { .. })
    ));
    let backend = f.backends[1].clone();
    assert!(
        !blocking(move || backend.publish_split("test", root, [63; 16]))
            .await
            .unwrap()
            .changed
    );
    assert_ne!(
        children[0][1]
            .get(ColumnFamily::Write, &key(b"a0000"))
            .unwrap()
            .as_ref(),
        Some(&old_expiry)
    );
    if large {
        verify_large_bound_checkpoint(&mut f, &children[0], low_region, uploader.as_ref().unwrap())
            .await;
    }
    drop(original);
    f.close().await;
}

async fn verify_large_bound_checkpoint(
    f: &mut Fixture,
    children: &[Driver],
    region: RegionId,
    uploader: &Arc<kv9_engine::checkpoint::RemoteUploader>,
) {
    // Only the low child has reopened drivers and thus fresh manifest seams.
    // The prior worker owns the other drivers' one-shot seams for their lifetime;
    // this fixture must not remint them when starting a second maintenance task.
    for backend in &f.backends {
        let other = backend
            .group_handles
            .lock()
            .unwrap()
            .keys()
            .copied()
            .filter(|id| *id != region)
            .collect::<Vec<_>>();
        for id in other {
            backend.raw_directory.remove(id);
            backend.group_handles.lock().unwrap().remove(&id);
        }
    }
    let at = children[1].propose(&Command::Noop).unwrap();
    applied(children, at).await;
    let engines = f
        .backends
        .iter()
        .map(|backend| backend.group_handles.lock().unwrap()[&region].0.clone())
        .collect::<Vec<_>>();
    let before = engines
        .iter()
        .map(|engine| {
            WalEngine::checkpoint_reference(engine.path())
                .unwrap()
                .unwrap()
                .index
        })
        .collect::<Vec<_>>();
    let tasks = f
        .backends
        .iter()
        .map(|backend| {
            crate::runtime::data_checkpoint::Maintenance::start(
                backend.clone(),
                uploader.clone(),
                true,
                Duration::from_millis(100),
                &tokio::runtime::Handle::current(),
            )
        })
        .collect::<Vec<_>>();
    let selected = tokio::time::timeout(Duration::from_secs(60), async {
        while !engines.iter().zip(&before).all(|(engine, before)| {
            WalEngine::checkpoint_reference(engine.path())
                .unwrap()
                .is_some_and(|m| m.index > *before)
        }) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    let stats = tasks.iter().map(|task| task.snapshot()).collect::<Vec<_>>();
    for task in &tasks {
        task.cancel();
    }
    for task in tasks {
        task.drain().await;
    }
    assert!(
        selected.is_ok(),
        "bound checkpoint workers: {}",
        serde_json::to_string(&stats).unwrap()
    );
    // Quiesce actual owners before reopening any storage. Fixture::close will
    // then only tear down servers and remove its directory.
    for driver in f.meta.iter().chain(&f.data) {
        driver.stop();
    }
    for owner in f.owners.drain(..) {
        owner.join().unwrap();
    }
    let range = f.backends[1]
        .raw_directory
        .by_region(region)
        .unwrap()
        .capture_parts()
        .2
        .clone();
    let identity = DataCheckpointIdentity {
        cluster: f.backends[0].ttl_context().unwrap().root.cluster_id,
        root: range.root,
        region,
        creation: range.creation,
    };
    let key = encode_key(KeyMode::Raw, range.keyspace, b"a0000").unwrap();
    let expiry = children[1].get(ColumnFamily::Write, &key).unwrap().unwrap();
    for (i, engine) in engines.iter().enumerate() {
        let manifest = WalEngine::checkpoint_reference(engine.path())
            .unwrap()
            .unwrap();
        assert!(manifest.requires_large_format().unwrap());
        let path = f.path.join(format!("n{}/split-{}", i + 1, region.0));
        let mut storage = DiskRaftStorage::open(&path, &[1, 2, 3]).unwrap().0;
        let (reopened, _, recovered) =
            data_group::open(&mut storage, engine.path(), Some(uploader), identity).unwrap();
        assert_eq!(recovered.unwrap().range(), Some(&range));
        assert_eq!(
            reopened.get(ColumnFamily::Default, &key).unwrap(),
            Some(b"renewed after publication".to_vec())
        );
        assert_eq!(
            reopened.get(ColumnFamily::Write, &key).unwrap().as_ref(),
            Some(&expiry)
        );
        eprintln!(
            "large_bound_checkpoint_recovered replica={i} image_bytes={} cut={}:{}",
            manifest.files.iter().map(|f| f.size).sum::<u64>(),
            manifest.term,
            manifest.index
        );
    }
    assert_eq!(uploader.spool_disk_snapshot().reserved_bytes, 0);
    assert_eq!(uploader.image_memory_snapshot().retained_bytes, 0);
}

async fn until_split(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("split maintenance made no progress");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn split_worker_resumes_committed_intent_with_zero_trigger_and_shared_admission() {
    use crate::maintenance::Class;
    use crate::runtime::split_maintenance::Maintenance;
    let (f, creations, children) = split_fixture().await;
    assert!(matches!(
        f.client(0).activate().await.outcome,
        TtlOutcome::Success { .. }
    ));

    // Actual dispatch must wait for the shared heavy capacity on each node.
    let mut holds = Vec::new();
    for backend in &f.backends {
        let mut lane = backend.maintenance.lane(Class::DataCheckpoint);
        let first = lane.try_acquire().unwrap();
        let second = lane.try_acquire().unwrap();
        holds.push((lane, first, second));
    }
    let workers = f
        .backends
        .iter()
        .map(|backend| {
            let worker = Maintenance::start(backend.clone(), 0, &tokio::runtime::Handle::current());
            worker.configure(true, 0);
            worker
        })
        .collect::<Vec<_>>();
    until_split(|| {
        f.backends
            .iter()
            .all(|b| b.maintenance.snapshot().waiting_classes == 1)
    })
    .await;
    assert!(workers.iter().all(|w| w.snapshot().passes == 0));
    assert!(f.data[0]
        .get(ColumnFamily::Default, SEAL_KEY)
        .unwrap()
        .is_none());

    // Losing serving eligibility withdraws demand. Zero alone must not do so.
    for worker in &workers {
        worker.configure(false, 0);
    }
    until_split(|| {
        f.backends
            .iter()
            .all(|b| b.maintenance.snapshot().waiting_classes == 0)
    })
    .await;
    for worker in &workers {
        worker.configure(true, 0);
    }
    until_split(|| {
        f.backends
            .iter()
            .all(|b| b.maintenance.snapshot().waiting_classes == 1)
    })
    .await;

    // Foreground TTL remains usable while split admission is blocked.
    for (key, ttl_secs) in [
        (b"a-live".as_slice(), Some(60)),
        (b"z-expired", Some(0)),
        (b"b-permanent", None),
    ] {
        assert!(matches!(
            f.client(0)
                .call(Operation::Put {
                    pairs: vec![(key.to_vec(), key.to_vec())],
                    ttl_secs,
                })
                .await
                .outcome,
            TtlOutcome::Success { .. }
        ));
    }
    let key = |k: &[u8]| encode_key(KeyMode::Raw, f.range.keyspace, k).unwrap();
    let live = f.data[0]
        .get(ColumnFamily::Write, &key(b"a-live"))
        .unwrap()
        .unwrap();
    let expired = f.data[0]
        .get(ColumnFamily::Write, &key(b"z-expired"))
        .unwrap()
        .unwrap();

    // Background work uses the same all-store format gate as manual work.
    f.discoveries[2].raft_receive.store(false, Ordering::SeqCst);
    drop(holds);
    until_split(|| workers[0].snapshot().failures > 0).await;
    assert!(f.data[0]
        .get(ColumnFamily::Default, SEAL_KEY)
        .unwrap()
        .is_none());
    f.discoveries[2].authorize_raft();
    until_split(|| {
        f.backends.iter().all(|b| {
            committed_split_plan(&b.node.meta_raft.store, [63; 16])
                .unwrap()
                .published()
        })
    })
    .await;
    until_split(|| {
        workers
            .iter()
            .map(|w| w.snapshot().publications)
            .sum::<u64>()
            == 1
    })
    .await;
    for (half, drivers) in children.iter().enumerate() {
        let expected_key = if half == 0 {
            b"a-live".as_slice()
        } else {
            b"z-expired"
        };
        let expected_expiry = if half == 0 { &live } else { &expired };
        until_split(|| {
            drivers
                .iter()
                .all(|d| d.get(ColumnFamily::Default, RANGE_KEY).unwrap().is_some())
        })
        .await;
        for driver in drivers {
            assert_eq!(
                driver
                    .get(ColumnFamily::Write, &key(expected_key))
                    .unwrap()
                    .as_ref(),
                Some(expected_expiry)
            );
            assert_eq!(
                driver
                    .get(ColumnFamily::Default, &key(expected_key))
                    .unwrap()
                    .unwrap(),
                expected_key
            );
            let state = ImportState::decode(
                &driver
                    .get(ColumnFamily::Default, IMPORT_KEY)
                    .unwrap()
                    .unwrap(),
            )
            .unwrap();
            assert!(state.complete);
        }
    }
    assert!(children[0][0]
        .get(ColumnFamily::Write, &key(b"b-permanent"))
        .unwrap()
        .is_none());
    assert_eq!(
        children[0][0]
            .get(ColumnFamily::Default, &key(b"b-permanent"))
            .unwrap()
            .unwrap(),
        b"b-permanent"
    );
    let stats = workers
        .iter()
        .map(Maintenance::snapshot)
        .collect::<Vec<_>>();
    assert_eq!(stats.iter().map(|s| s.seals).sum::<u64>(), 1);
    assert_eq!(stats.iter().map(|s| s.populations).sum::<u64>(), 2);
    assert!(stats
        .iter()
        .all(|s| s.trigger_bytes == 0 && s.triggers == 0));
    for backend in &f.backends {
        assert_eq!(
            kv9_meta::data_groups::split::committed_splits(&backend.node.meta_raft.store)
                .unwrap()
                .len(),
            1
        );
        for creation in &creations {
            assert!(kv9_meta::data_groups::committed_creation(
                &backend.node.meta_raft.store,
                creation.task()
            )
            .unwrap()
            .is_some());
        }
    }
    for worker in &workers {
        worker.cancel();
    }
    for worker in workers {
        tokio::time::timeout(Duration::from_secs(5), worker.drain())
            .await
            .unwrap();
    }
    for backend in &f.backends {
        let budget = backend.maintenance.snapshot();
        assert_eq!(budget.active, 0);
        assert_eq!(budget.heavy_active, 0);
        assert_eq!(budget.waiting_classes, 0);
    }
    f.close().await;
}
