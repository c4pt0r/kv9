use std::sync::Arc;
use std::time::Duration;

use kv9_common::codec::{encode_key, KeyMode};
use kv9_common::data_range::DataRange;
use kv9_common::{ClusterId, KeyspaceId, NodeId, RegionId, RootDigest, StoreIncarnation, TenantId};
use kv9_engine::{ColumnFamily, ReplicatedEngine, WalEngine, WriteBatch};

use super::*;
use crate::command::RegionFence;
use crate::driver::NodeDriver;
use crate::rawnode::RaftPeer;
use crate::snapshot_install::JointInstaller;
use crate::transport::RaftTransport;
use crate::RaftGroup;
use crate::{Command, MemStateMachine, Role};

struct NullTransport;
impl RaftTransport for NullTransport {
    fn send(&self, _: NodeId, _: raft::prelude::Message) {}
    fn drain(&self) -> Vec<raft::prelude::Message> {
        Vec::new()
    }
}

struct LiveGroup {
    directory: std::path::PathBuf,
    driver: Arc<NodeDriver<DiskRaftStorage, WalEngine>>,
    engine: Arc<WalEngine>,
    range: DataRange,
    cluster: ClusterId,
}

impl Drop for LiveGroup {
    fn drop(&mut self) {
        self.driver.stop();
        if !std::thread::panicking() {
            let _ = std::fs::remove_dir_all(&self.directory);
        } else {
            eprintln!("retained capture fixture: {}", self.directory.display());
        }
    }
}

fn live_group() -> LiveGroup {
    live_group_with(
        RootDigest::sha256(b"capture-root"),
        ClusterId::from_bytes([9; 16]),
    )
}

fn live_group_with(root: RootDigest, cluster: ClusterId) -> LiveGroup {
    let directory =
        std::env::temp_dir().join(format!("kv9-capture-{}", StoreIncarnation::mint().unwrap()));
    std::fs::create_dir_all(&directory).unwrap();
    let (storage, pristine) = DiskRaftStorage::open(&directory.join("raft"), &[1]).unwrap();
    assert!(pristine);
    let peer = Arc::new(RaftPeer::with_storage(NodeId(1), RegionId(100), storage).unwrap());
    let (engine, _) = WalEngine::open(directory.join("data.wal")).unwrap();
    let engine = Arc::new(engine);
    let creation = RootDigest::sha256(b"capture-creation");
    let mut state = MemStateMachine::with_engine(engine.clone()).unwrap();
    state.set_data_group(root, RegionId(100), creation).unwrap();
    let transport: Arc<dyn RaftTransport> = Arc::new(NullTransport);
    let driver = NodeDriver::new(peer, transport, state).unwrap();
    driver.peer().campaign().unwrap();
    for _ in 0..100 {
        driver.tick_and_step().unwrap();
        if driver.status().role == Role::Leader {
            break;
        }
    }
    assert_eq!(driver.status().role, Role::Leader);
    let range = DataRange {
        root,
        creation,
        region: RegionId(100),
        keyspace: KeyspaceId(7),
        tenant: TenantId(1),
        conf_ver: 1,
        version: 1,
        start: Vec::new(),
        end: Vec::new(),
        sealed: false,
    };
    let group = LiveGroup {
        directory,
        driver,
        engine,
        range: range.clone(),
        cluster,
    };
    apply(
        &group,
        &Command::DataRange {
            expected: None,
            next: range,
        },
    );
    group
}

fn apply(group: &LiveGroup, command: &Command) {
    let proposed = group.driver.propose(command).unwrap();
    for _ in 0..200 {
        group.driver.tick_and_step().unwrap();
        if group
            .driver
            .driver_applied()
            .is_some_and(|p| p.index >= proposed.index.0)
        {
            break;
        }
    }
    group
        .driver
        .wait_applied(proposed, Duration::from_secs(5))
        .unwrap();
}

fn put_row(group: &LiveGroup, key: &[u8], value: &[u8]) {
    let mut batch = WriteBatch::new();
    batch.put(
        ColumnFamily::Default,
        encode_key(KeyMode::Raw, KeyspaceId(7), key).unwrap(),
        value.to_vec(),
    );
    apply(
        group,
        &Command::fenced_write_from_batch(
            RegionFence {
                region_id: 100,
                conf_ver: 1,
                version: 1,
            },
            &batch,
        ),
    );
}

