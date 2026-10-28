use super::*;
use crate::command::{RawTtlAction, RawTtlCommand, RawTtlMutation};
use crate::driver::NodeDriver;
use crate::transport::{InProcHub, RaftTransport};
use crate::{Command, ManifestChangePayload, MemStateMachine, RaftGroup, RaftPeer, Role};
use kv9_common::raw_ttl::ExpiryPlan;
use kv9_engine::{ObjectKey, ObjectStore};
use std::time::Duration;

type Driver = NodeDriver<DiskRaftStorage, WalEngine>;

fn pump(drivers: &[&Driver]) {
    for _ in 0..30 {
        for driver in drivers {
            driver.tick_and_step().unwrap();
        }
    }
}
fn commit(source: &Driver, drivers: &[&Driver], command: Command) -> AppliedPosition {
    let proposed = source.propose(&command).unwrap();
    pump(drivers);
    source
        .wait_applied(proposed, Duration::from_secs(2))
        .unwrap();
    AppliedPosition {
        term: proposed.term,
        index: proposed.index.0,
    }
}
fn command(f: &Fixture, action: RawTtlAction) -> Command {
    Command::RawTtl(RawTtlCommand::new(&f.range, action).unwrap())
}
fn write(f: &Fixture, key: &[u8], value: &[u8], ttl: Option<u64>) -> Command {
    command(
        f,
        RawTtlAction::Write {
            plan: ExpiryPlan::resolve(
                ttl,
                ClockSample::with_uncertainty(1000, 0).unwrap(),
                floor(1000),
            )
            .unwrap(),
            mutations: vec![RawTtlMutation::Put {
                key: key.to_vec(),
                value: value.to_vec(),
            }],
        },
    )
}
fn open_destination(
    f: &Fixture,
    adopted: &AdoptedGeneration,
    uploader: &RemoteUploader,
    hub: &Arc<InProcHub>,
) -> (Arc<WalEngine>, Arc<Driver>) {
    let mut storage = DiskRaftStorage::recover(&adopted.raft_directory).unwrap();
    let base = storage.compacted_base().unwrap().unwrap();
    let engine = Arc::new(adopted.open_engine(&mut storage, uploader).unwrap().0);
    let mut state = MemStateMachine::with_engine(engine.clone()).unwrap();
    state
        .set_data_group(f.range.root, f.range.region, f.range.creation)
        .unwrap();
    let peer = Arc::new(
        RaftPeer::with_installed_storage(f.identity.node_id, f.range.region, storage, base)
            .unwrap(),
    );
    let driver = NodeDriver::with_installed_base(
        peer,
        Arc::new(hub.endpoint(f.identity.node_id)) as Arc<dyn RaftTransport>,
        state,
        base,
    )
    .unwrap();
    (engine, driver)
}

