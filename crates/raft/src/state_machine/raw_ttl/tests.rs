use super::*;
use crate::command::{RawTtlAction as Action, RawTtlMutation as Mutation};
use crate::driver::{ApplyWaitOutcome, NodeDriver};
use crate::transport::{InProcHub, RaftTransport};
use crate::{RaftGroup, RaftPeer, Role};
use kv9_common::raw_ttl::{ClockSample, ExpiryPlan};
use kv9_common::NodeId;
use kv9_common::{KeyspaceId, RootDigest, TenantId};
use kv9_engine::{Engine, WalEngine};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::Duration;

fn range() -> DataRange {
    DataRange {
        root: RootDigest::from_bytes([1; 32]),
        creation: RootDigest::from_bytes([2; 32]),
        region: RegionId(100),
        keyspace: KeyspaceId(100),
        tenant: TenantId(1),
        conf_ver: 1,
        version: 1,
        start: b"a".to_vec(),
        end: b"z".to_vec(),
        sealed: false,
    }
}
fn floor(ms: u64) -> ClockWatermark {
    ClockSample::with_uncertainty(ms, 0)
        .unwrap()
        .advance(ClockWatermark::default())
        .unwrap()
}
fn plan(ttl: Option<u64>, ms: u64) -> ExpiryPlan {
    ExpiryPlan::resolve(
        ttl,
        ClockSample::with_uncertainty(ms, 0).unwrap(),
        ClockWatermark::default(),
    )
    .unwrap()
}
fn cmd(action: Action) -> Command {
    Command::RawTtl(RawTtlCommand::new(&range(), action).unwrap())
}
fn put(key: &[u8], value: &[u8]) -> Mutation {
    Mutation::Put {
        key: key.to_vec(),
        value: value.to_vec(),
    }
}
fn write(ttl: Option<u64>, ms: u64, mutations: Vec<Mutation>) -> Command {
    cmd(Action::Write {
        plan: plan(ttl, ms),
        mutations,
    })
}
fn physical(key: &[u8]) -> Vec<u8> {
    encode_key(KeyMode::Raw, range().keyspace, key).unwrap()
}
fn apply<E: ApplyStore>(sm: &mut MemStateMachine<E>, command: &Command) -> ApplyResult {
    sm.apply(&CommittedEntry {
        index: LogIndex(sm.applied_index().0 + 1),
        term: 3,
        kind: crate::EntryKind::Command,
        data: command.encode(),
    })
    .unwrap()
}
fn setup<E: ApplyStore + 'static>(engine: Arc<E>) -> MemStateMachine<E> {
    let r = range();
    let mut sm = MemStateMachine::with_engine(engine).unwrap();
    sm.set_data_group(r.root, r.region, r.creation).unwrap();
    if sm.applied_index().0 == 0 {
        apply(
            &mut sm,
            &Command::DataRange {
                expected: None,
                next: r,
            },
        );
    }
    sm
}
fn enabled() -> MemStateMachine {
    let mut sm = setup(Arc::new(MemEngine::new()));
    apply(
        &mut sm,
        &cmd(Action::Enable {
            observed: floor(100),
        }),
    );
    sm
}
fn expiry<E: ApplyStore>(sm: &MemStateMachine<E>, key: &[u8]) -> Option<ExpiryRecord> {
    sm.get(ColumnFamily::Write, &physical(key))
        .unwrap()
        .map(|b| ExpiryRecord::decode(&b).unwrap())
}
fn value<E: ApplyStore>(sm: &MemStateMachine<E>, key: &[u8]) -> Option<Vec<u8>> {
    sm.get(ColumnFamily::Default, &physical(key)).unwrap()
}
fn clock<E: ApplyStore>(sm: &MemStateMachine<E>) -> ClockWatermark {
    ClockWatermark::decode(&sm.get(ColumnFamily::Default, CLOCK_KEY).unwrap().unwrap()).unwrap()
}
fn legacy() -> Vec<Command> {
    let op = KvOp::Put {
        cf: 0,
        key: physical(b"b"),
        value: b"bypass".to_vec(),
    };
    vec![
        Command::Put {
            cf: 0,
            key: physical(b"b"),
            value: b"bypass".to_vec(),
        },
        Command::Write {
            ops: vec![op.clone()],
        },
        Command::CatalogTxn {
            ops: vec![op.clone()],
        },
        Command::Fenced {
            fence: RawTtlCommand::new(&range(), Action::Enable { observed: floor(0) })
                .unwrap()
                .fence(),
            inner: FencedInner::Write { ops: vec![op] },
        },
    ]
}