#[test]
fn capture_binds_cut_scope_range_and_initial_configuration() {
    let group = live_group();
    put_row(&group, b"alpha", b"1");
    put_row(&group, b"beta", b"2");
    let planned = plan_capture(&group.driver, &group.engine, group.cluster).unwrap();
    let engine_cut = match group.engine.applied_position().unwrap() {
        kv9_engine::DurableAppliedPosition::AppliedThrough(at) => at,
        other => panic!("unexpected engine position {other:?}"),
    };
    assert_eq!(planned.cut(), engine_cut);
    let manifest = planned.manifest();
    assert_eq!(manifest.index, engine_cut.index);
    assert_eq!(manifest.term, engine_cut.term);
    assert_eq!(manifest.scope.cluster, group.cluster.to_string());
    assert_eq!(manifest.scope.region, 100);
    assert_eq!(manifest.scope.conf_ver, 1);
    assert_eq!(manifest.scope.version, 1);
    assert_eq!(planned.range(), &group.range);
    // The canonical record round-trips through the exact installer codec and
    // carries the initial single-voter configuration with an empty vote.
    let (image, hs) = snapshot::decode(&planned.record).unwrap();
    assert_eq!(image.get_metadata().index, engine_cut.index);
    assert_eq!(image.get_metadata().term, engine_cut.term);
    assert_eq!(image.get_metadata().get_conf_state().voters, vec![1]);
    assert!(image.get_metadata().get_conf_state().learners.is_empty());
    assert_eq!(hs.commit, engine_cut.index);
    assert_eq!(hs.term, engine_cut.term);
    assert_eq!(hs.vote, 0);
    let (parsed_range, parsed_manifest) = super::super::parse_image(&image).unwrap();
    assert_eq!(parsed_range, group.range);
    assert_eq!(parsed_manifest.index, engine_cut.index);
}

#[test]
fn capture_attaches_the_configuration_at_the_cut_not_a_later_one() {
    let group = live_group();
    put_row(&group, b"alpha", b"1");
    let before = plan_capture(&group.driver, &group.engine, group.cluster).unwrap();
    // Commit a configuration change PAST the engine cut: the driver applies
    // it (driver_applied advances) while the engine watermark stays put.
    let conf = group.driver.add_learner(NodeId(2)).unwrap();
    for _ in 0..200 {
        group.driver.tick_and_step().unwrap();
        if group
            .driver
            .driver_applied()
            .is_some_and(|p| p.index >= conf.index.0)
        {
            break;
        }
    }
    group
        .driver
        .wait_conf_applied(conf, Duration::from_secs(5))
        .unwrap();
    let lagging = plan_capture(&group.driver, &group.engine, group.cluster).unwrap();
    assert_eq!(lagging.cut(), before.cut(), "engine cut must not move");
    let (image, _) = snapshot::decode(&lagging.record).unwrap();
    assert!(
        image.get_metadata().get_conf_state().learners.is_empty(),
        "a configuration committed past the cut must not attach to the image"
    );
    // Once a later data command moves the engine cut past the configuration
    // entry, the capture attaches the NEW configuration.
    put_row(&group, b"beta", b"2");
    let advanced = plan_capture(&group.driver, &group.engine, group.cluster).unwrap();
    assert!(advanced.cut().index > conf.index.0);
    let (image, _) = snapshot::decode(&advanced.record).unwrap();
    assert_eq!(image.get_metadata().get_conf_state().learners, vec![2]);
    assert_eq!(
        advanced.configuration_applied_at.map(|p| p.index),
        Some(conf.index.0),
        "the attached configuration must name its own commit position"
    );
}

#[test]
fn capture_refuses_groups_without_applied_ownership() {
    let group = live_group();
    let bare = std::env::temp_dir().join(format!(
        "kv9-capture-bare-{}",
        StoreIncarnation::mint().unwrap()
    ));
    let (bare_engine, _) = WalEngine::open(bare.join("data.wal")).unwrap();
    let error = plan_capture(&group.driver, &bare_engine, group.cluster).unwrap_err();
    assert!(
        error.to_string().contains("nothing applied")
            || error.to_string().contains("no applied range ownership"),
        "{error}"
    );
    std::fs::remove_dir_all(&bare).ok();
}

