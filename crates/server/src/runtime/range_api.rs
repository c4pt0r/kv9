//! Private public-Raw dispatch handles. Ownership remains with RegionManager;
//! shutdown clears this directory and stops drivers before dropping store locks.
use super::*;
use crate::api::{
    RawReadJob, RawReadPreparation, RawWrite, RawWriteCompletion, RawWritePreparation,
};
use kv9_common::data_range::{DataRange, RANGE_KEY};
use kv9_engine::ColumnFamily;

#[derive(Default)]
pub(crate) struct RawDirectory {
    groups: Mutex<BTreeMap<RegionId, Arc<RawGroup>>>,
    clock: Option<Arc<dyn crate::ttl_clock::TtlClock>>,
}
impl RawDirectory {
    pub(crate) fn with_clock(clock: Option<Arc<dyn crate::ttl_clock::TtlClock>>) -> Self {
        Self {
            groups: Mutex::default(),
            clock,
        }
    }
    pub(crate) fn clock(&self) -> Option<Arc<dyn crate::ttl_clock::TtlClock>> {
        self.clock.clone()
    }
    /// Key-aware resolution for a possibly split keyspace: the ONE unsealed
    /// group whose range contains `key`. Sealed parents never resolve; the
    /// per-request `authorize` still re-checks containment and the epoch.
    pub(crate) fn get_for(&self, keyspace: KeyspaceId, key: &[u8]) -> Option<Arc<RawGroup>> {
        let groups = self.groups.lock().expect("raw directory poisoned");
        let mut matches = groups.values().filter(|g| {
            g.binding.keyspace == keyspace && !g.binding.sealed && g.binding.contains(key)
        });
        let only = matches.next()?.clone();
        matches.next().is_none().then_some(only)
    }
    pub(crate) fn scoped(&self, scope: &DataRange) -> Result<Arc<dyn RawApi>> {
        let groups = self.groups.lock().expect("raw directory poisoned");
        let group = groups
            .get(&scope.region)
            .filter(|g| g.binding == *scope)
            .ok_or(Error::StaleEpoch {
                region: scope.region,
            })?;
        Ok(group.clone())
    }
    pub(crate) fn remove(&self, region: kv9_common::RegionId) {
        self.groups
            .lock()
            .expect("raw directory poisoned")
            .remove(&region);
    }
    pub(crate) fn insert(&self, group: Arc<RawGroup>) {
        // REPLACE: reconciliation refreshes the binding when the committed
        // directory advances (a parent sealing, a version bump). Serving
        // authorization re-reads the engine row per request either way; a
        // stale cached binding must never keep resolving a sealed range.
        self.groups
            .lock()
            .expect("raw directory poisoned")
            .insert(group.binding.region, group);
    }
    pub(crate) fn clear(&self) {
        self.groups.lock().expect("raw directory poisoned").clear();
    }
    pub(crate) fn by_region(&self, region: RegionId) -> Option<Arc<RawGroup>> {
        self.groups
            .lock()
            .expect("raw directory poisoned")
            .get(&region)
            .cloned()
    }
    /// One round-robin candidate; never copy the complete directory or retain
    /// its lock while a maintenance job reads storage or waits on consensus.
    pub(crate) fn next_after(&self, after: Option<RegionId>) -> Option<Arc<RawGroup>> {
        use std::ops::Bound::{Excluded, Unbounded};
        let groups = self.groups.lock().expect("raw directory poisoned");
        after
            .and_then(|id| groups.range((Excluded(id), Unbounded)).next())
            .or_else(|| groups.first_key_value())
            .map(|(_, group)| group.clone())
    }
}

pub(crate) struct RawGroup {
    clock: Option<Arc<dyn crate::ttl_clock::TtlClock>>,
    binding: DataRange,
    engine: Arc<WalEngine>,
    driver: Arc<NodeDriver<DiskRaftStorage, WalEngine>>,
}
impl RawGroup {
    pub(crate) fn same_owner(&self, other: &Self) -> bool {
        self.binding == other.binding
            && Arc::ptr_eq(&self.driver, &other.driver)
            && Arc::ptr_eq(&self.engine, &other.engine)
    }
    pub(crate) fn range_binding(&self) -> &DataRange {
        &self.binding
    }
}

