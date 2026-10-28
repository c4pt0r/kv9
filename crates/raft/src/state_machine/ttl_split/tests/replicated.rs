use super::*;
use crate::driver::{ApplyWaitOutcome, NodeDriver};
use crate::state_machine::checkpoint_recovery::data_group::{self, DataCheckpointIdentity};
use crate::storage::DiskRaftStorage;
use crate::transport::{InProcHub, RaftTransport};
use crate::{RaftGroup, RaftPeer, Role};
use kv9_common::{ClusterId, NodeId};
use std::time::Duration;

mod staging;

type Driver = Arc<NodeDriver<DiskRaftStorage, WalEngine>>;

#[test]
fn actual_source_snapshot_rejects_orphan_expiry_foreign_keys_and_unsealed_or_wrong_plans() {
    for fault in 0..5 {
        let directory = temp("source-factory");
        let p = plan();
        let source = Group::open(
            directory.join("source"),
            p.parent.clone(),
            ClusterId::mint().unwrap(),
            0,
        );
        source.submit(
            0,
            &Command::DataRange {
                expected: None,
                next: p.parent.clone(),
            },
        );
        source.submit(
            0,
            &Command::RawTtl(
                RawTtlCommand::new(
                    &p.parent,
                    RawTtlAction::Enable {
                        observed: ClockWatermark::default(),
                    },
                )
                .unwrap(),
            ),
        );
        source.submit(
            0,
            &write(
                &p.parent,
                b"a",
                b"expired but must be copied",
                Some(0),
                1000,
            ),
        );
        source.submit(0, &write(&p.parent, b"z", b"opaque high value", None, 1000));
        let deadline = || std::time::Instant::now() + Duration::from_secs(5);
        assert!(source.drivers[0]
            .capture_ttl_split_source(&p, deadline())
            .is_err());
        source.submit(0, &split(RawTtlSplitAction::Seal(p.clone())));
        let captured = source.drivers[1]
            .capture_ttl_split_source(&p, deadline())
            .unwrap();
        assert_eq!(captured.spec(false).rows, 1);
        assert_eq!(captured.spec(true).rows, 1);
        assert_ne!(captured.spec(false).digest, captured.spec(true).digest);
        assert!(source.drivers[0]
            .capture_ttl_split_source(&p, std::time::Instant::now())
            .is_err());
        let mut wrong = p.clone();
        wrong.operation[0] ^= 1;
        assert!(source.drivers[0]
            .capture_ttl_split_source(&wrong, deadline())
            .is_err());
        let mut corruption = kv9_engine::WriteBatch::new();
        let a =
            kv9_common::codec::encode_key(kv9_common::codec::KeyMode::Raw, p.parent.keyspace, b"a")
                .unwrap();
        match fault {
            0 => {
                // A well-formed expiry without its value is still forbidden.
                let expiry = read(&source.engines[0], ColumnFamily::Write, &a).unwrap();
                let missing = kv9_common::codec::encode_key(
                    kv9_common::codec::KeyMode::Raw,
                    p.parent.keyspace,
                    b"missing",
                )
                .unwrap();
                corruption.put(ColumnFamily::Write, missing, expiry)
            }
            1 => corruption.put(
                ColumnFamily::Default,
                vec![255, 255],
                b"outside scan".to_vec(),
            ),
            2 => corruption.put(ColumnFamily::Lock, a.clone(), b"unexpected lock".to_vec()),
            3 => corruption.put(ColumnFamily::Write, a.clone(), b"broken expiry".to_vec()),
            _ => corruption.put(
                ColumnFamily::Default,
                kv9_common::codec::encode_key(
                    kv9_common::codec::KeyMode::Raw,
                    kv9_common::KeyspaceId(p.parent.keyspace.0 + 1),
                    b"foreign",
                )
                .unwrap(),
                b"foreign value".to_vec(),
            ),
        };
        source.engines[0].write(corruption).unwrap();
        assert!(
            source.drivers[0]
                .capture_ttl_split_source(&p, deadline())
                .is_err(),
            "accepted source fault {fault}"
        );
        // The previously captured follower snapshot remains its original cut.
        assert_eq!(captured.spec(false).rows, 1);
        for driver in &source.drivers {
            driver.stop();
        }
        drop(captured);
        drop(source);
        std::fs::remove_dir_all(directory).unwrap();
    }
}

