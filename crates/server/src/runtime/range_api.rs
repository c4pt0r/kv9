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
    aggregator: WriteAggregator,
}
impl RawGroup {
    pub(crate) fn range_binding(&self) -> &DataRange {
        &self.binding
    }
}

/// Bounded proposal aggregation for PUBLIC raw writes (#20 slice): in-flight
/// SAME-EPOCH fenced writes of one group coalesce into ONE raft entry —
/// applied atomically at one exact position that every participant receives
/// as its receipt. Disabled unless KV9_PROPOSAL_BATCH_OPS > 0; the delay
/// window (KV9_PROPOSAL_BATCH_DELAY_MS, clamped to 50ms) bounds the latency
/// cost, and byte/op caps bound the entry. Catalog transactions, migration
/// verbs and every non-raw path are untouched: they keep their own
/// term-fenced one-command semantics.
pub(crate) struct WriteAggregator {
    ops_cap: usize,
    bytes_cap: usize,
    delay: Duration,
    open: Mutex<Option<Arc<OpenBatch>>>,
}

struct OpenBatch {
    inner: Mutex<Option<OpenInner>>,
    flush: tokio::sync::Notify,
}

struct OpenInner {
    fence: kv9_raft::RegionFence,
    batch: kv9_engine::WriteBatch,
    bytes: usize,
    waiters: Vec<tokio::sync::oneshot::Sender<std::result::Result<AppliedPosition, SharedError>>>,
}

/// A fan-out-safe error image: routing-critical variants stay typed so a
/// batched writer still receives its NotLeader hint or StaleEpoch signal.
#[derive(Clone)]
enum SharedError {
    NotLeader { leader: Option<NodeId> },
    StaleEpoch { region: RegionId },
    Other(String),
}

impl SharedError {
    fn capture(error: &Error) -> Self {
        match error {
            Error::NotLeader { leader } => SharedError::NotLeader { leader: *leader },
            Error::StaleEpoch { region } => SharedError::StaleEpoch { region: *region },
            other => SharedError::Other(other.to_string()),
        }
    }
    fn into_error(self) -> Error {
        match self {
            SharedError::NotLeader { leader } => Error::NotLeader { leader },
            SharedError::StaleEpoch { region } => Error::StaleEpoch { region },
            SharedError::Other(message) => Error::Raft(message),
        }
    }
}

fn batch_bytes(batch: &kv9_engine::WriteBatch) -> usize {
    batch
        .mutations()
        .iter()
        .map(|m| match m {
            kv9_engine::Mutation::Put { key, value, .. } => key.len() + value.len(),
            kv9_engine::Mutation::Delete { key, .. } => key.len(),
        })
        .sum()
}

impl WriteAggregator {
    fn from_env() -> Self {
        let ops_cap = std::env::var("KV9_PROPOSAL_BATCH_OPS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let delay_ms: u64 = std::env::var("KV9_PROPOSAL_BATCH_DELAY_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2)
            .min(50);
        Self {
            ops_cap,
            bytes_cap: 1 << 20,
            delay: Duration::from_millis(delay_ms),
            open: Mutex::new(None),
        }
    }

    fn enabled(&self) -> bool {
        self.ops_cap > 0
    }

    /// Stage one fenced write. Returns either a follower's wait on an open
    /// batch, or the leadership of a NEW batch (the caller must flush it
    /// after the delay window). Only equal-fence writes merge; a mismatch
    /// closes the open batch immediately so its leader proposes without
    /// waiting out the window.
    fn stage(&self, fence: kv9_raft::RegionFence, batch: kv9_engine::WriteBatch) -> Staged {
        let bytes = batch_bytes(&batch);
        let mut open = self.open.lock().expect("aggregator poisoned");
        if let Some(state) = open.as_ref() {
            let mut inner = state.inner.lock().expect("open batch poisoned");
            match inner.as_mut() {
                Some(current)
                    if current.fence == fence
                        && current.batch.len() + batch.len() <= self.ops_cap
                        && current.bytes + bytes <= self.bytes_cap =>
                {
                    current.batch.append(batch);
                    current.bytes += bytes;
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    current.waiters.push(tx);
                    let full = current.batch.len() >= self.ops_cap;
                    drop(inner);
                    if full {
                        state.flush.notify_waiters();
                    }
                    return Staged::Joined(rx);
                }
                Some(_) => {
                    // Fence or capacity mismatch: close the open batch now.
                    drop(inner);
                    state.flush.notify_waiters();
                }
                None => {}
            }
        }
        let state = Arc::new(OpenBatch {
            inner: Mutex::new(Some(OpenInner {
                fence,
                batch,
                bytes,
                waiters: Vec::new(),
            })),
            flush: tokio::sync::Notify::new(),
        });
        *open = Some(state.clone());
        Staged::Leader(state)
    }
}

