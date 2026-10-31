//! Metadata-leader routing for durable checkpoint object ownership. Calls run
//! on the checkpoint's blocking worker, with bounded admission and one budget.
use super::*;
use crate::checkpoint_retention::CheckpointLedger;
use kv9_common::retention::OwnerId;
use kv9_meta::retention::{decode_request, plan_retention, MAX_LEDGER_REQUEST_BYTES};
use kv9_raft::grpc::checkpoint_retention::{self as wire, LedgerQuery, LedgerReply};
use std::sync::Weak;

mod bootstrap;
mod data_fence;
pub(super) mod membership;
pub(super) mod origins;
mod refusal;
mod selection;
pub(super) mod timeline;

enum Verified {
    Timeline(timeline::PreparedTimeline),
    Origins(origins::PreparedOrigins),
    Membership(membership::PreparedMembership),
    DataFence(data_fence::PreparedDataFence),
    Bootstrap(bootstrap::PreparedRetirement),
    Refusal(refusal::PreparedRefusal),
    Selection(selection::PreparedSelection),
}
impl Verified {
    fn source(&self) -> Option<kv9_meta::endpoint::NodeEndpoint> {
        match self {
            Self::Timeline(v) => v.source(),
            Self::Origins(v) => v.source(),
            Self::Membership(v) => v.source(),
            Self::DataFence(v) => v.source(),
            Self::Bootstrap(v) => v.source(),
            Self::Refusal(v) => v.source(),
            Self::Selection(v) => v.source(),
        }
    }
    fn plan(
        &self,
        view: &dyn kv9_engine::ReadView,
        root: &RootDescriptor,
        request: &kv9_meta::retention::LedgerRequest,
    ) -> Result<kv9_meta::retention::LedgerPlan> {
        match self {
            Self::Timeline(v) => v.plan(view, root, request),
            Self::Origins(v) => v.plan(view, root, request),
            Self::Membership(v) => v.plan(view, root, request),
            Self::DataFence(v) => v.plan(view, root, request),
            Self::Bootstrap(v) => v.plan(view, root, request),
            Self::Refusal(v) => v.plan(view, root, request),
            Self::Selection(v) => v.plan(view, root, request),
        }
    }
}

const _: () = assert!(MAX_LEDGER_REQUEST_BYTES == wire::MAX_COMMAND_BYTES);
const _: () =
    assert!(kv9_meta::retention::checkpoint_intent::MAX_INTENT_BYTES == wire::MAX_INTENT_BYTES);
const _: () = assert!(
    kv9_meta::retention::checkpoint_admission::MAX_EXECUTOR_PAGE_BYTES
        == wire::MAX_EXECUTOR_PAGE_BYTES
);

