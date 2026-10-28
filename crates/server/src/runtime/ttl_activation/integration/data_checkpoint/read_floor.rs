use super::*;
use crate::checkpoint_retention::{CheckpointLedger, CheckpointOwners, CheckpointRead};
use crate::recovery_retention::RecoveryRetention;
use crate::runtime::checkpoint_retention::RoutedLedger;
use kv9_common::data_read_fence::ReadFloor;
use kv9_common::retention::PinPhase;
use kv9_meta::retention::{checkpoint_intent::CheckpointIntent, checkpoint_reader, OwnerKind};
use kv9_raft::state_machine::checkpoint_recovery::data_group::{self, DataCheckpointIdentity};
use std::path::{Path, PathBuf};

mod contention;

fn copy_tree(from: &Path, to: &Path) {
    if from.is_dir() {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            copy_tree(&entry.path(), &to.join(entry.file_name()));
        }
    } else {
        std::fs::copy(from, to).unwrap();
    }
}
fn copy_wal(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for extension in ["wal", "segments", "checkpoint"] {
        let file = from.with_extension(extension);
        if file.exists() {
            copy_tree(&file, &to.join(file.file_name().unwrap()));
        }
    }
}
fn inventory(path: &Path) -> Vec<(PathBuf, RootDigest)> {
    fn add(path: &Path, out: &mut Vec<(PathBuf, RootDigest)>) {
        if path.is_dir() {
            for entry in std::fs::read_dir(path).unwrap() {
                add(&entry.unwrap().path(), out);
            }
        } else {
            out.push((
                path.to_path_buf(),
                RootDigest::sha256(&std::fs::read(path).unwrap()),
            ));
        }
    }
    let mut out = Vec::new();
    add(path, &mut out);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}