#[test]
fn ttl_activation_is_a_barrier_and_all_legacy_writes_are_refused() {
    let mut sm = setup(Arc::new(MemEngine::new()));
    let candidate = plan(Some(1), 100)
        .at_write(range().creation, AppliedPosition { term: 3, index: 1 })
        .unwrap()
        .unwrap();
    for command in [
        cmd(Action::Observe {
            observed: floor(200),
        }),
        write(Some(1), 100, vec![put(b"b", b"value")]),
        cmd(Action::Cleanup {
            candidates: vec![(b"b".to_vec(), candidate)],
        }),
    ] {
        assert_eq!(
            apply(&mut sm, &command).outcome,
            ApplyOutcome::RawTtlRejected {
                region: range().region,
                reason: TtlRejection::NotEnabled
            }
        );
        assert_eq!(value(&sm, b"b"), None);
    }
    assert!(sm.can_group_raw(&legacy()[0]));
    let enable = cmd(Action::Enable {
        observed: floor(100),
    });
    assert!(!sm.can_group_raw(&enable));
    apply(&mut sm, &enable);
    apply(&mut sm, &write(Some(1), 100, vec![put(b"b", b"original")]));
    let original = expiry(&sm, b"b");
    for command in legacy() {
        assert!(!sm.can_group_raw(&command));
        assert_eq!(
            apply(&mut sm, &command).outcome,
            ApplyOutcome::RawTtlRejected {
                region: range().region,
                reason: TtlRejection::LegacyCommand
            }
        );
        assert_eq!(value(&sm, b"b"), Some(b"original".to_vec()));
        assert_eq!(expiry(&sm, b"b"), original);
        assert_eq!(clock(&sm), floor(100));
    }
    let before = sm.applied_index();
    assert!(sm
        .apply_raw_group(&[(
            AppliedPosition {
                term: 3,
                index: before.0 + 1
            },
            legacy().remove(0)
        )])
        .is_err());
    assert_eq!(sm.applied_index(), before);
    apply(&mut sm, &Command::Noop);
}

#[test]
fn ttl_reserved_records_cannot_be_forged_before_activation() {
    for op in [
        KvOp::Put {
            cf: 0,
            key: CLOCK_KEY.to_vec(),
            value: floor(100).encode().to_vec(),
        },
        KvOp::Delete {
            cf: 0,
            key: CLOCK_KEY.to_vec(),
        },
        KvOp::Put {
            cf: crate::cf_code(ColumnFamily::Write),
            key: physical(b"b"),
            value: vec![],
        },
        KvOp::Delete {
            cf: crate::cf_code(ColumnFamily::Write),
            key: physical(b"b"),
        },
    ] {
        let mut sm = setup(Arc::new(MemEngine::new()));
        let before = sm.applied_index();
        let command = Command::Write {
            ops: vec![
                KvOp::Put {
                    cf: 0,
                    key: physical(b"c"),
                    value: vec![1],
                },
                op,
            ],
        };
        assert!(!sm.can_group_raw(&command));
        assert!(sm
            .apply_at(
                AppliedPosition {
                    term: 3,
                    index: before.0 + 1
                },
                &command
            )
            .is_err());
        assert_eq!(value(&sm, b"c"), None);
        assert_eq!(sm.applied_index(), before);
        assert!(!sm.ttl_enabled);
    }
}