const REQUEST_BUDGET: Duration = Duration::from_secs(2);
/// How long a ledger request queues for the catalog transaction lock (fair, FIFO) before it
/// answers "busy". One holder's commit includes a durable Raft round; at 100 ms a request
/// behind one or two holders on a loaded host was refused every time, and the data
/// checkpoint worker starved behind the metadata checkpoint and sweep traffic.
pub(super) const CATALOG_WAIT: Duration = Duration::from_millis(500);
fn remaining(deadline: Instant) -> Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| Error::MetaNotReady("checkpoint retention deadline expired".into()))
}
impl RuntimeBackend {
    fn read_checkpoint_executors(
        &self,
        expected_root: RootDigest,
        pending: OwnerId,
        after: Option<OwnerId>,
        limit: usize,
        deadline: Instant,
    ) -> Result<Vec<u8>> {
        self.ensure_serving()?;
        self.driver.read_barrier(remaining(deadline)?)?;
        let txn = self.node.meta_raft.store.begin()?;
        let root = kv9_meta::root::certified_root(&txn)?.ok_or_else(|| {
            Error::MetaNotReady("checkpoint executors require certified root".into())
        })?;
        if root.digest() != expected_root {
            return Err(Error::Config(
                "checkpoint executor read root differs".into(),
            ));
        }
        let page = kv9_meta::retention::checkpoint_admission::executors(
            txn.into_view().as_ref(),
            &root,
            pending,
            after,
            limit,
        )?
        .encode(root.digest())?;
        remaining(deadline)?;
        Ok(page)
    }
    fn read_checkpoint_intent(
        &self,
        expected_root: RootDigest,
        owner: OwnerId,
        deadline: Instant,
    ) -> Result<Option<Vec<u8>>> {
        self.ensure_serving()?;
        self.driver.read_barrier(remaining(deadline)?)?;
        let txn = self.node.meta_raft.store.begin()?;
        let root = kv9_meta::root::certified_root(&txn)?.ok_or_else(|| {
            Error::MetaNotReady("checkpoint intent requires certified root".into())
        })?;
        if root.digest() != expected_root {
            return Err(Error::Config("checkpoint intent read root differs".into()));
        }
        let intent = kv9_meta::retention::checkpoint_intent::checkpoint_intent(
            txn.into_view().as_ref(),
            &root,
            owner,
        )?;
        remaining(deadline)?;
        Ok(intent.map(|i| i.encode()))
    }
    /// The caller holds the catalog transaction lock through planning and exact
    /// receipt. Drain earlier ambiguous proposals before taking the one view;
    /// fence the final append to the same leader term as the drain.
    pub(super) fn apply_checkpoint_retention(
        &self,
        expected_root: Option<RootDigest>,
        bytes: &[u8],
        deadline: Instant,
    ) -> Result<crate::api::RetentionUpdateResult> {
        self.apply_checkpoint_retention_checked(expected_root, bytes, deadline, None)
    }
    fn apply_checkpoint_retention_checked(
        &self,
        expected_root: Option<RootDigest>,
        bytes: &[u8],
        deadline: Instant,
        verified: Option<&Verified>,
    ) -> Result<crate::api::RetentionUpdateResult> {
        if bytes.len() > MAX_LEDGER_REQUEST_BYTES {
            return Err(Error::Config("retention request exceeds byte bound".into()));
        }
        let term = observe_proposal_wait(
            &self.driver.metrics().logical_proposal_wait,
            || {
                remaining(deadline)?;
                self.driver.propose(&Command::Noop)
            },
            |at, budget| self.driver.wait_applied(at, budget),
            remaining(deadline)?,
        )?
        .term;
        let txn = self.node.meta_raft.store.begin()?;
        let root = kv9_meta::root::certified_root(&txn)?
            .ok_or_else(|| Error::MetaNotReady("retention requires a certified root".into()))?;
        if expected_root.is_some_and(|r| r != root.digest()) {
            return Err(Error::Config("retention request root differs".into()));
        }
        let request =
            decode_request(bytes, root.digest()).map_err(|e| Error::Config(e.to_string()))?;
        if let Some(route) = verified.and_then(|v| v.source()) {
            if kv9_meta::endpoint::node_endpoint(&txn, route.node)? != Some(route) {
                return Err(Error::MetaNotReady(
                    "checkpoint evidence source changed".into(),
                ));
            }
        }
        let view = txn.into_view();
        self.check_capture_owner_binding(view.as_ref(), &root, &request)?;
        let plan = match verified {
            Some(v) => v.plan(view.as_ref(), &root, &request)?,
            None => plan_retention(view.as_ref(), &root, &request)?,
        };
        let revision = plan.revision();
        let changed = plan.changed();
        let command = if changed {
            Command::from_batch(&plan.into_batch())
        } else {
            Command::Noop
        };
        let applied = observe_proposal_wait(
            &self.driver.metrics().logical_proposal_wait,
            || {
                remaining(deadline)?;
                self.driver.propose_in_term(&command, term)
            },
            |at, budget| self.driver.wait_applied(at, budget),
            remaining(deadline)?,
        )?;
        remaining(deadline)?;
        Ok(crate::api::RetentionUpdateResult {
            revision,
            changed,
            applied,
        })
    }
    pub(super) fn read_checkpoint_owner(
        &self,
        expected_root: RootDigest,
        owner: OwnerId,
        deadline: Instant,
    ) -> Result<Option<Vec<u8>>> {
        self.ensure_serving()?;
        self.driver.read_barrier(remaining(deadline)?)?;
        let txn = self.node.meta_raft.store.begin()?;
        let root = kv9_meta::root::certified_root(&txn)?
            .ok_or_else(|| Error::MetaNotReady("retention requires a certified root".into()))?;
        if root.digest() != expected_root {
            return Err(Error::Config("retention read root differs".into()));
        }
        let observation =
            kv9_meta::retention::retention_owner(txn.into_view().as_ref(), &root, owner)?
                .as_ref()
                .map(kv9_meta::retention::encode_owner_observation)
                .transpose()?;
        remaining(deadline)?;
        Ok(observation)
    }
    pub(super) fn checkpoint_retention_local(
        &self,
        root: RootDigest,
        query: &LedgerQuery,
        deadline: Instant,
    ) -> Result<LedgerReply> {
        remaining(deadline)?;
        query.validate(root)?;
        self.ensure_serving()?;
        // Reject mismatched roots/invalid commands before any catalog append.
        if self.ttl_context()?.root.digest() != root {
            return Err(Error::Config("checkpoint retention root differs".into()));
        }
        let request = match query {
            LedgerQuery::Apply(bytes) => {
                Some(decode_request(bytes, root).map_err(|e| Error::Config(e.to_string()))?)
            }
            _ => None,
        };
        let status = self.driver.status();
        if status.role != Role::Leader {
            return Err(Error::NotLeader {
                leader: status.leader_id,
            });
        }
        match query {
            LedgerQuery::CapturePlan(bytes) => self.record_capture_plan(root, bytes, deadline),
            LedgerQuery::Apply(bytes) => {
                let verified = match request {
                    Some(request) => {
                        if let Some(v) =
                            self.prepare_checkpoint_refusal(root, &request, deadline)?
                        {
                            Some(Verified::Refusal(v))
                        } else if let Some(v) =
                            self.prepare_bootstrap_retirement(root, &request, deadline)?
                        {
                            Some(Verified::Bootstrap(v))
                        } else if let Some(v) =
                            self.prepare_data_read_fence(root, &request, deadline)?
                        {
                            Some(Verified::DataFence(v))
                        } else if let Some(v) =
                            self.prepare_configuration_timeline(root, &request, deadline)?
                        {
                            Some(Verified::Timeline(v))
                        } else if let Some(v) =
                            self.prepare_configuration_origins(root, &request, deadline)?
                        {
                            Some(Verified::Origins(v))
                        } else if let Some(v) =
                            self.prepare_data_membership(root, &request, deadline)?
                        {
                            Some(Verified::Membership(v))
                        } else {
                            self.prepare_checkpoint_selection(root, &request, deadline)?
                                .map(Verified::Selection)
                        }
                    }
                    None => None,
                };
                let _guard = self
                    .node
                    .meta_raft
                    .wait_catalog_txn(deadline.min(Instant::now() + CATALOG_WAIT))
                    .ok_or_else(|| {
                        Error::MetaNotReady("checkpoint retention catalog busy".into())
                    })?;
                let r = self.apply_checkpoint_retention_checked(
                    Some(root),
                    bytes,
                    deadline,
                    verified.as_ref(),
                )?;
                Ok(LedgerReply::Applied {
                    revision: r.revision,
                    changed: r.changed,
                    applied: r.applied,
                })
            }
            LedgerQuery::Owner(owner) => Ok(LedgerReply::Owner(
                self.read_checkpoint_owner(root, *owner, deadline)?,
            )),
            LedgerQuery::Intent(owner) => Ok(LedgerReply::Intent(
                self.read_checkpoint_intent(root, *owner, deadline)?,
            )),
            LedgerQuery::Executors {
                pending,
                after,
                limit,
            } => Ok(LedgerReply::Executors(self.read_checkpoint_executors(
                root, *pending, *after, *limit, deadline,
            )?)),
        }
    }
}