struct RangePermit(kv9_raft::RegionFence);

impl RawGroup {
    pub(crate) fn new(
        binding: DataRange,
        engine: Arc<WalEngine>,
        driver: Arc<NodeDriver<DiskRaftStorage, WalEngine>>,
        clock: Option<Arc<dyn crate::ttl_clock::TtlClock>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            clock,
            binding,
            engine,
            driver,
        })
    }
    /// Read-only handles for driver-owned source capture. This grants no
    /// write, serving or installation capability beyond what each part
    /// already enforces itself.
    pub(crate) fn capture_parts(
        &self,
    ) -> (
        &Arc<NodeDriver<DiskRaftStorage, WalEngine>>,
        &Arc<WalEngine>,
        &DataRange,
    ) {
        (&self.driver, &self.engine, &self.binding)
    }
    pub(crate) fn leader(&self) -> Option<NodeId> {
        self.driver.status().leader_id
    }
    fn authorize(
        &self,
        view: &dyn ReadView,
        ctx: &RequestContext,
        span: KeySpan<'_>,
    ) -> Result<RangePermit> {
        // Legacy reads never inherit TTL capabilities: the distinct TTL path
        // commits time before reading. Check this reader's actual snapshot.
        if let Some(bytes) = view.get(ColumnFamily::Default, kv9_common::raw_ttl::CLOCK_KEY)? {
            kv9_common::raw_ttl::ClockWatermark::decode(&bytes)?;
            return Err(Error::RawTtlRejected {
                region: self.binding.region,
                reason: kv9_common::raw_ttl::TtlRejection::LegacyCommand,
            });
        }
        self.authorize_range(view, ctx, span)
    }
    fn authorize_range(
        &self,
        view: &dyn ReadView,
        ctx: &RequestContext,
        span: KeySpan<'_>,
    ) -> Result<RangePermit> {
        let range = view
            .get(ColumnFamily::Default, RANGE_KEY)?
            .map(|b| DataRange::decode(&b))
            .transpose()?
            .ok_or_else(|| {
                Error::MetaNotReady("data range has not applied its initial ownership".into())
            })?;
        if (
            range.root,
            range.creation,
            range.region,
            range.keyspace,
            range.tenant,
        ) != (
            self.binding.root,
            self.binding.creation,
            self.binding.region,
            self.binding.keyspace,
            self.binding.tenant,
        ) {
            return Err(Error::Raft("data range immutable binding differs".into()));
        }
        if ctx.keyspace != range.keyspace {
            return Err(Error::KeyspaceNotFound(ctx.keyspace));
        }
        if range.sealed
            || ctx.region_epoch.conf_ver != range.conf_ver
            || ctx.region_epoch.version != range.version
        {
            return Err(Error::StaleEpoch {
                region: range.region,
            });
        }
        let contained = match span {
            KeySpan::Point(key) => range.contains(key),
            KeySpan::BatchKeys(keys) => keys.iter().all(|k| range.contains(k)),
            KeySpan::BatchPairs(pairs) => pairs.iter().all(|(k, _)| range.contains(k)),
            KeySpan::BatchMutations(mutations) => mutations.iter().all(|m| range.contains(m.key())),
            KeySpan::Range { start, end } => {
                range.contains(start) && range_end_within_region(end, &range.end)
            }
        };
        if !contained {
            return Err(Error::RangeCrossesRegion);
        }
        Ok(RangePermit(kv9_raft::RegionFence {
            region_id: range.region.0,
            conf_ver: range.conf_ver,
            version: range.version,
        }))
    }
    fn permit(&self, ctx: &RequestContext, span: KeySpan<'_>) -> Result<RangePermit> {
        let permit = self.authorize(self.engine.snapshot()?.as_ref(), ctx, span)?;
        self.write_pressure()?;
        Ok(permit)
    }
    pub(super) fn write_pressure(&self) -> Result<()> {
        // Early refusal avoids preparation work but reserves no capacity.
        // The peer repeats it atomically with the actual typed data append.
        self.driver.peer().check_write_pressure()
    }
    fn view(&self, ctx: &RequestContext, span: KeySpan<'_>) -> Result<Box<dyn ReadView + '_>> {
        let barrier = self.driver.read_barrier(READ_BARRIER_DEADLINE)?;
        let _ = barrier;
        let view = self.engine.snapshot()?;
        self.authorize(view.as_ref(), ctx, span)?;
        Ok(view)
    }
    fn commit(
        &self,
        permit: RangePermit,
        batch: kv9_engine::WriteBatch,
    ) -> Result<AppliedPosition> {
        if batch.is_empty() {
            return Ok(AppliedPosition { term: 0, index: 0 });
        }
        propose_and_wait(
            &self.driver,
            &Command::fenced_write_from_batch(permit.0, &batch),
            RAW_APPLY_DEADLINE,
        )
    }
    fn prepared<T: Send + 'static>(
        self: Arc<Self>,
        ctx: RequestContext,
        keys: Vec<UserKey>,
        read: impl FnOnce(&dyn ReadView, &RequestContext, &[UserKey]) -> Result<T> + Send + 'static,
    ) -> RawReadPreparation<T> {
        Box::pin(async move {
            let barrier = self
                .driver
                .read_barrier_async(READ_BARRIER_DEADLINE)
                .await?;
            Ok(RawReadJob::Blocking(Box::new(move || {
                let _ = barrier;
                let view = self.engine.snapshot()?;
                self.authorize(view.as_ref(), &ctx, KeySpan::BatchKeys(&keys))?;
                read(view.as_ref(), &ctx, &keys)
            })))
        })
    }
}