struct Group {
    drivers: Vec<Driver>,
    engines: Vec<Arc<WalEngine>>,
    range: DataRange,
    path: PathBuf,
    cluster: ClusterId,
    uploader: Option<Arc<kv9_engine::checkpoint::RemoteUploader>>,
}
impl Group {
    fn open(path: PathBuf, range: DataRange, cluster: ClusterId, leader: usize) -> Self {
        Self::open_with(path, range, cluster, leader, None)
    }
    fn open_with(
        path: PathBuf,
        range: DataRange,
        cluster: ClusterId,
        leader: usize,
        uploader: Option<Arc<kv9_engine::checkpoint::RemoteUploader>>,
    ) -> Self {
        let hub = InProcHub::new();
        let mut drivers = Vec::new();
        let mut engines = Vec::new();
        for n in 1..=3 {
            let mut storage = DiskRaftStorage::open(&path.join(format!("raft-{n}")), &[1, 2, 3])
                .unwrap()
                .0;
            let identity = DataCheckpointIdentity {
                cluster,
                root: range.root,
                region: range.region,
                creation: range.creation,
            };
            let engine = Arc::new(
                data_group::open(
                    &mut storage,
                    path.join(format!("engine-{n}")),
                    uploader.as_deref(),
                    identity,
                )
                .unwrap()
                .0,
            );
            engine.enable_segmentation().unwrap();
            let mut sm = MemStateMachine::with_engine(engine.clone()).unwrap();
            sm.set_data_group(range.root, range.region, range.creation)
                .unwrap();
            let peer = Arc::new(RaftPeer::with_storage(NodeId(n), range.region, storage).unwrap());
            drivers.push(
                NodeDriver::new(
                    peer,
                    Arc::new(hub.endpoint(NodeId(n))) as Arc<dyn RaftTransport>,
                    sm,
                )
                .unwrap(),
            );
            engines.push(engine);
        }
        drivers[leader].peer().campaign().unwrap();
        let g = Self {
            drivers,
            engines,
            range,
            path,
            cluster,
            uploader,
        };
        g.pump();
        assert_eq!(g.drivers[leader].status().role, Role::Leader);
        if matches!(
            ReplicatedEngine::applied_position(g.engines[leader].as_ref()).unwrap(),
            DurableAppliedPosition::AppliedNothing
        ) {
            g.submit(leader, &Command::Noop);
        }
        g
    }
    fn pump(&self) {
        for _ in 0..30 {
            for d in &self.drivers {
                d.step().unwrap();
            }
        }
    }
    fn submit(&self, leader: usize, command: &Command) -> ApplyWaitOutcome {
        // Low-level recovery/failure fixture only. Runtime integration tests
        // submit through the source-derived full-format gate instead.
        let at = if matches!(command, Command::RawTtlSplit(_) | Command::RawTtl(_)) {
            self.drivers[leader]
                .peer()
                .propose_raw_for_harness(command.encode())
                .unwrap()
        } else {
            self.drivers[leader].propose(command).unwrap()
        };
        self.pump();
        self.drivers[leader]
            .wait_applied(at, Duration::from_secs(1))
            .unwrap()
    }
    fn reopen(self, leader: usize) -> Self {
        let path = self.path.clone();
        let range = self.range.clone();
        let cluster = self.cluster;
        let uploader = self.uploader.clone();
        for d in &self.drivers {
            d.stop();
        }
        drop(self);
        Self::open_with(path, range, cluster, leader, uploader)
    }
}