/// No engine/group references and no new runtime/thread per call. The owning
/// worker must be joined before the runtime and StoreGuard are released.
pub(super) struct RoutedLedger {
    backend: Weak<RuntimeBackend>,
    runtime: tokio::runtime::Handle,
    deadline: Option<Instant>,
}
impl RoutedLedger {
    pub(super) fn executors(
        &self,
        root: RootDigest,
        pending: OwnerId,
        after: Option<OwnerId>,
        limit: usize,
    ) -> Result<kv9_meta::retention::checkpoint_admission::ExecutorPage> {
        match self.call(
            LedgerQuery::Executors {
                pending,
                after,
                limit,
            },
            Some(root),
        )? {
            LedgerReply::Executors(bytes) => {
                kv9_meta::retention::checkpoint_admission::ExecutorPage::decode(
                    &bytes, root, pending, after, limit,
                )
            }
            _ => Err(Error::MetaNotReady(
                "checkpoint executors reply differs".into(),
            )),
        }
    }
    #[cfg(test)]
    pub(super) fn intent(
        &self,
        root: &RootDescriptor,
        owner: OwnerId,
    ) -> Result<Option<kv9_meta::retention::checkpoint_intent::CheckpointIntent>> {
        match self.call(LedgerQuery::Intent(owner), Some(root.digest()))? {
            LedgerReply::Intent(bytes) => bytes
                .map(|b| {
                    kv9_meta::retention::checkpoint_intent::CheckpointIntent::decode(
                        &b, root, owner,
                    )
                })
                .transpose(),
            _ => Err(Error::MetaNotReady(
                "checkpoint intent reply differs".into(),
            )),
        }
    }
    pub(super) fn new(backend: &Arc<RuntimeBackend>, runtime: tokio::runtime::Handle) -> Self {
        Self {
            backend: Arc::downgrade(backend),
            runtime,
            deadline: None,
        }
    }
    pub(super) fn until(
        backend: &Arc<RuntimeBackend>,
        runtime: tokio::runtime::Handle,
        deadline: Instant,
    ) -> Self {
        Self {
            backend: Arc::downgrade(backend),
            runtime,
            deadline: Some(deadline),
        }
    }
    fn call(&self, query: LedgerQuery, expected_root: Option<RootDigest>) -> Result<LedgerReply> {
        let deadline = self.deadline.map_or(Instant::now() + REQUEST_BUDGET, |d| {
            d.min(Instant::now() + REQUEST_BUDGET)
        });
        remaining(deadline)?;
        let backend = self
            .backend
            .upgrade()
            .ok_or_else(|| Error::MetaNotReady("checkpoint worker stopped".into()))?;
        Self::call_backend(&backend, query, expected_root, deadline, &self.runtime)
    }
    pub(super) fn record_capture(backend: &RuntimeBackend, bytes: Vec<u8>) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(30);
        match Self::call_backend(
            backend,
            LedgerQuery::CapturePlan(bytes),
            None,
            deadline,
            &tokio::runtime::Handle::current(),
        )? {
            LedgerReply::Applied { applied, .. } => backend
                .wait_catalog_applied(applied, remaining(deadline)?)
                .map_err(|e| Error::Raft(format!("capture catalog local apply: {e}"))),
            _ => Err(Error::MetaNotReady(
                "capture catalog receipt differs".into(),
            )),
        }
    }
    pub(super) fn call_backend(
        backend: &RuntimeBackend,
        query: LedgerQuery,
        expected_root: Option<RootDigest>,
        deadline: Instant,
        runtime: &tokio::runtime::Handle,
    ) -> Result<LedgerReply> {
        let context = backend.ttl_context()?;
        let root = context.root.digest();
        if expected_root.is_some_and(|r| r != root) {
            return Err(Error::Config(
                "checkpoint ledger client root differs".into(),
            ));
        }
        query.validate(root)?;
        if let LedgerQuery::Apply(bytes) = &query {
            decode_request(bytes, root).map_err(|e| Error::Config(e.to_string()))?;
        }
        let _permit = context
            .retention_capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                Error::MetaNotReady("checkpoint ledger client capacity exhausted".into())
            })?;
        if backend.driver.status().role == Role::Leader {
            match backend.checkpoint_retention_local(root, &query, deadline) {
                Err(Error::NotLeader { .. }) => {}
                other => return other,
            }
        }
        // This synchronous facade is exclusively used from a blocking worker.
        runtime.block_on(async {
            for address in backend.ttl_metadata_candidates() {
                remaining(deadline)?;
                let connect_deadline = deadline.min(Instant::now() + Duration::from_millis(250));
                let channel = match context.channel(address, connect_deadline).await {
                    Ok(c) => c,
                    Err(_) => continue, // No RPC was submitted on a failed connection.
                };
                match wire::on_channel(
                    channel,
                    root,
                    &query,
                    backend.node.id,
                    Some(context.token.clone()),
                    remaining(deadline)?,
                )
                .await
                {
                    Err(Error::NotLeader { .. }) => continue,
                    other => return other, // Unknown writes are terminal for this call.
                }
            }
            Err(Error::MetaNotReady(
                "checkpoint metadata leader unavailable".into(),
            ))
        })
    }
}
impl CheckpointLedger for RoutedLedger {
    fn data_read_fences(&self) -> Result<Option<Arc<kv9_common::data_read_fence::DataReadFences>>> {
        Ok(Some(
            self.backend
                .upgrade()
                .ok_or_else(|| Error::MetaNotReady("checkpoint worker stopped".into()))?
                .ttl_context()?
                .data_read_fences
                .clone(),
        ))
    }