impl RawApi for RawGroup {
    fn prepare_raw_ttl(
        self: Arc<Self>,
        ctx: RequestContext,
        operation: crate::raw_ttl::Operation,
        deadline: Instant,
    ) -> crate::raw_ttl::Preparation {
        self.prepare_ttl(ctx, operation, deadline)
    }

    fn prepare_raw_get(
        self: Arc<Self>,
        ctx: RequestContext,
        key: UserKey,
    ) -> RawReadPreparation<Option<Value>> {
        self.prepared(ctx, vec![key], |view, ctx, keys| {
            RawExecutor.get(&LeaderRead::new(view, true, None)?, ctx.keyspace, &keys[0])
        })
    }
    fn prepare_raw_batch_get(
        self: Arc<Self>,
        ctx: RequestContext,
        keys: Vec<UserKey>,
    ) -> RawReadPreparation<Vec<Option<Value>>> {
        self.prepared(ctx, keys, |view, ctx, keys| {
            RawExecutor.batch_get(&LeaderRead::new(view, true, None)?, ctx.keyspace, keys)
        })
    }
    fn prepare_raw_write(
        self: Arc<Self>,
        ctx: RequestContext,
        operation: RawWrite,
    ) -> RawWritePreparation {
        Box::new(move || {
            let (permit, batch) = match operation {
                RawWrite::Put { key, value } => (
                    self.permit(&ctx, KeySpan::Point(&key))?,
                    RawExecutor.plan_put(ctx.keyspace, &key, value, RawWriteOptions::default())?,
                ),
                RawWrite::Delete { key } => (
                    self.permit(&ctx, KeySpan::Point(&key))?,
                    RawExecutor.plan_delete(ctx.keyspace, &key)?,
                ),
                RawWrite::BatchPut(pairs) => (
                    self.permit(&ctx, KeySpan::BatchPairs(&pairs))?,
                    RawExecutor.plan_batch_put(ctx.keyspace, &pairs, RawWriteOptions::default())?,
                ),
                RawWrite::Batch(mutations) => (
                    self.permit(&ctx, KeySpan::BatchMutations(&mutations))?,
                    RawExecutor.plan_batch(ctx.keyspace, &mutations, RawWriteOptions::default())?,
                ),
            };
            if batch.is_empty() {
                return Ok(
                    Box::pin(async { Ok(AppliedPosition { term: 0, index: 0 }) })
                        as RawWriteCompletion,
                );
            }
            let command = Arc::new(Command::fenced_write_from_batch(permit.0, &batch));
            let deadline = Instant::now() + RAW_APPLY_DEADLINE;
            let first = self.driver.propose_with_async_wait(&command, deadline)?;
            Ok(Box::pin(async move {
                finish_async_proposal(self.driver.clone(), command, first, deadline)
                    .await
                    .0
            }) as RawWriteCompletion)
        })
    }
    fn raw_get(&self, ctx: &RequestContext, key: &[u8]) -> Result<Option<Value>> {
        let view = self.view(ctx, KeySpan::Point(key))?;
        RawExecutor.get(
            &LeaderRead::new(view.as_ref(), true, None)?,
            ctx.keyspace,
            key,
        )
    }
    fn raw_batch_get(&self, ctx: &RequestContext, keys: &[UserKey]) -> Result<Vec<Option<Value>>> {
        let view = self.view(ctx, KeySpan::BatchKeys(keys))?;
        RawExecutor.batch_get(
            &LeaderRead::new(view.as_ref(), true, None)?,
            ctx.keyspace,
            keys,
        )
    }
    fn raw_put(&self, ctx: &RequestContext, key: UserKey, value: Value) -> Result<AppliedPosition> {
        self.commit(
            self.permit(ctx, KeySpan::Point(&key))?,
            RawExecutor.plan_put(ctx.keyspace, &key, value, RawWriteOptions::default())?,
        )
    }
    fn raw_batch_put(
        &self,
        ctx: &RequestContext,
        pairs: &[(UserKey, Value)],
    ) -> Result<AppliedPosition> {
        self.commit(
            self.permit(ctx, KeySpan::BatchPairs(pairs))?,
            RawExecutor.plan_batch_put(ctx.keyspace, pairs, RawWriteOptions::default())?,
        )
    }
    fn raw_batch(
        &self,
        ctx: &RequestContext,
        mutations: &[RawMutation],
    ) -> Result<AppliedPosition> {
        // One permit covers every key, so a batch that straddles a range boundary is
        // refused before anything is proposed; the apply-time fence re-checks every key.
        self.commit(
            self.permit(ctx, KeySpan::BatchMutations(mutations))?,
            RawExecutor.plan_batch(ctx.keyspace, mutations, RawWriteOptions::default())?,
        )
    }
    fn raw_delete(&self, ctx: &RequestContext, key: &[u8]) -> Result<AppliedPosition> {
        self.commit(
            self.permit(ctx, KeySpan::Point(key))?,
            RawExecutor.plan_delete(ctx.keyspace, key)?,
        )
    }
    fn raw_scan(
        &self,
        ctx: &RequestContext,
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<(UserKey, Value)>> {
        let view = self.view(ctx, KeySpan::Range { start, end })?;
        RawExecutor.scan(
            &LeaderRead::new(view.as_ref(), true, None)?,
            ctx.keyspace,
            start,
            end,
            limit,
        )
    }
    fn raw_delete_range(
        &self,
        ctx: &RequestContext,
        start: &[u8],
        end: &[u8],
    ) -> Result<DeleteRangeReceipt> {
        let barrier = self.driver.read_barrier(READ_BARRIER_DEADLINE)?;
        let _ = barrier;
        let view = self.engine.snapshot()?;
        if !end.is_empty() && start >= end {
            return Ok(DeleteRangeReceipt::default());
        }
        self.authorize(view.as_ref(), ctx, KeySpan::Range { start, end })?;
        let read = LeaderRead::new(view.as_ref(), true, None)?;
        run_delete_range(
            start,
            end,
            |remaining_start| {
                self.permit(
                    ctx,
                    KeySpan::Range {
                        start: remaining_start,
                        end,
                    },
                )
            },
            |cursor| {
                RawExecutor.plan_delete_range_chunk(
                    &read,
                    ctx.keyspace,
                    cursor,
                    start,
                    end,
                    RAW_DELETE_RANGE_CHUNK,
                )
            },
            |permit, batch| self.commit(permit, batch),
        )
    }
}

#[cfg(test)]
mod ttl_tests {
    use super::*;
    use kv9_common::raw_ttl::{ClockWatermark, TtlRejection};
    use kv9_common::{RootDigest, StoreIncarnation, TenantId};
    use kv9_raft::command::{RawTtlAction, RawTtlCommand};
    use kv9_raft::transport::{InProcHub, RaftTransport};
    use kv9_raft::{MemStateMachine, RaftGroup, RaftPeer};