#[test]
fn ttl_transfer_three_replicas_recover_mid_copy_then_finish_on_a_new_leader() {
    let directory = temp("replicated");
    let p = plan();
    let cluster = ClusterId::mint().unwrap();
    let source = Group::open(directory.join("source"), p.parent.clone(), cluster, 0);
    source.submit(
        0,
        &Command::DataRange {
            expected: None,
            next: p.parent.clone(),
        },
    );
    source.submit(
        0,
        &Command::RawTtl(
            RawTtlCommand::new(
                &p.parent,
                RawTtlAction::Enable {
                    observed: ClockWatermark::default(),
                },
            )
            .unwrap(),
        ),
    );
    for k in [b"a", b"b", b"z"] {
        source.submit(0, &write(&p.parent, k, k, Some(1), 1000));
    }
    source.submit(0, &split(RawTtlSplitAction::Seal(p.clone())));
    let seal =
        SplitSeal::decode(&read(&source.engines[0], ColumnFamily::Default, SEAL_KEY).unwrap())
            .unwrap();
    let rows = [
        captured(&source.engines[0], b"a"),
        captured(&source.engines[0], b"b"),
    ];
    let specification = spec(seal.clone(), false, &rows);
    let child = Group::open(directory.join("child"), p.low.clone(), cluster, 0);
    let empty = child.drivers[0].inspect_empty_ttl_import(&p.low).unwrap();
    assert_eq!(empty.target(), &p.low);
    let begin = split(RawTtlSplitAction::Begin {
        spec: specification,
        empty_at: empty.position(),
    });
    assert!(
        child.drivers[0].propose(&begin).is_err(),
        "ordinary runtime proposal must stay disabled"
    );
    assert!(matches!(
        child.submit(0, &begin),
        ApplyWaitOutcome::Applied(_)
    ));
    let first = split(RawTtlSplitAction::Append {
        target: p.low.clone(),
        expected: state(&child.engines[0]).digest(),
        rows: vec![rows[0].clone()],
    });
    assert!(matches!(
        child.submit(0, &first),
        ApplyWaitOutcome::Applied(_)
    ));
    let partial = state(&child.engines[0]);
    assert!(child.drivers[0].inspect_empty_ttl_import(&p.low).is_err());
    image_roundtrip(&child, false);
    let child = child.reopen(1);
    for engine in &child.engines {
        assert_eq!(state(engine), partial);
        assert!(read(engine, ColumnFamily::Default, RANGE_KEY).is_none());
        assert_eq!(
            read(engine, ColumnFamily::Write, &rows[0].key),
            Some(rows[0].expiry.unwrap().encode().to_vec())
        );
    }
    assert!(matches!(
        child.submit(1, &first),
        ApplyWaitOutcome::FenceRejected { .. }
    ));
    assert!(matches!(
        child.submit(1, &begin),
        ApplyWaitOutcome::Applied(_)
    ));
    child.submit(
        1,
        &split(RawTtlSplitAction::Append {
            target: p.low.clone(),
            expected: partial.digest(),
            rows: vec![rows[1].clone()],
        }),
    );
    let finish = split(RawTtlSplitAction::Finish {
        target: p.low.clone(),
        expected: state(&child.engines[0]).digest(),
    });
    assert!(matches!(
        child.submit(1, &finish),
        ApplyWaitOutcome::Applied(_)
    ));
    image_roundtrip(&child, true);
    child.submit(1, &write(&p.low, b"a", b"renewed", Some(60), 3000));
    let cleanup = Command::RawTtl(
        RawTtlCommand::new(
            &p.low,
            RawTtlAction::Cleanup {
                candidates: rows
                    .iter()
                    .map(|r| (r.key[4..].to_vec(), r.expiry.unwrap()))
                    .collect(),
            },
        )
        .unwrap(),
    );
    child.submit(1, &cleanup);
    assert!(matches!(
        child.submit(1, &first),
        ApplyWaitOutcome::FenceRejected { .. }
    ));
    child.submit(1, &finish);
    let child = child.reopen(2);
    let source = source.reopen(1);
    for engine in &source.engines {
        assert_eq!(
            SplitSeal::decode(&read(engine, ColumnFamily::Default, SEAL_KEY).unwrap()).unwrap(),
            seal
        );
        assert_eq!(
            read(engine, ColumnFamily::Default, RANGE_KEY),
            Some(p.sealed_range().unwrap().encode())
        );
    }
    for engine in &child.engines {
        assert!(state(engine).complete);
        assert_eq!(
            read(engine, ColumnFamily::Default, &key(b"a")),
            Some(b"renewed".to_vec())
        );
        assert_eq!(read(engine, ColumnFamily::Default, &key(b"b")), None);
        assert_eq!(read(engine, ColumnFamily::Write, &key(b"b")), None);
        assert_eq!(
            ClockWatermark::decode(&read(engine, ColumnFamily::Default, CLOCK_KEY).unwrap())
                .unwrap()
                .unix_ms(),
            3000
        );
    }
    drop((source, child));
    std::fs::remove_dir_all(directory).unwrap();
}

