use super::*;
use kv9_common::StoreIdentity;
use kv9_engine::{Engine, MemoryObjectStore};

fn attach(group: &LiveGroup, node: u64) {
    let at = group.driver.add_learner(NodeId(node)).unwrap();
    for _ in 0..200 {
        group.driver.tick_and_step().unwrap();
        if group.driver.status().applied_index >= at.index.0 {
            break;
        }
    }
    group
        .driver
        .wait_conf_applied(at, Duration::from_secs(5))
        .unwrap();
    apply(group, &Command::Noop);
}
fn request(group: &LiveGroup) -> CaptureRequest {
    let source = StoreIdentity {
        cluster_id: group.cluster,
        node_id: NodeId(1),
        store_incarnation: StoreIncarnation::from_bytes([11; 16]),
        root_digest: group.range.root,
    };
    CaptureRequest {
        source,
        destination: StoreIdentity {
            node_id: NodeId(2),
            store_incarnation: StoreIncarnation::from_bytes([22; 16]),
            ..source
        },
        operation: [31; 16],
        range: group.range.clone(),
    }
}
fn plan(
    group: &LiveGroup,
    req: &CaptureRequest,
    uploader: &RemoteUploader,
) -> Result<PlannedCapture> {
    plan_capture_durable(
        &group.driver,
        &group.engine,
        req,
        uploader,
        true,
        |_, _, _| Ok(false),
    )
}

#[test]
fn durable_descriptor_is_bounded_canonical_data_and_verifies_every_remote_object() {
    let group = live_group();
    attach(&group, 2);
    put_row(&group, b"alpha", b"fixed");
    let objects = Arc::new(MemoryObjectStore::new());
    let uploader = RemoteUploader::for_harness(objects.clone());
    let planned = plan(&group, &request(&group), &uploader).unwrap();
    let descriptor = planned.descriptor().unwrap();
    assert_eq!(
        CaptureDescriptor::decode(descriptor.encode()).unwrap(),
        descriptor
    );
    for end in 0..descriptor.encode().len() {
        assert!(CaptureDescriptor::decode(&descriptor.encode()[..end]).is_err());
    }
    for span in [40..56, 168..184, 184..192] {
        let mut invalid = descriptor.encode().to_vec();
        invalid[span].fill(0);
        assert!(CaptureDescriptor::decode(&invalid).is_err());
    }
    assert!(CaptureDescriptor::decode(&vec![0; MAX_CAPTURE_DESCRIPTOR_BYTES + 1]).is_err());
    assert!(descriptor.verify_uploaded(&uploader).is_err());
    let captured = planned.upload(&uploader).unwrap();
    assert_eq!(descriptor.verify_uploaded(&uploader).unwrap(), captured);
    assert!(observe_durable_capture(
        &group.driver,
        &group.engine,
        request(&group).source,
        [99; 16],
        &uploader
    )
    .unwrap()
    .is_none());
}

#[test]
fn durable_plan_keeps_data_and_configuration_across_actual_source_reopen() {
    let mut group = live_group();
    attach(&group, 2);
    put_row(&group, b"alpha", b"original");
    let req = request(&group);
    let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()));
    let first = plan(&group, &req, &uploader).unwrap();
    let cut = first.cut();
    let record = first.record.clone();
    let manifest = first.manifest().clone();
    assert!(plan(&group, &req, &uploader)
        .unwrap_err()
        .to_string()
        .contains("locked"));
    drop(first);
    put_row(&group, b"alpha", b"changed");
    attach(&group, 3);
    let directory = group.directory.clone();
    let weak_driver = Arc::downgrade(&group.driver);
    let weak_engine = Arc::downgrade(&group.engine);
    group.preserve = true;
    drop(group);
    assert!(weak_driver.upgrade().is_none());
    assert!(weak_engine.upgrade().is_none());
    drop(uploader);
    // Fresh driver, full local WAL/Raft recovery, fresh uploader and budget.
    let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()));
    let mut storage = DiskRaftStorage::recover(&directory.join("raft")).unwrap();
    let (engine, _, _) = crate::state_machine::checkpoint_recovery::data_group::open(
        &mut storage,
        directory.join("data.wal"),
        Some(&uploader),
        crate::state_machine::checkpoint_recovery::data_group::DataCheckpointIdentity {
            cluster: req.source.cluster_id,
            root: req.range.root,
            region: req.range.region,
            creation: req.range.creation,
        },
    )
    .unwrap();
    let engine = Arc::new(engine);
    let mut state = MemStateMachine::with_engine(engine.clone()).unwrap();
    state
        .set_data_group(req.range.root, req.range.region, req.range.creation)
        .unwrap();
    let peer = Arc::new(RaftPeer::with_storage(NodeId(1), req.range.region, storage).unwrap());
    let driver = NodeDriver::new(peer, Arc::new(NullTransport), state).unwrap();
    driver.peer().campaign().unwrap();
    for _ in 0..100 {
        driver.tick_and_step().unwrap();
    }
    let group = LiveGroup {
        driver,
        engine,
        directory,
        range: req.range.clone(),
        cluster: req.source.cluster_id,
        preserve: false,
    };
    let fixed = plan(&group, &req, &uploader).unwrap();
    assert_eq!(fixed.cut(), cut);
    assert_eq!(fixed.manifest(), &manifest);
    assert_eq!(fixed.record, record);
    assert_eq!(fixed.configuration.learners, vec![2]);
    assert!(group.driver.status().learners.contains(&3));
    let captured = fixed.upload(&uploader).unwrap();
    assert_eq!(captured.record, record);
    let restored = uploader.restore(&manifest).unwrap();
    let key = encode_key(KeyMode::Raw, KeyspaceId(7), b"alpha").unwrap();
    assert_eq!(
        restored.get(ColumnFamily::Default, &key).unwrap(),
        Some(b"original".to_vec())
    );
    assert_eq!(
        group.engine.get(ColumnFamily::Default, &key).unwrap(),
        Some(b"changed".to_vec())
    );
}