#[test]
fn live_capture_installs_ttl_at_learner_then_replays_renewal_cleanup_and_restart() {
    let f = Fixture::new();
    let hub = InProcHub::new();
    let objects = Arc::new(MemoryObjectStore::new());
    let uploader = RemoteUploader::for_harness(objects.clone());
    let source_engine = Arc::new(
        WalEngine::open(f.directory.join("live-source/data.wal"))
            .unwrap()
            .0,
    );
    let storage = DiskRaftStorage::open(&f.directory.join("live-source/raft"), &[1])
        .unwrap()
        .0;
    let mut state = MemStateMachine::with_engine(source_engine.clone()).unwrap();
    state
        .set_data_group(f.range.root, f.range.region, f.range.creation)
        .unwrap();
    let source = NodeDriver::new(
        Arc::new(RaftPeer::with_storage(NodeId(1), f.range.region, storage).unwrap()),
        Arc::new(hub.endpoint(NodeId(1))) as Arc<dyn RaftTransport>,
        state,
    )
    .unwrap();
    source.peer().campaign().unwrap();
    pump(&[&source]);
    assert_eq!(source.status().role, Role::Leader);
    commit(
        &source,
        &[&source],
        Command::DataRange {
            expected: None,
            next: f.range.clone(),
        },
    );
    // Isolate transfer from the separately tested all-replica activation gate.
    // The actual Enable entry still goes through Raft and positioned apply.
    source
        .peer()
        .propose_raw_for_harness(
            command(
                &f,
                RawTtlAction::Enable {
                    observed: floor(1000),
                },
            )
            .encode(),
        )
        .unwrap();
    pump(&[&source]);
    commit(
        &source,
        &[&source],
        write(&f, b"e", b"expired\0\xff", Some(0)),
    );
    commit(
        &source,
        &[&source],
        write(&f, b"l", b"before-capture", Some(1)),
    );
    commit(&source, &[&source], write(&f, b"p", b"", None));
    let e_version = ExpiryRecord::decode(
        &source_engine
            .get(ColumnFamily::Write, &physical(&f, b"e"))
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let l_version = ExpiryRecord::decode(
        &source_engine
            .get(ColumnFamily::Write, &physical(&f, b"l"))
            .unwrap()
            .unwrap(),
    )
    .unwrap();

    // An ordinary checkpoint publication leaves valid pair/history records in
    // the next image. Destination validation must retain and validate them too.
    let prepared = uploader
        .upload(
            source_engine
                .freeze(FlushScope {
                    cluster: f.identity.cluster_id.to_string(),
                    region: f.range.region.0,
                    conf_ver: f.range.conf_ver,
                    version: f.range.version,
                })
                .unwrap(),
        )
        .unwrap();
    commit(
        &source,
        &[&source],
        Command::ManifestChange(ManifestChangePayload::from_prepared(prepared, 0).unwrap()),
    );
    let learner = source.add_learner(f.identity.node_id).unwrap();
    pump(&[&source]);
    source
        .wait_conf_applied(learner, Duration::from_secs(2))
        .unwrap();
    commit(
        &source,
        &[&source],
        command(
            &f,
            RawTtlAction::Observe {
                observed: floor(1000),
            },
        ),
    );
    let planned = crate::snapshot_install::capture::plan_capture(
        &source,
        &source_engine,
        f.identity.cluster_id,
    )
    .unwrap();
    let captured_cut = planned.cut();
    // Tail writes after capture must arrive via Raft, not leak into the image.
    commit(
        &source,
        &[&source],
        write(&f, b"l", b"renewed-in-tail", Some(5)),
    );
    let tail = commit(
        &source,
        &[&source],
        command(
            &f,
            RawTtlAction::Observe {
                observed: floor(2000),
            },
        ),
    );
    let captured = planned.upload(&uploader).unwrap();
    assert_eq!(captured.cut, captured_cut);
    assert_eq!(captured.configuration.learners, vec![4]);
    let (snapshot, hs) = snapshot::decode(&captured.record).unwrap();
    let (_, original_manifest) = parse_image(&snapshot).unwrap();
    let mut installer = f.open();
    let installed = installer.install(&snapshot, &hs, &uploader).unwrap();
    assert_eq!(installed.records, 3);
    assert_eq!(installed.ttl_watermark, Some(floor(1000)));
    drop(installer);
    let adopted = adopt_for_runtime(&f.guard, f.identity, f.range.region, &uploader).unwrap();
    let (destination_engine, destination) = open_destination(&f, &adopted, &uploader, &hub);
    assert_eq!(destination.raw_ttl_state().unwrap().1, floor(1000));
    assert_eq!(destination.manifest_pair(100).unwrap().generation, 1);
    assert_eq!(
        destination_engine
            .get(ColumnFamily::Default, &physical(&f, b"l"))
            .unwrap(),
        Some(b"before-capture".to_vec())
    );
    assert_eq!(
        destination_engine
            .get(ColumnFamily::Write, &physical(&f, b"l"))
            .unwrap()
            .unwrap(),
        l_version.encode()
    );
    assert_eq!(
        destination_engine
            .get(ColumnFamily::Write, &physical(&f, b"e"))
            .unwrap()
            .unwrap(),
        e_version.encode()
    );
    for _ in 0..20 {
        pump(&[&source, &destination]);
        if destination
            .driver_applied()
            .is_some_and(|at| at.index >= tail.index)
        {
            break;
        }
    }
    assert!(destination.driver_applied().unwrap().index >= tail.index);
    assert_eq!(destination.raw_ttl_state().unwrap().1, floor(2000));
    let renewed = destination_engine
        .get(ColumnFamily::Write, &physical(&f, b"l"))
        .unwrap()
        .unwrap();
    assert_ne!(renewed, l_version.encode());
    let cleaned = commit(
        &source,
        &[&source, &destination],
        command(
            &f,
            RawTtlAction::Cleanup {
                candidates: vec![(b"e".to_vec(), e_version), (b"l".to_vec(), l_version)],
            },
        ),
    );
    pump(&[&source, &destination]);
    assert!(destination.driver_applied().unwrap().index >= cleaned.index);
    for engine in [&source_engine, &destination_engine] {
        assert_eq!(
            engine
                .get(ColumnFamily::Default, &physical(&f, b"e"))
                .unwrap(),
            None
        );
        assert_eq!(
            engine
                .get(ColumnFamily::Write, &physical(&f, b"e"))
                .unwrap(),
            None
        );
        assert_eq!(
            engine
                .get(ColumnFamily::Default, &physical(&f, b"l"))
                .unwrap(),
            Some(b"renewed-in-tail".to_vec())
        );
        assert_eq!(
            engine
                .get(ColumnFamily::Write, &physical(&f, b"l"))
                .unwrap()
                .unwrap(),
            renewed
        );
    }
    destination.stop();
    drop(destination);
    drop(destination_engine);
    drop(adopted);
    let reopened = adopt_for_runtime(&f.guard, f.identity, f.range.region, &uploader).unwrap();
    let (recovered_engine, recovered) = open_destination(&f, &reopened, &uploader, &hub);
    assert_eq!(
        recovered_engine.applied_position().unwrap(),
        DurableAppliedPosition::AppliedThrough(cleaned)
    );
    // Driver observation starts at the installed cut, not the engine's later
    // WAL position. It must consume committed tail history before advancing.
    assert_eq!(
        recovered.driver_applied().unwrap().index,
        reopened.cut.index
    );
    assert_eq!(recovered.raw_ttl_state().unwrap().1, floor(2000));
    assert_eq!(
        recovered_engine
            .get(ColumnFamily::Default, &physical(&f, b"e"))
            .unwrap(),
        None
    );
    assert_eq!(
        recovered_engine
            .get(ColumnFamily::Default, &physical(&f, b"l"))
            .unwrap(),
        Some(b"renewed-in-tail".to_vec())
    );
    assert_eq!(
        recovered_engine
            .get(ColumnFamily::Write, &physical(&f, b"l"))
            .unwrap()
            .unwrap(),
        renewed
    );
    assert_eq!(
        recovered_engine
            .get(ColumnFamily::Default, &physical(&f, b"p"))
            .unwrap(),
        Some(Vec::new())
    );
    pump(&[&source, &recovered]);
    assert!(recovered.driver_applied().unwrap().index >= cleaned.index);
    assert_eq!(recovered.raw_ttl_state().unwrap().1, floor(2000));
    // After adoption a real later publication may replace the installed
    // checkpoint. Its cut, rather than the original capture, can then become
    // the compacted protocol base. Preserve the publication in the suffix.
    let prepared = uploader
        .upload(
            source_engine
                .freeze(FlushScope {
                    cluster: f.identity.cluster_id.to_string(),
                    region: f.range.region.0,
                    conf_ver: f.range.conf_ver,
                    version: f.range.version,
                })
                .unwrap(),
        )
        .unwrap();
    let change = ManifestChangePayload::from_prepared(prepared, 1).unwrap();
    let later = CheckpointManifest::decode(&change.changeset).unwrap();
    let publication = commit(
        &source,
        &[&source, &recovered],
        Command::ManifestChange(change),
    );
    assert!(later.index > reopened.cut.index);
    assert!(recovered_engine.checkpoint_applied(&later).unwrap());
    recovered
        .peer()
        .compact_confirmed_prefix(
            later.position(),
            b"test-only committed compaction authority",
        )
        .unwrap();
    recovered.peer().reclaim_log().unwrap();
    recovered.stop();
    drop(recovered);
    drop(recovered_engine);
    drop(reopened);
    // Exercise upgrading a real old receipt after legitimate tail growth and
    // selection. The recorded floor must still come from the original image,
    // not the newer engine's clock. Only fixture code downgrades the frame.
    let marker_path = f.directory.join("data-groups/100/runtime-adopted");
    let verified = fs::read(&marker_path).unwrap();
    let mut old = b"KV9ADP01".to_vec();
    old.extend_from_slice(&verified[8..88]);
    fs::write(&marker_path, checksum(old)).unwrap();
    let upgraded = adopt_for_runtime(&f.guard, f.identity, f.range.region, &uploader).unwrap();
    let mut storage = DiskRaftStorage::recover(&upgraded.raft_directory).unwrap();
    let (engine, _, facts) = upgraded
        .open_engine_with_dependencies(&mut storage, &uploader)
        .unwrap();
    assert!(facts
        .installed()
        .unwrap()
        .verified_original_during_adoption());
    assert!(!facts.used_original_install_image());
    assert_eq!(
        facts.installed().unwrap().image().watermark(),
        Some(floor(1000))
    );
    assert_eq!(facts.selected().unwrap().watermark(), Some(floor(2000)));
    assert_eq!(
        facts.selected().unwrap().manifest(),
        RootDigest::sha256(&later.encode().unwrap())
    );
    drop(engine);
    drop(storage);
    drop(upgraded);
    assert_eq!(fs::read(&marker_path).unwrap(), verified);
    // Test-only physical removal, not a production GC decision. Shared files
    // remain; only objects absent from the successor closure may be removed.
    let mut removed = 0;
    for file in &original_manifest.files {
        if !later.files.iter().any(|current| current.key == file.key) {
            objects
                .delete(&ObjectKey::new(file.key.clone()).unwrap())
                .unwrap();
            removed += 1;
        }
    }
    assert!(removed > 0);
    assert!(uploader.restore(&original_manifest).is_err());
    let adopted = adopt_for_runtime(&f.guard, f.identity, f.range.region, &uploader).unwrap();
    let (engine, compacted) = open_destination(&f, &adopted, &uploader, &hub);
    assert_eq!(compacted.driver_applied().unwrap().index, later.index);
    assert_eq!(compacted.driver_applied().unwrap().term, later.term);
    assert_eq!(
        engine.applied_position().unwrap(),
        DurableAppliedPosition::AppliedThrough(publication)
    );
    pump(&[&source, &compacted]);
    assert!(compacted.driver_applied().unwrap().index >= publication.index);
    assert_eq!(compacted.raw_ttl_state().unwrap().1, floor(2000));
    assert_eq!(
        engine
            .get(ColumnFamily::Write, &physical(&f, b"l"))
            .unwrap()
            .unwrap(),
        renewed
    );
    compacted.stop();
    drop(compacted);
    drop(engine);
    drop(adopted);
    objects
        .delete(&ObjectKey::new(later.files[0].key.clone()).unwrap())
        .unwrap();
    let adopted = adopt_for_runtime(&f.guard, f.identity, f.range.region, &uploader).unwrap();
    let mut storage = DiskRaftStorage::recover(&adopted.raft_directory).unwrap();
    assert!(
        adopted.open_engine(&mut storage, &uploader).is_err(),
        "actual selected closure is still required"
    );
    source.stop();
}