#[test]
fn ttl_writes_keep_deadlines_monotonic_time_and_atomic_duplicate_key_semantics() {
    let mut sm = enabled();
    let command = write(Some(1), 100, vec![put(b"b", b"KV9TTL01\0arbitrary")]);
    apply(&mut sm, &command);
    let first = expiry(&sm, b"b").unwrap();
    assert_eq!(
        first,
        plan(Some(1), 100)
            .at_write(range().creation, AppliedPosition { term: 3, index: 3 })
            .unwrap()
            .unwrap()
    );
    // Same-position replay cannot write or change the expiry identity.
    sm.apply_at(AppliedPosition { term: 3, index: 3 }, &command)
        .unwrap();
    assert_eq!(expiry(&sm, b"b"), Some(first));
    apply(
        &mut sm,
        &cmd(Action::Observe {
            observed: floor(5000),
        }),
    );
    // A delayed/reproposed command keeps its original deadline, even at a new
    // index. Its new write identity prevents old cleanup from deleting it.
    apply(&mut sm, &command);
    let delayed = expiry(&sm, b"b").unwrap();
    assert_ne!(delayed, first);
    assert!(delayed.expired_at(clock(&sm)));
    assert_eq!(
        delayed,
        plan(Some(1), 100)
            .at_write(
                range().creation,
                AppliedPosition {
                    term: 3,
                    index: sm.applied_index().0
                }
            )
            .unwrap()
            .unwrap()
    );
    apply(&mut sm, &cmd(Action::Enable { observed: floor(0) }));
    assert_eq!(clock(&sm), floor(5000));
    apply(&mut sm, &write(Some(0), 100, vec![put(b"c", b"")]));
    assert!(expiry(&sm, b"c").unwrap().expired_at(clock(&sm)));
    apply(
        &mut sm,
        &write(
            None,
            200,
            vec![
                put(b"b", b"permanent"),
                Mutation::Delete { key: b"c".to_vec() },
            ],
        ),
    );
    assert_eq!(expiry(&sm, b"b"), None);
    assert_eq!(value(&sm, b"b"), Some(b"permanent".to_vec()));
    assert_eq!((value(&sm, b"c"), expiry(&sm, b"c")), (None, None));
    apply(
        &mut sm,
        &write(
            Some(2),
            5000,
            vec![
                put(b"d", b"old"),
                Mutation::Delete { key: b"d".to_vec() },
                put(b"d", b"last"),
                put(b"e", b"gone"),
                Mutation::Delete { key: b"e".to_vec() },
            ],
        ),
    );
    assert_eq!(value(&sm, b"d"), Some(b"last".to_vec()));
    assert!(expiry(&sm, b"d").is_some());
    assert_eq!((value(&sm, b"e"), expiry(&sm, b"e")), (None, None));
    assert_eq!(clock(&sm), floor(5000));
}

#[test]
fn ttl_cleanup_rechecks_expiry_entire_identity_and_permanent_overwrites() {
    let mut sm = enabled();
    let write = write(Some(1), 100, vec![put(b"b", b"first")]);
    apply(&mut sm, &write);
    let first = expiry(&sm, b"b").unwrap();
    let cleanup = |record| {
        cmd(Action::Cleanup {
            candidates: vec![(b"b".to_vec(), record); 2],
        })
    };
    apply(&mut sm, &cleanup(first));
    assert_eq!(value(&sm, b"b"), Some(b"first".to_vec()));
    apply(
        &mut sm,
        &cmd(Action::Observe {
            observed: floor(1100),
        }),
    );
    apply(&mut sm, &write);
    let second = expiry(&sm, b"b").unwrap();
    let foreign = ExpiryRecord::new(
        RootDigest::from_bytes([9; 32]),
        AppliedPosition {
            term: 3,
            index: sm.applied_index().0,
        },
        1100,
    )
    .unwrap();
    for record in [first, foreign] {
        apply(&mut sm, &cleanup(record));
        assert_eq!(expiry(&sm, b"b"), Some(second));
        assert_eq!(value(&sm, b"b"), Some(b"first".to_vec()));
    }
    apply(&mut sm, &cleanup(second));
    assert_eq!((value(&sm, b"b"), expiry(&sm, b"b")), (None, None));
    apply(&mut sm, &cleanup(second));
    apply(
        &mut sm,
        &super::tests::write(None, 1100, vec![put(b"b", b"permanent")]),
    );
    apply(&mut sm, &cleanup(second));
    assert_eq!(value(&sm, b"b"), Some(b"permanent".to_vec()));
}

