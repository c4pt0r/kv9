//! Private public-Raw dispatch handles. Ownership remains with RegionManager;
//! shutdown clears this directory and stops drivers before dropping store locks.
use super::*;
use crate::api::{
    RawReadJob, RawReadPreparation, RawWrite, RawWriteCompletion, RawWritePreparation,
};
use kv9_common::data_range::{DataRange, RANGE_KEY};
use kv9_engine::ColumnFamily;

#[derive(Default)]
pub(crate) struct RawDirectory(Mutex<BTreeMap<RegionId, Arc<RawGroup>>>);
impl RawDirectory {
    /// Key-aware resolution for a possibly split keyspace: the ONE unsealed
    /// group whose range contains `key`. Sealed parents never resolve; the
    /// per-request `authorize` still re-checks containment and the epoch.
    pub(crate) fn get_for(&self, keyspace: KeyspaceId, key: &[u8]) -> Option<Arc<RawGroup>> {
        let groups = self.0.lock().expect("raw directory poisoned");
        let mut matches = groups.values().filter(|g| {
            g.binding.keyspace == keyspace && !g.binding.sealed && g.binding.contains(key)
        });
        let only = matches.next()?.clone();
        matches.next().is_none().then_some(only)
    }
    /// Any unsealed group of the keyspace, for keyless leader hints.
    pub(crate) fn any_unsealed(&self, keyspace: KeyspaceId) -> Option<Arc<RawGroup>> {
        let groups = self.0.lock().expect("raw directory poisoned");
        groups
            .values()
            .find(|g| g.binding.keyspace == keyspace && !g.binding.sealed)
            .cloned()
    }
    pub(crate) fn scoped(&self, scope: &DataRange) -> Result<Arc<dyn RawApi>> {
        let groups = self.0.lock().expect("raw directory poisoned");
        let group = groups
            .get(&scope.region)
            .filter(|g| g.binding == *scope)
            .ok_or(Error::StaleEpoch {
                region: scope.region,
            })?;
        Ok(group.clone())
    }
    pub(crate) fn remove(&self, region: kv9_common::RegionId) {
        self.0
            .lock()
            .expect("raw directory poisoned")
            .remove(&region);
    }
    pub(crate) fn insert(&self, group: Arc<RawGroup>) {
        // REPLACE: reconciliation refreshes the binding when the committed
        // directory advances (a parent sealing, a version bump). Serving
        // authorization re-reads the engine row per request either way; a
        // stale cached binding must never keep resolving a sealed range.
        self.0
            .lock()
            .expect("raw directory poisoned")
            .insert(group.binding.region, group);
    }
    pub(crate) fn clear(&self) {
        self.0.lock().expect("raw directory poisoned").clear();
    }
    pub(crate) fn by_region(&self, region: RegionId) -> Option<Arc<RawGroup>> {
        self.0
            .lock()
            .expect("raw directory poisoned")
            .get(&region)
            .cloned()
    }
}

pub(crate) struct RawGroup {
    binding: DataRange,
    engine: Arc<WalEngine>,
    driver: Arc<NodeDriver<DiskRaftStorage, WalEngine>>,
}
struct RangePermit(kv9_raft::RegionFence);

impl RawGroup {
    pub(crate) fn new(
        binding: DataRange,
        engine: Arc<WalEngine>,
        driver: Arc<NodeDriver<DiskRaftStorage, WalEngine>>,
    ) -> Arc<Self> {
        Arc::new(Self {
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
        self.authorize(self.engine.snapshot()?.as_ref(), ctx, span)
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