    #[test]
    fn legacy_raw_authorization_checks_the_actual_snapshot_after_ttl_activation() {
        let dir = std::env::temp_dir().join(format!(
            "kv9-ttl-reader-{}",
            StoreIncarnation::mint().unwrap()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let range = DataRange {
            root: RootDigest::from_bytes([1; 32]),
            creation: RootDigest::from_bytes([2; 32]),
            region: RegionId(100),
            keyspace: KeyspaceId(100),
            tenant: TenantId(1),
            conf_ver: 1,
            version: 1,
            start: Vec::new(),
            end: Vec::new(),
            sealed: false,
        };
        let engine = Arc::new(WalEngine::open(dir.join("engine.wal")).unwrap().0);
        let storage = DiskRaftStorage::open(&dir.join("raft"), &[1]).unwrap().0;
        let mut sm = MemStateMachine::with_engine(engine.clone()).unwrap();
        sm.set_data_group(range.root, range.region, range.creation)
            .unwrap();
        let hub = InProcHub::new();
        let driver = NodeDriver::new(
            Arc::new(RaftPeer::with_storage(NodeId(1), range.region, storage).unwrap()),
            Arc::new(hub.endpoint(NodeId(1))) as Arc<dyn RaftTransport>,
            sm,
        )
        .unwrap();
        driver.peer().campaign().unwrap();
        for _ in 0..10 {
            driver.step().unwrap();
        }
        driver
            .propose(&Command::DataRange {
                expected: None,
                next: range.clone(),
            })
            .unwrap();
        for _ in 0..10 {
            driver.step().unwrap();
        }
        let group = RawGroup::new(range.clone(), engine.clone(), driver.clone(), None);
        let ctx = RequestContext {
            keyspace: range.keyspace,
            region_epoch: kv9_region::RegionEpoch {
                conf_ver: 1,
                version: 1,
            },
            origin: crate::api::RequestOrigin::from_transport("ttl-guard-test"),
        };
        let before = engine.snapshot().unwrap();
        assert!(group
            .authorize(before.as_ref(), &ctx, KeySpan::Point(b"b"))
            .is_ok());
        driver
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
        for _ in 0..10 {
            driver.step().unwrap();
        }
        let after = engine.snapshot().unwrap();
        assert!(matches!(
            group.authorize(after.as_ref(), &ctx, KeySpan::Point(b"b")),
            Err(Error::RawTtlRejected {
                reason: TtlRejection::LegacyCommand,
                ..
            })
        ));
        assert!(group
            .authorize(before.as_ref(), &ctx, KeySpan::Point(b"b"))
            .is_ok());
        drop(after);
        drop(before);
        drop(group);
        drop(driver);
        drop(engine);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod range_batch_tests {
    //! The atomic range batch contract (DESIGN §9.1): one permit covers every key, so a
    //! batch either lands whole inside its range or is refused before any proposal.
    use super::*;
    use kv9_common::{RootDigest, StoreIncarnation, TenantId};
    use kv9_raft::transport::{InProcHub, RaftTransport};
    use kv9_raft::{MemStateMachine, RaftGroup, RaftPeer};

    fn put(key: &[u8], value: &[u8]) -> RawMutation {
        RawMutation::Put {
            key: key.to_vec(),
            value: value.to_vec(),
        }
    }
    fn delete(key: &[u8]) -> RawMutation {
        RawMutation::Delete { key: key.to_vec() }
    }

    #[test]
    fn a_range_batch_is_atomic_ordered_and_confined_to_its_range() {
        let dir = std::env::temp_dir().join(format!(
            "kv9-range-batch-{}",
            StoreIncarnation::mint().unwrap()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // A bounded range: keys in [a, m) belong here, everything else is foreign.
        let range = DataRange {
            root: RootDigest::from_bytes([1; 32]),
            creation: RootDigest::from_bytes([2; 32]),
            region: RegionId(100),
            keyspace: KeyspaceId(100),
            tenant: TenantId(1),
            conf_ver: 1,
            version: 1,
            start: b"a".to_vec(),
            end: b"m".to_vec(),
            sealed: false,
        };
        let engine = Arc::new(WalEngine::open(dir.join("engine.wal")).unwrap().0);
        let storage = DiskRaftStorage::open(&dir.join("raft"), &[1]).unwrap().0;
        let mut sm = MemStateMachine::with_engine(engine.clone()).unwrap();
        sm.set_data_group(range.root, range.region, range.creation)
            .unwrap();
        let hub = InProcHub::new();
        let driver = NodeDriver::new(
            Arc::new(RaftPeer::with_storage(NodeId(1), range.region, storage).unwrap()),
            Arc::new(hub.endpoint(NodeId(1))) as Arc<dyn RaftTransport>,
            sm,
        )
        .unwrap();
        driver.peer().campaign().unwrap();
        for _ in 0..10 {
            driver.step().unwrap();
        }
        driver
            .propose(&Command::DataRange {
                expected: None,
                next: range.clone(),
            })
            .unwrap();
        for _ in 0..10 {
            driver.step().unwrap();
        }
        let group = RawGroup::new(range.clone(), engine.clone(), driver.clone(), None);
        let ctx = RequestContext {
            keyspace: range.keyspace,
            region_epoch: kv9_region::RegionEpoch {
                conf_ver: 1,
                version: 1,
            },
            origin: crate::api::RequestOrigin::from_transport("range-batch-test"),
        };
        let owner = driver.spawn(Duration::from_millis(2)).unwrap();

        // Mixed puts and deletes apply in order as ONE entry: the later entry for `a`
        // (a delete) wins over the earlier put, and `b` lands in the same position.
        let applied = group
            .raw_batch(
                &ctx,
                &[
                    put(b"a", b"1"),
                    put(b"b", b"2"),
                    delete(b"a"),
                    put(b"c", b"3"),
                ],
            )
            .unwrap();
        assert!(
            applied.index > 0,
            "a non-empty batch reaches a real position"
        );
        assert_eq!(group.raw_get(&ctx, b"a").unwrap(), None);
        assert_eq!(group.raw_get(&ctx, b"b").unwrap(), Some(b"2".to_vec()));
        assert_eq!(group.raw_get(&ctx, b"c").unwrap(), Some(b"3".to_vec()));
        let after_first = driver.status().log_last_index;

        // A batch that straddles the range end is refused as a whole: the in-range
        // key `b` is NOT written, and nothing was proposed.
        assert!(matches!(
            group.raw_batch(&ctx, &[put(b"b", b"9"), put(b"z", b"9")]),
            Err(Error::RangeCrossesRegion)
        ));
        assert_eq!(group.raw_get(&ctx, b"b").unwrap(), Some(b"2".to_vec()));
        assert_eq!(driver.status().log_last_index, after_first);

        // A key below the range start is foreign too, even as the first (anchor) key.
        assert!(matches!(
            group.raw_batch(&ctx, &[put(b"0", b"9"), put(b"b", b"9")]),
            Err(Error::RangeCrossesRegion)
        ));

        // A stale epoch (a split or seal bumped the version) refuses the whole batch
        // before planning: the client must refresh its route and retry.
        let stale = RequestContext {
            region_epoch: kv9_region::RegionEpoch {
                conf_ver: 1,
                version: 2,
            },
            ..ctx.clone()
        };
        assert!(matches!(
            group.raw_batch(&stale, &[put(b"b", b"9")]),
            Err(Error::StaleEpoch { .. })
        ));
        assert_eq!(group.raw_get(&ctx, b"b").unwrap(), Some(b"2".to_vec()));

        // Deleting an absent key is a no-op, not an error; the batch still applies.
        group
            .raw_batch(&ctx, &[delete(b"gone"), put(b"d", b"4")])
            .unwrap();
        assert_eq!(group.raw_get(&ctx, b"d").unwrap(), Some(b"4".to_vec()));

        // An empty batch proposes nothing.
        let empty = group.raw_batch(&ctx, &[]).unwrap();
        assert_eq!((empty.term, empty.index), (0, 0));

        driver.stop();
        owner.join().unwrap();
        drop(group);
        drop(driver);
        drop(engine);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

mod ttl;