#[test]
fn ttl_apply_revalidates_scope_bounds_and_seal_before_any_batch_effect() {
    let mut sm = enabled();
    let mut scopes = Vec::new();
    for field in 0..4 {
        let mut scope = range();
        match field {
            0 => scope.root = RootDigest::from_bytes([8; 32]),
            1 => scope.creation = RootDigest::from_bytes([8; 32]),
            2 => scope.region = RegionId(101),
            _ => scope.keyspace = KeyspaceId(101),
        }
        scopes.push(scope);
    }
    for scope in scopes {
        let command = Command::RawTtl(
            RawTtlCommand::new(
                &scope,
                Action::Write {
                    plan: plan(Some(1), 9000),
                    mutations: vec![put(b"b", b"bad")],
                },
            )
            .unwrap(),
        );
        assert_eq!(
            apply(&mut sm, &command).outcome,
            ApplyOutcome::RawTtlRejected {
                region: scope.region,
                reason: TtlRejection::InvalidScope
            }
        );
    }
    for field in 0..3 {
        let mut scope = range();
        match field {
            0 => scope.conf_ver += 1,
            1 => scope.version += 1,
            _ => scope.end = b"zz".to_vec(),
        }
        let command = Command::RawTtl(
            RawTtlCommand::new(
                &scope,
                Action::Write {
                    plan: plan(Some(1), 9000),
                    mutations: vec![
                        put(b"b", b"bad"),
                        put(if field == 2 { b"z" } else { b"c" }, b"bad"),
                    ],
                },
            )
            .unwrap(),
        );
        assert_eq!(
            apply(&mut sm, &command).outcome,
            ApplyOutcome::FenceRejected(range().region)
        );
    }
    let r = range();
    let mut sealed = r.clone();
    sealed.sealed = true;
    sealed.version += 1;
    let seal = Command::DataRange {
        expected: Some(r.digest()),
        next: sealed,
    };
    assert_eq!(
        apply(&mut sm, &seal).outcome,
        ApplyOutcome::RawTtlRejected {
            region: r.region,
            reason: TtlRejection::LegacyCommand,
        }
    );
    assert_eq!(value(&sm, b"b"), None);
    assert_eq!(clock(&sm), floor(100));

    // A range sealed before activation still fences every TTL action.
    let mut sm = setup(Arc::new(MemEngine::new()));
    apply(&mut sm, &seal);
    assert_eq!(
        apply(
            &mut sm,
            &cmd(Action::Observe {
                observed: floor(9000)
            })
        )
        .outcome,
        ApplyOutcome::FenceRejected(r.region)
    );
    assert_eq!(value(&sm, b"b"), None);
    assert!(sm.get(ColumnFamily::Default, CLOCK_KEY).unwrap().is_none());
}