async fn prepare_next_image(
    f: &Fixture,
    root: &RootDescriptor,
    store: Arc<readers::ReadCount>,
) -> (CheckpointManifest, Command) {
    assert!(matches!(
        f.client(0)
            .call(Operation::Put {
                pairs: vec![(b"floor".to_vec(), b"new".to_vec())],
                ttl_secs: Some(60)
            })
            .await
            .outcome,
        TtlOutcome::Success { .. }
    ));
    let root = root.clone();
    let e = engine(f, 0);
    let region = f.range.region;
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    tokio::task::spawn_blocking(move || {
        let uploader = RemoteUploader::for_harness(store);
        let plan = RemoteUploader::plan(
            e.freeze(kv9_engine::checkpoint::FlushScope {
                cluster: root.cluster_id.to_string(),
                region: region.0,
                conf_ver: 1,
                version: 1,
            })
            .unwrap(),
        )
        .unwrap();
        let manifest = plan.manifest().clone();
        let owners = CheckpointOwners::new(&root, &manifest, 1).unwrap();
        owners.before_io(&ledger).unwrap();
        let command = Command::ManifestChange(
            kv9_raft::ManifestChangePayload::from_prepared(
                uploader.upload_planned(plan).unwrap(),
                1,
            )
            .unwrap(),
        );
        (manifest, command)
    })
    .await
    .unwrap()
}
async fn publish_next_image(f: &Fixture, command: Command) {
    let d = f.data[0].clone();
    tokio::task::spawn_blocking(move || {
        let at = d.propose(&command).unwrap();
        assert!(matches!(
            d.wait_applied(at, Duration::from_secs(3)).unwrap(),
            kv9_raft::driver::ApplyWaitOutcome::Manifest {
                verdict: kv9_raft::ManifestVerdict::Applied { .. },
                ..
            }
        ));
        // Actual adoption workers finish this positive handoff. Their shared
        // metadata catalog can be busy; a one-shot helper is not a completion
        // guarantee. selected_floor also checks the exact terminal owners.
    })
    .await
    .unwrap();
    until(|| {
        f.data
            .iter()
            .all(|d| d.manifest_pair(f.range.region.0).unwrap().generation == 2)
    })
    .await;
}
fn owner_phases(
    f: &Fixture,
    root: &RootDescriptor,
    manifest: &CheckpointManifest,
    predecessor: u64,
) -> (Option<PinPhase>, Option<PinPhase>) {
    let intent = CheckpointIntent::new(root, manifest, predecessor).unwrap();
    let snapshot = f.backends[1]
        .node
        .meta_raft
        .store
        .engine()
        .snapshot()
        .unwrap();
    let phase = |kind| {
        let binding = intent.binding(kind).unwrap();
        kv9_meta::retention::retention_owner(snapshot.as_ref(), root, binding.descriptor.id)
            .unwrap()
            .map(|owner| {
                assert_eq!(owner.binding, binding);
                owner.phase
            })
    };
    (phase(OwnerKind::Pending), phase(OwnerKind::Version))
}
async fn selected_floor(f: &Fixture, jobs: &[Maintenance], generation: u64) {
    eprintln!("read floor: select generation {generation}");
    until(|| {
        f.backends.iter().all(|b| {
            b.ttl_context()
                .unwrap()
                .data_read_fences
                .floor(f.range.region)
                .unwrap()
                .is_some_and(|floor| floor.generation == generation)
        }) && jobs.iter().all(|j| j.snapshot().adoptions >= generation)
    })
    .await;
    let root = &f.backends[0].ttl_context().unwrap().root;
    let manifest = selected(f, 0).unwrap();
    assert_eq!(
        owner_phases(f, root, &manifest, generation - 1),
        (Some(PinPhase::Released), Some(PinPhase::Published))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn durable_data_read_floor_blocks_old_get_and_cold_wal_rollback_on_three_replicas() {
    let mut f = Fixture::new().await;
    let (root, old, objects) = readers::published(&f).await;
    let uploader = Arc::new(RemoteUploader::for_harness(objects.clone()));
    let no_image = f.path.join("no-image");
    copy_wal(&engine(&f, 0).path(), &no_image);
    let jobs: Vec<_> = (0..3)
        .map(|i| readers::task(&f, i, uploader.clone()))
        .collect();
    selected_floor(&f, &jobs, 1).await;
    let old_wal = f.path.join("old-image");
    copy_wal(&engine(&f, 0).path(), &old_wal);
    let (next, command) = prepare_next_image(&f, &root, objects.clone()).await;
    publish_next_image(&f, command).await;
    selected_floor(&f, &jobs, 2).await;
    for job in jobs {
        job.drain().await;
    }
    let mut floors = Vec::new();
    for i in 0..3 {
        let context = f.backends[i].ttl_context().unwrap();
        let floor = context
            .data_read_fences
            .floor(f.range.region)
            .unwrap()
            .unwrap();
        assert_eq!(
            floor,
            ReadFloor {
                identity: context.local.identity(),
                creation: f.range.creation,
                image: crate::checkpoint_retention::data_read_image(&next)
                    .unwrap()
                    .unwrap(),
                generation: 2
            }
        );
        floors.push(floor);
        let ledger = RoutedLedger::new(&f.backends[i], tokio::runtime::Handle::current());
        let old = old.clone();
        let next = next.clone();
        let root = root.clone();
        let uploader = uploader.clone();
        let objects = objects.clone();
        tokio::task::spawn_blocking(move || {
            let gets = objects.gets.load(Ordering::SeqCst);
            let mut reader =
                CheckpointRead::new(&root, &old, 0, OwnerKind::Version, &ledger).unwrap();
            let (result, report) = uploader
                .with_local_read_report(|scoped| {
                    reader.before_io(&ledger, scoped)?;
                    scoped.restore(&old)
                })
                .unwrap();
            assert!(
                result.is_err(),
                "old image reached object GET below durable floor"
            );
            assert_eq!(objects.gets.load(Ordering::SeqCst), gets);
            reader.observe_drain(report).unwrap();
            reader.finish(&ledger).unwrap();
            let source = CheckpointIntent::new(&root, &old, 0)
                .unwrap()
                .binding(OwnerKind::Version)
                .unwrap();
            let binding =
                checkpoint_reader::binding(&source, ledger.local_lifetime().unwrap()).unwrap();
            let bytes = ledger
                .owner(root.digest(), binding.descriptor.id)
                .unwrap()
                .unwrap();
            assert_eq!(
                kv9_meta::retention::decode_owner_observation(
                    &bytes,
                    root.digest(),
                    binding.descriptor.id
                )
                .unwrap()
                .phase,
                kv9_common::retention::PinPhase::Released
            );
            let retention = RecoveryRetention::new(root, floor.identity, Arc::new(ledger));
            let recovery = retention.reader(&uploader, None);
            use kv9_raft::state_machine::checkpoint_recovery::data_group::DataCheckpointReader;
            recovery.restore_published(&next, 1).unwrap();
            assert!(objects.gets.load(Ordering::SeqCst) > gets);
        })
        .await
        .unwrap();
    }
    eprintln!("read floor: failover and cold recovery");
    f.meta[1].peer().transfer_leader_for_tests(NodeId(3));
    until(|| f.meta[2].status().role == Role::Leader && f.meta[1].status().role != Role::Leader)
        .await;
    // Stop and join only the three DATA owners; retain real routed metadata
    // consensus for successful cold recovery's Reader admission and finish.
    for driver in &f.data {
        driver.stop();
    }
    for index in [5, 3, 1] {
        f.owners.remove(index).join().unwrap();
    }
    let latest = f.path.join("new-image");
    copy_wal(&engine(&f, 0).path(), &latest);
    let raft_dir = f.path.join("cold-raft");
    copy_tree(&f.path.join("n1/data-raft"), &raft_dir);
    let ledger = Arc::new(RoutedLedger::new(
        &f.backends[0],
        tokio::runtime::Handle::current(),
    ));
    let identity = DataCheckpointIdentity {
        cluster: root.cluster_id,
        root: root.digest(),
        region: f.range.region,
        creation: f.range.creation,
    };
    let root_for_read = root.clone();
    let uploader_for_read = uploader.clone();
    let objects_for_read = objects.clone();
    let store_identity = floors[0].identity;
    tokio::task::spawn_blocking(move || {
        let retention = RecoveryRetention::new(root_for_read, store_identity, ledger);
        let reader = retention.reader(&uploader_for_read, None);
        for path in [&no_image, &old_wal] {
            let before = inventory(path);
            let gets = objects_for_read.gets.load(Ordering::SeqCst);
            let mut storage = DiskRaftStorage::recover(&raft_dir).unwrap();
            let result = data_group::open_with_retained_reader(
                &mut storage,
                path.join("data.wal"),
                Some(&uploader_for_read),
                Some(&reader),
                identity,
            );
            assert!(
                result.is_err(),
                "rollback cold open ignored local read floor"
            );
            let error = result.err().unwrap().to_string();
            assert!(
                error.contains("durable local read floor"),
                "wrong refusal: {error}"
            );
            assert_eq!(objects_for_read.gets.load(Ordering::SeqCst), gets);
            assert_eq!(inventory(path), before, "refused open mutated WAL");
        }
        let mut storage = DiskRaftStorage::recover(&raft_dir).unwrap();
        data_group::open_with_retained_reader(
            &mut storage,
            latest.join("data.wal"),
            Some(&uploader_for_read),
            Some(&reader),
            identity,
        )
        .unwrap();
    })
    .await
    .unwrap();
    let region = f.range.region;
    let path = f.shutdown_keep().await;
    // New exclusive store owners reconstruct providers from durable files.
    for floor in floors {
        let guard = StoreGuard::lock(&path.join(format!("n{}", floor.identity.node_id.0))).unwrap();
        let provider = guard.data_read_fences(&floor.identity).unwrap();
        assert_eq!(provider.floor(region).unwrap(), Some(floor));
        assert!(provider
            .check_read(
                &crate::checkpoint_retention::data_read_image(&old)
                    .unwrap()
                    .unwrap()
            )
            .is_err());
    }
    std::fs::remove_dir_all(path).unwrap();
}