#[test]
fn durable_plan_refuses_foreign_binding_and_requires_terminal_proof_to_reuse_slot() {
    let group = live_group();
    attach(&group, 2);
    let req = request(&group);
    let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()));
    let first = plan(&group, &req, &uploader).unwrap();
    let manifest = first.manifest().clone();
    let digest = first.image_digest;
    drop(first);
    let mut foreign = req.clone();
    foreign.source.store_incarnation = StoreIncarnation::mint().unwrap();
    assert!(plan_capture_durable(
        &group.driver,
        &group.engine,
        &foreign,
        &uploader,
        true,
        |_, _, _| Ok(true)
    )
    .is_err());
    foreign = req.clone();
    foreign.source.node_id = NodeId(9);
    assert!(plan(&group, &foreign, &uploader).is_err());
    let mut foreign = req.clone();
    foreign.destination.store_incarnation = StoreIncarnation::mint().unwrap();
    assert!(plan(&group, &foreign, &uploader).is_err());
    foreign = req.clone();
    foreign.range.version += 1;
    assert!(plan(&group, &foreign, &uploader).is_err());
    let mut next = req.clone();
    next.operation = [32; 16];
    assert!(plan(&group, &next, &uploader).is_err());
    assert!(plan_capture_durable(
        &group.driver,
        &group.engine,
        &next,
        &uploader,
        false,
        |_, _, _| Ok(true)
    )
    .is_err());
    put_row(&group, b"new", b"new");
    let replacement = plan_capture_durable(
        &group.driver,
        &group.engine,
        &next,
        &uploader,
        true,
        |old, selected, image_digest| {
            assert_eq!(old, &req);
            assert_eq!(selected, &manifest);
            assert_eq!(image_digest, digest);
            Ok(true) // The runtime tests exercise the committed metadata check.
        },
    )
    .unwrap();
    assert!(replacement.cut().index > manifest.index);
    drop(replacement);
    assert!(plan_capture_durable(
        &group.driver,
        &group.engine,
        &req,
        &uploader,
        false,
        |_, _, _| Ok(true)
    )
    .is_err());
    group.driver.stop();
    assert!(plan(&group, &next, &uploader).is_err());
}

#[test]
fn replica_reuses_original_record_and_requires_actual_healthy_voter_engine() {
    let group = live_group();
    attach(&group, 2);
    put_row(&group, b"alpha", b"original");
    let req = request(&group);
    let objects = Arc::new(MemoryObjectStore::new());
    let uploader = RemoteUploader::for_harness(objects.clone());
    let planned = plan(&group, &req, &uploader).unwrap();
    let descriptor = planned.descriptor().unwrap();
    drop(planned);
    check_capture_replica(&group.driver, &group.engine, req.source, &descriptor).unwrap();
    for local in [
        req.destination,
        StoreIdentity {
            root_digest: RootDigest::sha256(b"other root"),
            ..req.source
        },
        StoreIdentity {
            cluster_id: ClusterId::mint().unwrap(),
            ..req.source
        },
    ] {
        assert!(check_capture_replica(&group.driver, &group.engine, local, &descriptor).is_err());
    }
    let (foreign, _) = WalEngine::open(group.directory.join("other.wal")).unwrap();
    assert!(check_capture_replica(&group.driver, &foreign, req.source, &descriptor).is_err());
    drop(foreign);
    let original = uploader
        .transfer_journal(group.directory.join("capture.plan"))
        .unwrap();
    let reader = original.into_reader().unwrap().unwrap();
    let mut replica = uploader
        .transfer_journal(group.directory.join("capture-replica.plan"))
        .unwrap();
    replica
        .stage_received(descriptor.encode(), descriptor.manifest(), reader)
        .unwrap();
    drop(replica);
    put_row(&group, b"alpha", b"newer source state");
    check_capture_replica(&group.driver, &group.engine, req.source, &descriptor).unwrap();
    let replica = uploader
        .transfer_journal(group.directory.join("capture-replica.plan"))
        .unwrap();
    let resumed = descriptor.resume_replica(replica).unwrap();
    assert_eq!(resumed.descriptor().unwrap(), descriptor);
    let uploaded = resumed.upload(&uploader).unwrap();
    assert_eq!(uploaded, *descriptor.image());
    let restored = uploader.restore(descriptor.manifest()).unwrap();
    let key = encode_key(KeyMode::Raw, KeyspaceId(7), b"alpha").unwrap();
    assert_eq!(
        restored.get(ColumnFamily::Default, &key).unwrap(),
        Some(b"original".to_vec())
    );
    group.driver.stop();
    assert!(check_capture_replica(&group.driver, &group.engine, req.source, &descriptor).is_err());
}