    fn drain_outbox(
        &self,
    ) -> Result<Option<Arc<crate::checkpoint_retention::drain_outbox::DrainOutbox>>> {
        Ok(self
            .backend
            .upgrade()
            .ok_or_else(|| Error::MetaNotReady("checkpoint worker stopped".into()))?
            .ttl_context()?
            .drain_outbox
            .clone())
    }
    fn executors(
        &self,
        root: RootDigest,
        pending: OwnerId,
        after: Option<OwnerId>,
        limit: usize,
    ) -> Result<kv9_meta::retention::checkpoint_admission::ExecutorPage> {
        RoutedLedger::executors(self, root, pending, after, limit)
    }
    fn local_lifetime(&self) -> Result<kv9_meta::retention::LocalLifetime> {
        Ok(self
            .backend
            .upgrade()
            .ok_or_else(|| Error::MetaNotReady("checkpoint worker stopped".into()))?
            .ttl_context()?
            .lifetime)
    }
    fn apply(&self, request: Vec<u8>) -> Result<()> {
        match self.call(LedgerQuery::Apply(request), None)? {
            LedgerReply::Applied { .. } => Ok(()),
            _ => Err(Error::MetaNotReady("checkpoint apply reply differs".into())),
        }
    }
    fn owner(&self, root: RootDigest, owner: OwnerId) -> Result<Option<Vec<u8>>> {
        match self.call(LedgerQuery::Owner(owner), Some(root))? {
            LedgerReply::Owner(observation) => {
                if let Some(bytes) = &observation {
                    kv9_meta::retention::decode_owner_observation(bytes, root, owner)?;
                }
                Ok(observation)
            }
            _ => Err(Error::MetaNotReady("checkpoint owner reply differs".into())),
        }
    }
}