#[derive(Default)]
struct FaultStore {
    inner: MemEngine,
    batches: Mutex<Vec<(WriteBatch, AppliedPosition)>>,
    failure: AtomicU8,
}
impl ApplyStore for FaultStore {
    fn get(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<Vec<u8>>> {
        if self.failure.load(Ordering::SeqCst) == 3 {
            return Err(Error::Engine("injected read".into()));
        }
        Engine::get(&self.inner, cf, key)
    }
    fn write_applied(&self, batch: WriteBatch, at: AppliedPosition) -> Result<()> {
        if self.failure.load(Ordering::SeqCst) == 1 {
            return Err(Error::Engine("before atomic write".into()));
        }
        ReplicatedEngine::write_applied(&self.inner, batch.clone(), at)?;
        self.batches.lock().unwrap().push((batch, at));
        if self.failure.load(Ordering::SeqCst) == 2 {
            return Err(Error::Engine("after atomic write".into()));
        }
        Ok(())
    }
    fn applied_position(&self) -> Result<DurableAppliedPosition> {
        Ok(self
            .batches
            .lock()
            .unwrap()
            .last()
            .map_or(DurableAppliedPosition::AppliedNothing, |(_, at)| {
                DurableAppliedPosition::AppliedThrough(*at)
            }))
    }
}

#[test]
fn ttl_atomic_write_errors_never_publish_receipts_or_partial_data() {
    for failure in [1, 2, 3] {
        for activation in [false, true] {
            let engine = Arc::new(FaultStore::default());
            let mut sm = setup(engine.clone());
            if !activation {
                apply(
                    &mut sm,
                    &cmd(Action::Enable {
                        observed: floor(100),
                    }),
                );
            }
            let before = sm.applied_index();
            let writes = engine.batches.lock().unwrap().len();
            let command = if activation {
                cmd(Action::Enable {
                    observed: floor(200),
                })
            } else {
                write(Some(1), 200, vec![put(b"b", b"value")])
            };
            engine.failure.store(failure, Ordering::SeqCst);
            let at = AppliedPosition {
                term: 3,
                index: before.0 + 1,
            };
            assert!(sm.apply_at(at, &command).is_err());
            assert_eq!(sm.applied_index(), before);
            engine.failure.store(0, Ordering::SeqCst);
            assert_eq!(
                engine.batches.lock().unwrap().len(),
                writes + usize::from(failure == 2)
            );
            // The caller poisons a failed apply. A fresh owner recovers from
            // durable state, including a write that succeeded before its error.
            drop(sm);
            let mut recovered = setup(engine.clone());
            assert_eq!(
                recovered.applied_index().0,
                before.0 + u64::from(failure == 2)
            );
            recovered.apply_at(at, &command).unwrap();
            assert_eq!(clock(&recovered), floor(200));
            if !activation {
                assert_eq!(value(&recovered, b"b"), Some(b"value".to_vec()));
                assert_eq!(
                    expiry(&recovered, b"b"),
                    plan(Some(1), 200).at_write(range().creation, at).unwrap()
                );
                let batches = engine.batches.lock().unwrap();
                let (batch, durable_at) = batches.last().unwrap();
                assert_eq!(*durable_at, at);
                assert_eq!(
                    batch.len(),
                    3,
                    "value, expiry and clock share one positioned write"
                );
            }
        }
    }
}

#[test]
fn ttl_corrupt_state_is_an_apply_error_not_a_logical_refusal() {
    for (cf, key) in [
        (ColumnFamily::Default, CLOCK_KEY.to_vec()),
        (ColumnFamily::Default, RANGE_KEY.to_vec()),
        (ColumnFamily::Write, physical(b"b")),
    ] {
        let mut sm = enabled();
        let before = sm.applied_index();
        let mut batch = WriteBatch::new();
        batch.put(cf, key.clone(), vec![0]);
        Engine::write(sm.engine.as_ref(), batch).unwrap();
        let candidate = plan(Some(0), 100)
            .at_write(range().creation, AppliedPosition { term: 3, index: 1 })
            .unwrap()
            .unwrap();
        assert!(sm
            .apply_at(
                AppliedPosition {
                    term: 3,
                    index: before.0 + 1
                },
                &cmd(Action::Cleanup {
                    candidates: vec![(b"b".to_vec(), candidate)]
                })
            )
            .is_err());
        assert_eq!(sm.applied_index(), before);
        if key == CLOCK_KEY {
            assert!(MemStateMachine::with_engine(sm.engine.clone()).is_err());
        }
    }
}

#[test]
fn ttl_cleanup_failure_after_a_valid_candidate_keeps_the_whole_batch_unapplied() {
    let mut sm = enabled();
    apply(
        &mut sm,
        &write(
            Some(0),
            100,
            vec![put(b"b", b"first"), put(b"c", b"second")],
        ),
    );
    let first = expiry(&sm, b"b").unwrap();
    let second = expiry(&sm, b"c").unwrap();
    let before = sm.applied_index();
    let mut corrupt = WriteBatch::new();
    corrupt.put(ColumnFamily::Write, physical(b"c"), vec![0]);
    Engine::write(sm.engine.as_ref(), corrupt).unwrap();
    let command = cmd(Action::Cleanup {
        candidates: vec![(b"b".to_vec(), first), (b"c".to_vec(), second)],
    });
    assert!(sm
        .apply_at(
            AppliedPosition {
                term: 3,
                index: before.0 + 1
            },
            &command
        )
        .is_err());
    assert_eq!(sm.applied_index(), before);
    assert_eq!(value(&sm, b"b"), Some(b"first".to_vec()));
    assert_eq!(expiry(&sm, b"b"), Some(first));
    assert_eq!(value(&sm, b"c"), Some(b"second".to_vec()));
}

#[test]
fn ttl_wal_restart_preserves_activation_deadline_clock_and_cleared_metadata() {
    let dir = std::env::temp_dir().join(format!(
        "kv9-ttl-apply-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("engine.wal");
    let command = write(Some(1), 100, vec![put(b"b", b"value")]);
    let first;
    {
        let mut sm = setup(Arc::new(WalEngine::open(&path).unwrap().0));
        apply(
            &mut sm,
            &cmd(Action::Enable {
                observed: floor(100),
            }),
        );
        apply(&mut sm, &command);
        first = expiry(&sm, b"b").unwrap();
        apply(
            &mut sm,
            &cmd(Action::Observe {
                observed: floor(1100),
            }),
        );
    }
    {
        let mut sm = setup(Arc::new(WalEngine::open(&path).unwrap().0));
        assert_eq!(sm.applied_index(), LogIndex(4));
        assert!(sm.ttl_enabled);
        assert_eq!(expiry(&sm, b"b"), Some(first));
        assert!(first.expired_at(clock(&sm)));
        sm.apply_at(AppliedPosition { term: 3, index: 3 }, &command)
            .unwrap();
        assert_eq!(expiry(&sm, b"b"), Some(first));
        assert!(matches!(
            apply(&mut sm, &legacy()[0]).outcome,
            ApplyOutcome::RawTtlRejected { .. }
        ));
        apply(&mut sm, &write(None, 1100, vec![put(b"b", b"permanent")]));
    }
    {
        let mut sm = setup(Arc::new(WalEngine::open(&path).unwrap().0));
        assert_eq!(sm.applied_index(), LogIndex(6));
        assert_eq!(clock(&sm), floor(1100));
        assert_eq!(expiry(&sm, b"b"), None);
        apply(
            &mut sm,
            &cmd(Action::Cleanup {
                candidates: vec![(b"b".to_vec(), first)],
            }),
        );
        assert_eq!(value(&sm, b"b"), Some(b"permanent".to_vec()));
    }
    std::fs::remove_dir_all(dir).unwrap();
}

fn driver<E: ApplyStore + 'static>(
    engine: Arc<E>,
    id: NodeId,
    voters: &[NodeId],
    hub: &Arc<InProcHub>,
) -> Arc<NodeDriver<raft::storage::MemStorage, E>> {
    let r = range();
    let mut sm = MemStateMachine::with_engine(engine).unwrap();
    sm.set_data_group(r.root, r.region, r.creation).unwrap();
    NodeDriver::new(
        Arc::new(RaftPeer::new(id, r.region, voters).unwrap()),
        Arc::new(hub.endpoint(id)) as Arc<dyn RaftTransport>,
        sm,
    )
    .unwrap()
}
fn pump<E: ApplyStore + 'static>(drivers: &[Arc<NodeDriver<raft::storage::MemStorage, E>>]) {
    for _ in 0..30 {
        for driver in drivers {
            driver.step().unwrap();
        }
    }
}

#[test]
fn ttl_real_driver_keeps_activation_out_of_legacy_apply_groups_and_types_receipts() {
    let hub = InProcHub::new();
    let engine = Arc::new(FaultStore::default());
    let driver = driver(engine.clone(), NodeId(1), &[NodeId(1)], &hub);
    driver.peer().campaign().unwrap();
    pump(std::slice::from_ref(&driver));
    driver
        .propose(&Command::DataRange {
            expected: None,
            next: range(),
        })
        .unwrap();
    pump(std::slice::from_ref(&driver));
    let before = driver.propose(&legacy()[0]).unwrap();
    let activation = driver
        .peer()
        .propose_raw_for_harness(
            cmd(Action::Enable {
                observed: floor(100),
            })
            .encode(),
        )
        .unwrap();
    let refused = driver.propose(&legacy()[0]).unwrap();
    let accepted = driver
        .propose(&write(Some(1), 100, vec![put(b"b", b"ttl")]))
        .unwrap();
    pump(std::slice::from_ref(&driver));
    for at in [before, activation, accepted] {
        assert_eq!(
            driver.wait_applied(at, Duration::ZERO).unwrap(),
            ApplyWaitOutcome::Applied(AppliedPosition {
                term: at.term,
                index: at.index.0
            })
        );
    }
    assert_eq!(
        driver.wait_applied(refused, Duration::ZERO).unwrap(),
        ApplyWaitOutcome::RawTtlRejected {
            at: AppliedPosition {
                term: refused.term,
                index: refused.index.0
            },
            region: range().region,
            reason: TtlRejection::LegacyCommand,
        }
    );
    assert_eq!(
        engine.get(ColumnFamily::Default, &physical(b"b")).unwrap(),
        Some(b"ttl".to_vec())
    );
    let actual = engine
        .get(ColumnFamily::Write, &physical(b"b"))
        .unwrap()
        .unwrap();
    assert_eq!(
        ExpiryRecord::decode(&actual).unwrap(),
        plan(Some(1), 100)
            .at_write(
                range().creation,
                AppliedPosition {
                    term: accepted.term,
                    index: accepted.index.0
                }
            )
            .unwrap()
            .unwrap()
    );
}

#[test]
fn ttl_real_driver_poison_suppresses_receipts_before_and_after_durability() {
    for failure in [1, 2] {
        let hub = InProcHub::new();
        let engine = Arc::new(FaultStore::default());
        let driver = driver(engine.clone(), NodeId(1), &[NodeId(1)], &hub);
        driver.peer().campaign().unwrap();
        pump(std::slice::from_ref(&driver));
        driver
            .propose(&Command::DataRange {
                expected: None,
                next: range(),
            })
            .unwrap();
        driver
            .peer()
            .propose_raw_for_harness(
                cmd(Action::Enable {
                    observed: floor(100),
                })
                .encode(),
            )
            .unwrap();
        pump(std::slice::from_ref(&driver));
        let before = driver.driver_applied();
        let at = driver
            .propose(&write(Some(1), 200, vec![put(b"b", b"value")]))
            .unwrap();
        engine.failure.store(failure, Ordering::SeqCst);
        for _ in 0..10 {
            if driver.step().is_err() {
                break;
            }
        }
        assert!(driver.status().fatal.is_some());
        assert_eq!(driver.driver_applied(), before);
        assert!(driver.wait_applied(at, Duration::ZERO).is_err());
        engine.failure.store(0, Ordering::SeqCst);
        drop(driver);
        let recovered = setup(engine);
        if failure == 2 {
            assert_eq!(recovered.applied_index(), at.index);
            assert_eq!(value(&recovered, b"b"), Some(b"value".to_vec()));
            assert!(expiry(&recovered, b"b").is_some());
            assert_eq!(clock(&recovered), floor(200));
        } else {
            assert_eq!(value(&recovered, b"b"), None);
            assert_eq!(expiry(&recovered, b"b"), None);
            assert_eq!(clock(&recovered), floor(100));
        }
    }
}

#[test]
fn ttl_three_voters_delayed_apply_and_leader_change_keep_the_same_expiry_decision() {
    let hub = InProcHub::new();
    let ids = [NodeId(1), NodeId(2), NodeId(3)];
    let engines: Vec<_> = ids.iter().map(|_| Arc::new(MemEngine::new())).collect();
    let drivers: Vec<_> = ids
        .iter()
        .zip(&engines)
        .map(|(&id, engine)| driver(engine.clone(), id, &ids, &hub))
        .collect();
    drivers[0].peer().campaign().unwrap();
    pump(&drivers);
    assert_eq!(drivers[0].status().role, Role::Leader);
    drivers[0]
        .propose(&Command::DataRange {
            expected: None,
            next: range(),
        })
        .unwrap();
    drivers[0]
        .peer()
        .propose_raw_for_harness(
            cmd(Action::Enable {
                observed: floor(100),
            })
            .encode(),
        )
        .unwrap();
    pump(&drivers);
    drivers[2].pause_apply(true);
    let first = drivers[0]
        .propose(&write(Some(1), 100, vec![put(b"b", b"old")]))
        .unwrap();
    drivers[0]
        .propose(&cmd(Action::Observe {
            observed: floor(1100),
        }))
        .unwrap();
    pump(&drivers);
    let original = plan(Some(1), 100)
        .at_write(
            range().creation,
            AppliedPosition {
                term: first.term,
                index: first.index.0,
            },
        )
        .unwrap()
        .unwrap();
    assert!(
        Engine::get(engines[2].as_ref(), ColumnFamily::Write, &physical(b"b"))
            .unwrap()
            .is_none()
    );
    drivers[2].pause_apply(false);
    pump(&drivers);
    // The lagging voter must consume the encoded deadline and clock entries;
    // applying later cannot give it a later deadline or a different visibility.
    for engine in &engines {
        assert_eq!(
            ExpiryRecord::decode(
                &Engine::get(engine.as_ref(), ColumnFamily::Write, &physical(b"b"))
                    .unwrap()
                    .unwrap()
            )
            .unwrap(),
            original
        );
        let clock = ClockWatermark::decode(
            &Engine::get(engine.as_ref(), ColumnFamily::Default, CLOCK_KEY)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert!(original.expired_at(clock));
    }
    // Drop the old leader's traffic until check_quorum deposes it, then elect
    // among the caught-up followers. This uses real raft-rs voting/log apply.
    for _ in 0..40 {
        drivers[0].tick_and_step().unwrap();
        hub.endpoint(ids[1]).drain();
        hub.endpoint(ids[2]).drain();
    }
    assert_ne!(drivers[0].status().role, Role::Leader);
    for _ in 0..2000 {
        drivers[0].step().unwrap();
        drivers[1].tick_and_step().unwrap();
        drivers[2].tick_and_step().unwrap();
        if drivers[1..].iter().any(|d| d.status().role == Role::Leader) {
            break;
        }
    }
    let leader = drivers[1..]
        .iter()
        .find(|d| d.status().role == Role::Leader)
        .unwrap();
    assert!(leader.status().term > first.term);
    // Renew before the old cleanup reaches the new leader. A rollback sample
    // already rejected at proposal preparation cannot be invented in apply.
    let renewal = leader
        .propose(&write(Some(1), 1100, vec![put(b"b", b"renewed")]))
        .unwrap();
    leader
        .propose(&cmd(Action::Cleanup {
            candidates: vec![(b"b".to_vec(), original)],
        }))
        .unwrap();
    pump(&drivers);
    for engine in &engines {
        assert_eq!(
            Engine::get(engine.as_ref(), ColumnFamily::Default, &physical(b"b")).unwrap(),
            Some(b"renewed".to_vec())
        );
    }
    let renewed = plan(Some(1), 1100)
        .at_write(
            range().creation,
            AppliedPosition {
                term: renewal.term,
                index: renewal.index.0,
            },
        )
        .unwrap()
        .unwrap();
    leader
        .propose(&cmd(Action::Observe {
            observed: floor(2100),
        }))
        .unwrap();
    let cleanup = leader
        .propose(&cmd(Action::Cleanup {
            candidates: vec![(b"b".to_vec(), renewed)],
        }))
        .unwrap();
    pump(&drivers);
    for (driver, engine) in drivers.iter().zip(&engines) {
        assert!(matches!(
            driver.wait_applied(cleanup, Duration::ZERO).unwrap(),
            ApplyWaitOutcome::Applied(_)
        ));
        assert_eq!(
            Engine::get(engine.as_ref(), ColumnFamily::Default, &physical(b"b")).unwrap(),
            None
        );
        assert_eq!(
            Engine::get(engine.as_ref(), ColumnFamily::Write, &physical(b"b")).unwrap(),
            None
        );
    }
}