fn image_roundtrip(group: &Group, complete: bool) {
    use kv9_engine::checkpoint::{FlushScope, RemoteUploader};
    let uploader = RemoteUploader::for_harness(Arc::new(kv9_engine::MemoryObjectStore::new()));
    let r = &group.range;
    let manifest = uploader
        .upload(
            group.engines[0]
                .freeze(FlushScope {
                    cluster: group.cluster.to_string(),
                    region: r.region.0,
                    conf_ver: r.conf_ver,
                    version: r.version,
                })
                .unwrap(),
        )
        .unwrap()
        .into_manifest();
    let restored = uploader.restore(&manifest).unwrap();
    let view = Engine::snapshot(&restored).unwrap();
    let result = data_group::inspect_base(
        view.as_ref(),
        &manifest,
        DataCheckpointIdentity {
            cluster: group.cluster,
            root: r.root,
            region: r.region,
            creation: r.creation,
        },
    );
    assert_eq!(
        result.is_ok(),
        complete,
        "only a completed import has an installable range image"
    );
    assert_eq!(
        ImportState::decode(
            &view
                .get(ColumnFamily::Default, IMPORT_KEY)
                .unwrap()
                .unwrap()
        )
        .unwrap(),
        state(&group.engines[0])
    );
}

#[test]
fn ttl_import_empty_observation_refuses_dirty_state_and_orders_against_new_writes() {
    use crate::KvOp;
    let directory = temp("empty-proof");
    let p = plan();
    let cluster = ClusterId::mint().unwrap();
    let group = Group::open(directory.join("child"), p.low.clone(), cluster, 0);
    let empty = group.drivers[0].inspect_empty_ttl_import(&p.low).unwrap();
    let mut wrong = p.low.clone();
    wrong.creation = RootDigest::from_bytes([19; 32]);
    assert!(group.drivers[0].inspect_empty_ttl_import(&wrong).is_err());
    group.submit(
        0,
        &Command::Write {
            ops: vec![KvOp::Put {
                cf: 0,
                key: key(b"a"),
                value: b"racing legacy copy".to_vec(),
            }],
        },
    );
    assert!(group.drivers[0].inspect_empty_ttl_import(&p.low).is_err());
    let seal = SplitSeal {
        plan: p.clone(),
        cut: AppliedPosition { term: 2, index: 50 },
        watermark: ClockWatermark::default(),
    };
    assert!(matches!(
        group.submit(
            0,
            &split(RawTtlSplitAction::Begin {
                spec: spec(seal, false, &[]),
                empty_at: empty.position()
            })
        ),
        ApplyWaitOutcome::FenceRejected { .. }
    ));
    assert!(read(&group.engines[0], ColumnFamily::Default, CLOCK_KEY).is_none());
    assert_eq!(
        read(&group.engines[0], ColumnFamily::Default, &key(b"a")),
        Some(b"racing legacy copy".to_vec())
    );
    group.submit(
        0,
        &Command::Write {
            ops: vec![KvOp::Delete {
                cf: 0,
                key: key(b"a"),
            }],
        },
    );
    let next = group.drivers[0].inspect_empty_ttl_import(&p.low).unwrap();
    assert!(next.position().index > empty.position().index);
    group.submit(
        0,
        &Command::Write {
            ops: vec![KvOp::Put {
                cf: crate::cf_code(ColumnFamily::Lock),
                key: b"foreign".to_vec(),
                value: b"lock".to_vec(),
            }],
        },
    );
    assert!(group.drivers[0].inspect_empty_ttl_import(&p.low).is_err());
    drop(group);
    std::fs::remove_dir_all(directory).unwrap();
}