impl RuntimeBackend {
    /// One retention owner as the metadata leader holds it. A scoped node keeps
    /// no retention ledger (docs/META-REPLICA-MODES.md §3b: ledger rows are
    /// full-node rows), so its ledger reads go here.
    pub(crate) fn remote_retention_owner(
        &self,
        root: &kv9_common::RootDescriptor,
        id: OwnerId,
        runtime: &tokio::runtime::Handle,
    ) -> Result<Option<kv9_meta::retention::LedgerOwner>> {
        let deadline = Instant::now() + Duration::from_secs(10);
        match RoutedLedger::call_backend(self, LedgerQuery::Owner(id), Some(root.digest()), deadline, runtime)? {
            LedgerReply::Owner(Some(bytes)) => Ok(Some(kv9_meta::retention::decode_owner_observation(
                &bytes,
                root.digest(),
                id,
            )?)),
            LedgerReply::Owner(None) => Ok(None),
            _ => Err(Error::MetaNotReady("retention owner reply differs".into())),
        }
    }

    /// A retention owner from the local ledger on a full node, from the metadata
    /// leader on a scoped node.
    pub(crate) fn retention_owner_for_mode(
        &self,
        view: &dyn kv9_engine::ReadView,
        root: &kv9_common::RootDescriptor,
        id: OwnerId,
        runtime: &tokio::runtime::Handle,
    ) -> Result<Option<kv9_meta::retention::LedgerOwner>> {
        if self.slice_watch.is_scoped() {
            self.remote_retention_owner(root, id, runtime)
        } else {
            kv9_meta::retention::retention_owner(view, root, id)
        }
    }
}

