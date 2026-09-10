//! Bounded Raw apply groups. Catalog/manifest/conf entries are barriers.

use super::*;
use crate::{FencedInner, KvOp};
use kv9_common::codec::{decode_key, KeyMode};
use kv9_common::{Error, KeyspaceId};

pub(crate) const MAX_RAW_GROUP_ENTRIES: usize = 128;
pub(crate) const MAX_RAW_GROUP_BYTES: usize = 1024 * 1024;

fn raw_key(cf: u8, key: &[u8]) -> bool {
    cf == crate::cf_code(ColumnFamily::Default)
        && decode_key(key)
            .is_ok_and(|key| key.mode == KeyMode::Raw && key.keyspace != KeyspaceId::SYSTEM)
}

fn raw_ops(ops: &[KvOp]) -> bool {
    ops.iter().all(|op| match op {
        KvOp::Put { cf, key, .. } | KvOp::Delete { cf, key } => raw_key(*cf, key),
    })
}

impl<E: ApplyStore> MemStateMachine<E> {
    pub(crate) fn can_group_raw(&self, cmd: &Command) -> bool {
        match cmd {
            Command::Put { cf, key, .. } => raw_key(*cf, key),
            Command::Write { ops } => raw_ops(ops),
            Command::Fenced {
                inner: FencedInner::Write { ops },
                ..
            } => {
                raw_ops(ops)
                    && self
                        .adjudicator
                        .as_ref()
                        .is_some_and(|a| a.independent_of_raw_writes())
            }
            _ => false,
        }
    }