/// The full component loop: a live group captures at its cut with a learner
/// already in the committed configuration, and the offline joint installer
/// installs that exact image at the learner's prepared store, value-verified.
/// The destination must appear in the image configuration — capturing BEFORE
/// the learner is committed is refused by the unchanged installer gate.
#[test]
#[ignore = "requires an isolated real MinIO bucket and environment credentials"]
fn real_minio_captured_image_installs_at_a_learner_destination() {
    use kv9_common::root::{RootDescriptor, RootVoter};
    use kv9_common::store_lifecycle::StoreGuard;
    use kv9_common::{BootstrapGeneration, StoreIdentity};
    use kv9_engine::minio::{MinioConfig, MinioObjectStore};
    use kv9_engine::Engine;

    let uploader = kv9_engine::checkpoint::RemoteUploader::new(Arc::new(
        MinioObjectStore::connect(MinioConfig::from_env().unwrap()).unwrap(),
    ));
    // One root binds source range and destination store. Root voters are the
    // metadata concept; the group's own Raft configuration is independent.
    let destination_dir = std::env::temp_dir().join(format!(
        "kv9-capture-dest-{}",
        StoreIncarnation::mint().unwrap()
    ));
    let mut destination_guard = StoreGuard::lock(&destination_dir).unwrap();
    let prepared = destination_guard.prepare(NodeId(2)).unwrap();
    let root = RootDescriptor::new(
        ClusterId::mint().unwrap(),
        BootstrapGeneration::mint().unwrap(),
        vec![RootVoter {
            node_id: NodeId(2),
            addr: "127.0.0.1:19442".parse().unwrap(),
            store_incarnation: prepared.incarnation,
        }],
        b"capture-loop-fixture",
    )
    .unwrap();
    let identity = StoreIdentity::for_voter(&root, NodeId(2)).unwrap();
    destination_guard.bind(&root, &identity).unwrap();

    let group = live_group_with(root.digest(), root.cluster_id);
    put_row(&group, b"alpha", b"first");
    put_row(&group, b"beta", b"second");
    // Capturing before the learner exists must be refused BY THE INSTALLER:
    // the capture itself is a valid description of a learnerless group.
    let premature = plan_capture(&group.driver, &group.engine, group.cluster).unwrap();
    let premature = premature.upload(&uploader).unwrap();
    let conf = group.driver.add_learner(NodeId(2)).unwrap();
    for _ in 0..200 {
        group.driver.tick_and_step().unwrap();
        if group
            .driver
            .driver_applied()
            .is_some_and(|p| p.index >= conf.index.0)
        {
            break;
        }
    }
    group
        .driver
        .wait_conf_applied(conf, Duration::from_secs(5))
        .unwrap();
    put_row(&group, b"gamma", b"third");
    let captured = plan_capture(&group.driver, &group.engine, group.cluster)
        .unwrap()
        .upload(&uploader)
        .unwrap();
    assert_eq!(captured.configuration.learners, vec![2]);

    let mut installer =
        JointInstaller::open(&destination_guard, identity, group.range.clone()).unwrap();
    let (old_image, old_hs) = snapshot::decode(&premature.record).unwrap();
    let refused = installer
        .install(&old_image, &old_hs, &uploader)
        .unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("does not match the destination scope or membership"),
        "{refused}"
    );
    // The poisoned owner must be reopened; then the learner-bearing image
    // installs, recovers, and every value reads back from the restored engine.
    drop(installer);
    let mut installer =
        JointInstaller::open(&destination_guard, identity, group.range.clone()).unwrap();
    let (image, hs) = snapshot::decode(&captured.record).unwrap();
    let installed = installer.install(&image, &hs, &uploader).unwrap();
    assert_eq!(installed.position, captured.cut);
    assert_eq!(
        installed.records, 3,
        "user rows only; RANGE_KEY is excluded"
    );
    let recovered = installer.recover(&uploader).unwrap().unwrap();
    assert_eq!(recovered.image_digest, captured.image_digest);
    let generation = recovered.generation;
    drop(installer);
    let engine_path = destination_dir.join(format!(
        "data-groups/{}/install-{generation}/data.wal",
        group.range.region.0
    ));
    let (engine, _) = WalEngine::open_with_uploader(engine_path, Some(&uploader)).unwrap();
    for (key, value) in [
        (&b"alpha"[..], &b"first"[..]),
        (b"beta", b"second"),
        (b"gamma", b"third"),
    ] {
        assert_eq!(
            engine
                .get(
                    ColumnFamily::Default,
                    &encode_key(KeyMode::Raw, KeyspaceId(7), key).unwrap()
                )
                .unwrap()
                .as_deref(),
            Some(value),
        );
    }
    drop(destination_guard);
    std::fs::remove_dir_all(&destination_dir).ok();
}