/// A routed ledger that resubmits a request whose outcome came back unknown, a bounded
/// number of times. Ledger requests are exact and idempotent: the same bytes again apply
/// once and then confirm (the lost-receipt resumption every ledger worker relies on). A
/// destination install wraps its image reader in this, because each failed attempt
/// after the generation directory exists consumes one of the eight retained install
/// generations for good (docs/JOINT-SNAPSHOT-INSTALL.md); a load blip on the ledger
/// must not spend them.
pub(crate) struct ResubmittingLedger<'a> {
    inner: &'a RoutedLedger,
    attempts: usize,
}

impl<'a> ResubmittingLedger<'a> {
    pub(crate) fn new(inner: &'a RoutedLedger) -> Self {
        Self { inner, attempts: 4 }
    }

    fn transient(error: &Error) -> bool {
        matches!(error, Error::MetaNotReady(message)
            if message.contains("outcome unconfirmed")
                || message.contains("catalog busy")
                || message.contains("leader unavailable"))
            || matches!(error, Error::NotLeader { .. })
    }

    fn again<T>(&self, mut call: impl FnMut() -> Result<T>) -> Result<T> {
        let mut last = None;
        for attempt in 0..self.attempts {
            match call() {
                Err(error) if Self::transient(&error) => {
                    last = Some(error);
                    std::thread::sleep(Duration::from_millis(200 << attempt));
                }
                other => return other,
            }
        }
        Err(last.expect("at least one attempt"))
    }
}

impl CheckpointLedger for ResubmittingLedger<'_> {
    fn data_read_fences(&self) -> Result<Option<Arc<kv9_common::data_read_fence::DataReadFences>>> {
        self.inner.data_read_fences()
    }
    fn drain_outbox(&self) -> Result<Option<Arc<crate::checkpoint_retention::drain_outbox::DrainOutbox>>> {
        self.inner.drain_outbox()
    }
    fn executors(
        &self,
        root: RootDigest,
        pending: OwnerId,
        after: Option<OwnerId>,
        limit: usize,
    ) -> Result<kv9_meta::retention::checkpoint_admission::ExecutorPage> {
        self.again(|| CheckpointLedger::executors(self.inner, root, pending, after, limit))
    }
    fn local_lifetime(&self) -> Result<kv9_meta::retention::LocalLifetime> {
        self.inner.local_lifetime()
    }
    fn apply(&self, request: Vec<u8>) -> Result<()> {
        self.again(|| CheckpointLedger::apply(self.inner, request.clone()))
    }
    fn owner(&self, root: RootDigest, owner: OwnerId) -> Result<Option<Vec<u8>>> {
        self.again(|| CheckpointLedger::owner(self.inner, root, owner))
    }
}