    /// Compose an already-committed, bounded Raw prefix into one durable batch.
    /// The driver bounds encoded input bytes and never crosses a non-Raw entry.
    /// No receipts or applied watermark are published before write_applied succeeds.
    pub(crate) fn apply_raw_group(
        &mut self,
        commands: &[(AppliedPosition, Command)],
    ) -> Result<Vec<ApplyResult>> {
        if commands.is_empty() || commands.len() > MAX_RAW_GROUP_ENTRIES {
            return Err(Error::Raft("invalid Raw apply group length".into()));
        }
        let mut previous_index = self.applied.0;
        let mut previous_term = match self.engine.applied_position()? {
            DurableAppliedPosition::AppliedThrough(at) => at.term,
            _ => 0,
        };
        let mut batch = WriteBatch::new();
        let mut results = Vec::with_capacity(commands.len());
        for (at, cmd) in commands {
            // Validate every position; checking just the tail would hide an
            // invalid intermediate position inside an otherwise legal WAL record.
            if at.index <= previous_index || at.term == 0 || at.term < previous_term {
                return Err(Error::Raft(
                    "Raw apply group positions are not ordered".into(),
                ));
            }
            if !self.can_group_raw(cmd) {
                return Err(Error::Raft(
                    "non-independent command in Raw apply group".into(),
                ));
            }
            let index = LogIndex(at.index);
            let (next, result) = match cmd {
                Command::Fenced { fence, inner } => {
                    let adjudicator = self.adjudicator.as_ref().ok_or_else(|| {
                        Error::Raft("Raw apply group requires fence adjudicator".into())
                    })?;
                    if adjudicator.is_fresh(fence)? {
                        (inner.to_write_batch(), ApplyResult::write_ok(index))
                    } else {
                        (
                            WriteBatch::new(),
                            ApplyResult::fence_rejected(
                                index,
                                kv9_common::RegionId(fence.region_id),
                            ),
                        )
                    }
                }
                _ => (cmd.to_write_batch()?, ApplyResult::write_ok(index)),
            };
            batch.append(next);
            results.push(result);
            previous_index = at.index;
            previous_term = at.term;
        }
        let at = commands.last().expect("nonempty group checked above").0;
        self.engine.write_applied(batch, at)?;
        self.applied = LogIndex(at.index);
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::{ApplyWaitOutcome, NodeDriver};
    use crate::transport::{InProcHub, RaftTransport};
    use crate::{RaftGroup, RaftPeer, RegionFence, Role};
    use kv9_common::codec::encode_key;
    use kv9_common::{NodeId, RegionId};
    use kv9_engine::Engine;
    use std::sync::atomic::{AtomicU8, Ordering};
    use std::sync::Mutex;
    use std::time::Duration;

    const EPOCH: &[u8] = b"s\0\0\0epoch";

    #[derive(Default)]
    struct ProbeStore {
        data: MemEngine,
        writes: Mutex<Vec<(WriteBatch, AppliedPosition)>>,
        failure: AtomicU8,
    }

    impl ApplyStore for ProbeStore {
        fn get(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<Vec<u8>>> {
            Engine::get(&self.data, cf, key)
        }

        fn write_applied(&self, batch: WriteBatch, at: AppliedPosition) -> Result<()> {
            if self.failure.load(Ordering::SeqCst) == 1 {
                return Err(Error::Engine("injected before group durability".into()));
            }
            ReplicatedEngine::write_applied(&self.data, batch.clone(), at)?;
            self.writes.lock().unwrap().push((batch, at));
            if self.failure.load(Ordering::SeqCst) == 2 {
                return Err(Error::Engine("injected after group durability".into()));
            }
            Ok(())
        }

        fn applied_position(&self) -> Result<DurableAppliedPosition> {
            Ok(self
                .writes
                .lock()
                .unwrap()
                .last()
                .map_or(DurableAppliedPosition::AppliedNothing, |(_, at)| {
                    DurableAppliedPosition::AppliedThrough(*at)
                }))
        }
    }

    struct EpochFence(Arc<ProbeStore>);

    impl FenceAdjudicator for EpochFence {
        fn is_fresh(&self, fence: &RegionFence) -> Result<bool> {
            let epoch = self.0.get(ColumnFamily::Default, EPOCH)?.unwrap_or(vec![0]);
            if fence.region_id == 99 {
                return Err(Error::Engine("injected epoch read failure".into()));
            }
            Ok(fence.version >= u64::from(epoch[0]))
        }

        fn independent_of_raw_writes(&self) -> bool {
            true
        }
    }

    fn key() -> Vec<u8> {
        encode_key(KeyMode::Raw, KeyspaceId(1), b"key").unwrap()
    }

    fn put(value: &[u8]) -> Command {
        Command::Put {
            cf: 0,
            key: key(),
            value: value.to_vec(),
        }
    }

    fn at(index: u64) -> AppliedPosition {
        AppliedPosition { term: 2, index }
    }

    fn fenced(version: u64, value: &[u8]) -> Command {
        Command::Fenced {
            fence: RegionFence {
                region_id: 1,
                conf_ver: 1,
                version,
            },
            inner: FencedInner::Write {
                ops: vec![KvOp::Put {
                    cf: 0,
                    key: key(),
                    value: value.to_vec(),
                }],
            },
        }
    }

    fn machine(store: &Arc<ProbeStore>) -> MemStateMachine<ProbeStore> {
        let mut sm = MemStateMachine::with_engine(store.clone()).unwrap();
        sm.set_fence_adjudicator(Arc::new(EpochFence(store.clone())));
        sm
    }

    #[test]
    fn ordered_group_preserves_overwrites_deletes_and_exact_tail() {
        let store = Arc::new(ProbeStore::default());
        let mut sm = machine(&store);
        let commands = vec![
            (at(1), put(b"old")),
            (
                at(2),
                Command::Write {
                    ops: vec![KvOp::Delete { cf: 0, key: key() }],
                },
            ),
            (at(3), fenced(0, b"new")),
        ];
        let receipts = sm.apply_raw_group(&commands).unwrap();
        assert_eq!(
            receipts
                .iter()
                .map(|r| r.applied_index.0)
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(store.writes.lock().unwrap().len(), 1);
        assert_eq!(
            store.get(ColumnFamily::Default, &key()).unwrap(),
            Some(b"new".to_vec())
        );
        assert_eq!(
            store.applied_position().unwrap(),
            DurableAppliedPosition::AppliedThrough(at(3))
        );
        assert_eq!(machine(&store).applied_index(), LogIndex(3));
    }

    #[test]
    fn stale_verdict_is_retained_and_later_read_error_publishes_nothing() {
        let store = Arc::new(ProbeStore::default());
        let mut sm = machine(&store);
        sm.apply_at(
            at(1),
            &Command::Put {
                cf: 0,
                key: EPOCH.to_vec(),
                value: vec![2],
            },
        )
        .unwrap();
        let receipts = sm
            .apply_raw_group(&[(at(2), fenced(2, b"fresh")), (at(3), fenced(1, b"stale"))])
            .unwrap();
        assert_eq!(
            receipts[1].outcome,
            ApplyOutcome::FenceRejected(RegionId(1))
        );
        assert_eq!(
            store.get(ColumnFamily::Default, &key()).unwrap(),
            Some(b"fresh".to_vec())
        );
        let mut broken = fenced(2, b"broken");
        if let Command::Fenced { fence, .. } = &mut broken {
            fence.region_id = 99;
        }
        assert!(sm
            .apply_raw_group(&[(at(4), put(b"unpublished")), (at(5), broken)])
            .is_err());
        assert_eq!(store.writes.lock().unwrap().len(), 2);
        assert_eq!(sm.applied_index(), LogIndex(3));
        assert_eq!(
            store.get(ColumnFamily::Default, &key()).unwrap(),
            Some(b"fresh".to_vec())
        );
    }

    #[test]
    fn failed_group_returns_no_receipts_even_if_durable_effect_occurred() {
        for failure in [1, 2] {
            let store = Arc::new(ProbeStore::default());
            let mut sm = machine(&store);
            store.failure.store(failure, Ordering::SeqCst);
            assert!(sm
                .apply_raw_group(&[(at(1), put(b"a")), (at(2), put(b"b"))])
                .is_err());
            assert_eq!(
                sm.applied_index(),
                LogIndex(0),
                "failed group advanced live apply watermark"
            );
            let recovered = machine(&store);
            assert_eq!(
                recovered.applied_index(),
                LogIndex(if failure == 2 { 2 } else { 0 })
            );
            assert_eq!(
                store.get(ColumnFamily::Default, &key()).unwrap(),
                (failure == 2).then(|| b"b".to_vec())
            );
        }
    }

    #[test]
    fn grouping_refuses_hidden_metadata_and_non_opted_in_adjudicators() {
        struct Arbitrary;
        impl FenceAdjudicator for Arbitrary {
            fn is_fresh(&self, _: &RegionFence) -> Result<bool> {
                Ok(true)
            }
        }
        let store = Arc::new(ProbeStore::default());
        let mut sm = machine(&store);
        assert!(sm.can_group_raw(&fenced(1, b"raw")));
        for (cf, key) in [
            (0, EPOCH.to_vec()),
            (1, key()),
            (0, b"r\0\0\0reserved".to_vec()),
            (0, b"r".to_vec()),
            (0, b"\0kv9\0manifest_pair\0".to_vec()),
        ] {
            let cmd = Command::Fenced {
                fence: RegionFence {
                    region_id: 1,
                    conf_ver: 1,
                    version: 1,
                },
                inner: FencedInner::Write {
                    ops: vec![KvOp::Put {
                        cf,
                        key,
                        value: vec![3],
                    }],
                },
            };
            assert!(!sm.can_group_raw(&cmd));
            assert!(sm
                .apply_raw_group(&[(at(1), put(b"raw")), (at(2), cmd)])
                .is_err());
        }
        assert_eq!(store.writes.lock().unwrap().len(), 0);
        sm.set_fence_adjudicator(Arc::new(Arbitrary));
        assert!(!sm.can_group_raw(&fenced(1, b"raw")));
        assert!(sm.apply_raw_group(&[(at(1), fenced(1, b"raw"))]).is_err());
    }

    #[test]
    fn invalid_intermediate_position_cannot_hide_behind_valid_tail() {
        for positions in [
            [at(2), at(2), at(3)],
            [at(2), at(1), at(3)],
            [at(1), AppliedPosition { term: 1, index: 2 }, at(3)],
        ] {
            let store = Arc::new(ProbeStore::default());
            let mut sm = machine(&store);
            let commands: Vec<_> = positions.into_iter().map(|at| (at, put(b"v"))).collect();
            assert!(sm.apply_raw_group(&commands).is_err());
            assert_eq!(store.writes.lock().unwrap().len(), 0);
            assert_eq!(sm.applied_index(), LogIndex(0));
        }
    }

    fn driver(store: &Arc<ProbeStore>) -> Arc<NodeDriver<raft::storage::MemStorage, ProbeStore>> {
        let hub = InProcHub::new();
        let peer = Arc::new(RaftPeer::new(NodeId(1), RegionId(1), &[NodeId(1)]).unwrap());
        let d = NodeDriver::new(
            peer,
            Arc::new(hub.endpoint(NodeId(1))) as Arc<dyn RaftTransport>,
            machine(store),
        )
        .unwrap();
        d.peer().campaign().unwrap();
        for _ in 0..10 {
            d.step().unwrap();
        }
        assert_eq!(d.status().role, Role::Leader);
        d
    }

    #[test]
    fn real_driver_splits_at_catalog_epoch_change_and_preserves_receipt_verdict() {
        let store = Arc::new(ProbeStore::default());
        let d = driver(&store);
        let commands = [
            fenced(0, b"a"),
            fenced(0, b"b"),
            Command::CatalogTxn {
                ops: vec![KvOp::Put {
                    cf: 0,
                    key: EPOCH.to_vec(),
                    value: vec![1],
                }],
            },
            fenced(0, b"stale"),
            fenced(1, b"fresh"),
        ];
        let proposals: Vec<_> = commands.iter().map(|c| d.propose(c).unwrap()).collect();
        for _ in 0..10 {
            d.step().unwrap();
        }
        let writes = store.writes.lock().unwrap();
        assert_eq!(
            writes.iter().map(|(b, _)| b.len()).collect::<Vec<_>>(),
            [2, 1, 1],
            "catalog change was not an apply-group barrier"
        );
        drop(writes);
        assert!(matches!(
            d.wait_applied(proposals[3], Duration::ZERO).unwrap(),
            ApplyWaitOutcome::FenceRejected { .. }
        ));
        assert_eq!(
            store.get(ColumnFamily::Default, &key()).unwrap(),
            Some(b"fresh".to_vec())
        );
        assert_eq!(d.driver_applied().unwrap().index, proposals[4].index.0);
    }

    #[test]
    fn real_driver_bounds_entry_count_and_encoded_bytes_without_waiting() {
        for (count, bytes) in [(129, 1), (3, 600 * 1024), (1, 2 * 1024 * 1024)] {
            let store = Arc::new(ProbeStore::default());
            let d = driver(&store);
            let commands: Vec<_> = (0..count)
                .map(|_| d.propose(&put(&vec![7; bytes])).unwrap())
                .collect();
            for _ in 0..20 {
                d.step().unwrap();
            }
            let writes = store.writes.lock().unwrap();
            let lengths: Vec<_> = writes.iter().map(|(b, _)| b.len()).collect();
            assert_eq!(
                lengths,
                if count == 129 {
                    vec![128, 1]
                } else {
                    vec![1; count]
                }
            );
            assert_eq!(
                d.driver_applied().unwrap().index,
                commands.last().unwrap().index.0
            );
        }
    }

    #[test]
    fn real_driver_group_failure_keeps_receipts_and_driver_watermark_unpublished() {
        let store = Arc::new(ProbeStore::default());
        let d = driver(&store);
        let before = d.driver_applied();
        let a = d.propose(&put(b"a")).unwrap();
        let b = d.propose(&put(b"b")).unwrap();
        store.failure.store(2, Ordering::SeqCst);
        for _ in 0..10 {
            if d.step().is_err() {
                break;
            }
        }
        assert!(d.status().fatal.is_some());
        assert_eq!(d.driver_applied(), before);
        assert_eq!(d.status().applied_index, 0);
        for proposal in [a, b] {
            assert!(d.wait_applied(proposal, Duration::ZERO).is_err());
        }
        assert_eq!(store.writes.lock().unwrap().len(), 1);
        assert_eq!(machine(&store).applied_index(), b.index);
    }

    #[test]
    fn real_legacy_and_segmented_wals_sync_once_and_recover_the_composed_group() {
        use kv9_common::metrics::Outcome;
        use kv9_engine::WalEngine;
        for segmented in [false, true] {
            let dir = std::env::temp_dir().join(format!(
                "kv9-raw-group-{}-{:?}-{segmented}",
                std::process::id(),
                std::thread::current().id()
            ));
            std::fs::create_dir(&dir).unwrap();
            let path = dir.join("engine.wal");
            let engine = Arc::new(WalEngine::open(&path).unwrap().0);
            if segmented {
                engine.enable_segmentation().unwrap();
            }
            let syncs =
                || engine.io_metrics().sync.snapshot().outcomes[Outcome::Success as usize].count;
            let before = syncs();
            let commands = [
                (at(1), put(b"old")),
                (
                    at(2),
                    Command::Write {
                        ops: vec![KvOp::Delete { cf: 0, key: key() }],
                    },
                ),
                (at(3), put(b"new")),
            ];
            let mut sm = MemStateMachine::with_engine(engine.clone()).unwrap();
            assert_eq!(sm.apply_raw_group(&commands).unwrap().len(), 3);
            assert_eq!(
                syncs() - before,
                1,
                "Raw group performed more than one WAL sync"
            );
            drop(sm);
            drop(engine);

            let (engine, report) = WalEngine::open(&path).unwrap();
            assert_eq!(report.replayed_records, 1);
            let engine = Arc::new(engine);
            assert_eq!(
                ApplyStore::get(&*engine, ColumnFamily::Default, &key()).unwrap(),
                Some(b"new".to_vec())
            );
            assert_eq!(
                ApplyStore::applied_position(&*engine).unwrap(),
                DurableAppliedPosition::AppliedThrough(at(3))
            );
            let mut sm = MemStateMachine::with_engine(engine.clone()).unwrap();
            let before =
                engine.io_metrics().sync.snapshot().outcomes[Outcome::Success as usize].count;
            for (at, cmd) in &commands {
                sm.apply_at(*at, cmd).unwrap();
            }
            assert_eq!(
                engine.io_metrics().sync.snapshot().outcomes[Outcome::Success as usize].count,
                before,
                "replay re-applied an already durable group member"
            );
            drop(sm);
            drop(engine);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
}