enum Staged {
    /// This write joined an open batch; await the shared receipt.
    Joined(tokio::sync::oneshot::Receiver<std::result::Result<AppliedPosition, SharedError>>),
    /// This write opened the batch and owns its flush.
    Leader(Arc<OpenBatch>),
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
            aggregator: WriteAggregator::from_env(),
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
    /// The batching completion. A JOINED write awaits the shared receipt; the
    /// batch LEADER waits out the bounded window (or an early close), takes
    /// the merged batch, proposes it as ONE fenced command through the exact
    /// unchanged submission/retry path, and fans the one applied position —
    /// or the typed failure — out to every participant.
    fn batched_write(
        self: Arc<Self>,
        fence: kv9_raft::RegionFence,
        batch: kv9_engine::WriteBatch,
    ) -> RawWriteCompletion {
        match self.aggregator.stage(fence, batch) {
            Staged::Joined(rx) => Box::pin(async move {
                match rx.await {
                    Ok(Ok(position)) => Ok(position),
                    Ok(Err(shared)) => Err(shared.into_error()),
                    Err(_) => Err(Error::Raft(
                        "batched write leader dropped before completion".into(),
                    )),
                }
            }),
            Staged::Leader(state) => Box::pin(async move {
                let delay = self.aggregator.delay;
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = state.flush.notified() => {}
                }
                let taken = state
                    .inner
                    .lock()
                    .expect("open batch poisoned")
                    .take()
                    .expect("only the leader takes its batch");
                {
                    // Retire the slot if it still points at this batch, so a
                    // later write opens a fresh one instead of joining a
                    // taken batch.
                    let mut open = self.aggregator.open.lock().expect("aggregator poisoned");
                    if open
                        .as_ref()
                        .is_some_and(|current| Arc::ptr_eq(current, &state))
                    {
                        *open = None;
                    }
                }
                let command = Arc::new(Command::fenced_write_from_batch(taken.fence, &taken.batch));
                let deadline = Instant::now() + RAW_APPLY_DEADLINE;
                let driver = self.driver.clone();
                let submit = command.clone();
                let first = tokio::task::spawn_blocking(move || {
                    driver.propose_with_async_wait(&submit, deadline)
                })
                .await
                .map_err(|error| {
                    Error::Raft(format!("batched submission worker failed: {error}"))
                })?;
                let outcome = match first {
                    Ok(first) => {
                        finish_async_proposal(self.driver.clone(), command, first, deadline)
                            .await
                            .0
                    }
                    Err(error) => Err(error),
                };
                let shared = match &outcome {
                    Ok(position) => Ok(*position),
                    Err(error) => Err(SharedError::capture(error)),
                };
                for waiter in taken.waiters {
                    let _ = waiter.send(shared.clone());
                }
                outcome
            }),
        }
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
            if self.aggregator.enabled() {
                return Ok(self.clone().batched_write(permit.0, batch));
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

#[cfg(test)]
mod aggregator_tests {
    use super::*;

    fn aggregator(ops: usize, bytes: usize) -> WriteAggregator {
        WriteAggregator {
            ops_cap: ops,
            bytes_cap: bytes,
            delay: Duration::from_millis(1),
            open: Mutex::new(None),
        }
    }
    fn fence(version: u64) -> kv9_raft::RegionFence {
        kv9_raft::RegionFence {
            region_id: 7,
            conf_ver: 1,
            version,
        }
    }
    fn put(key: &[u8]) -> kv9_engine::WriteBatch {
        let mut batch = kv9_engine::WriteBatch::new();
        batch.put(ColumnFamily::Default, key.to_vec(), b"v".to_vec());
        batch
    }

    #[test]
    fn same_fence_writes_merge_in_arrival_order_and_share_the_receipt() {
        let agg = aggregator(8, 1 << 20);
        let Staged::Leader(state) = agg.stage(fence(1), put(b"a")) else {
            panic!("first write must lead");
        };
        assert!(matches!(agg.stage(fence(1), put(b"b")), Staged::Joined(_)));
        assert!(matches!(agg.stage(fence(1), put(b"c")), Staged::Joined(_)));
        let inner = state.inner.lock().unwrap();
        let open = inner.as_ref().unwrap();
        assert_eq!(open.batch.len(), 3, "all three merged into one entry");
        assert_eq!(open.waiters.len(), 2, "two joiners await the fan-out");
        let keys: Vec<&[u8]> = open
            .batch
            .mutations()
            .iter()
            .map(|m| match m {
                kv9_engine::Mutation::Put { key, .. } => key.as_slice(),
                kv9_engine::Mutation::Delete { key, .. } => key.as_slice(),
            })
            .collect();
        assert_eq!(keys, vec![b"a".as_slice(), b"b", b"c"], "arrival order");
    }

    #[test]
    fn a_fence_mismatch_never_merges_and_closes_the_open_batch() {
        let agg = aggregator(8, 1 << 20);
        let Staged::Leader(first) = agg.stage(fence(1), put(b"a")) else {
            panic!("first write must lead");
        };
        // An epoch change (split/seal bumps the version) opens a NEW batch.
        let Staged::Leader(second) = agg.stage(fence(2), put(b"b")) else {
            panic!("a mismatched fence must open its own batch");
        };
        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(first.inner.lock().unwrap().as_ref().unwrap().batch.len(), 1);
        assert_eq!(
            second.inner.lock().unwrap().as_ref().unwrap().batch.len(),
            1
        );
    }

    #[test]
    fn caps_bound_the_entry_and_a_taken_batch_never_admits_joiners() {
        let agg = aggregator(2, 1 << 20);
        let Staged::Leader(state) = agg.stage(fence(1), put(b"a")) else {
            panic!("first write must lead");
        };
        assert!(matches!(agg.stage(fence(1), put(b"b")), Staged::Joined(_)));
        // ops cap reached: the next write opens a new batch.
        let Staged::Leader(next) = agg.stage(fence(1), put(b"c")) else {
            panic!("a full batch must not admit more writes");
        };
        assert!(!Arc::ptr_eq(&state, &next));
        // The leader takes its batch exactly once; a joiner arriving after
        // the take must lead a fresh batch, never write into a proposed one.
        let taken = next.inner.lock().unwrap().take().unwrap();
        assert_eq!(taken.batch.len(), 1);
        assert!(matches!(agg.stage(fence(1), put(b"d")), Staged::Leader(_)));
    }

    #[test]
    fn byte_caps_close_batches_and_shared_errors_stay_typed() {
        let agg = aggregator(100, 4);
        let Staged::Leader(_) = agg.stage(fence(1), put(b"ab")) else {
            panic!("first write must lead");
        };
        // 2+1 value bytes staged; the next 3 bytes exceed the 4-byte cap.
        assert!(matches!(agg.stage(fence(1), put(b"cd")), Staged::Leader(_)));
        let not_leader = SharedError::capture(&Error::NotLeader {
            leader: Some(NodeId(3)),
        });
        assert!(matches!(
            not_leader.into_error(),
            Error::NotLeader {
                leader: Some(NodeId(3))
            }
        ));
        let stale = SharedError::capture(&Error::StaleEpoch {
            region: RegionId(9),
        });
        assert!(matches!(
            stale.into_error(),
            Error::StaleEpoch {
                region: RegionId(9)
            }
        ));
    }
}
