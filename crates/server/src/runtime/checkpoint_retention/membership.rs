//! Read-only independent history inspection. Caller supplies the historical
//! store identity; the current endpoint must match it, never replace it.
use super::*;
mod record;
use kv9_common::data_range::{DataRange, RANGE_KEY};
use kv9_common::raw_ttl::split::{ImportState, IMPORT_KEY};
use kv9_common::StoreIdentity;
use kv9_engine::ColumnFamily;
use kv9_meta::endpoint::{node_endpoint, NodeEndpoint};
use kv9_raft::grpc::data_membership::{
    inspect_on_channel, ConfirmedMembershipPage, MembershipTicket,
};
use kv9_raft::storage::membership_history::{MembershipCursor, MembershipPage};
pub(super) use record::PreparedMembership;

pub struct DataMembershipObservation {
    endpoint: NodeEndpoint,
    identity: StoreIdentity,
    reply: ConfirmedMembershipPage,
}
impl DataMembershipObservation {
    pub fn endpoint(&self) -> NodeEndpoint {
        self.endpoint
    }
    pub fn identity(&self) -> StoreIdentity {
        self.identity
    }
    pub fn page(&self) -> Option<&MembershipPage> {
        self.reply.page()
    }
    pub fn next_ticket(&self) -> Result<Option<MembershipTicket>> {
        self.reply.next_ticket()
    }
}
fn scope(engine: &WalEngine) -> Result<Option<DataRange>> {
    let view = engine.snapshot()?;
    if let Some(bytes) = view.get(ColumnFamily::Default, RANGE_KEY)? {
        return DataRange::decode(&bytes).map(Some);
    }
    view.get(ColumnFamily::Default, IMPORT_KEY)?
        .map(|bytes| ImportState::decode(&bytes).map(|state| state.spec.target().clone()))
        .transpose()
}
impl RuntimeBackend {
    pub(in crate::runtime) fn observe_data_membership(
        &self,
        root: RootDigest,
        region: RegionId,
        creation: RootDigest,
        cursor: Option<MembershipCursor>,
        deadline: Instant,
    ) -> Result<Option<MembershipPage>> {
        self.ensure_serving()?;
        remaining(deadline)?;
        let context = self.ttl_context()?;
        if root != context.root.digest() || region.0 < 100 || creation.as_bytes() == &[0; 32] {
            return Err(Error::Config("membership observation scope differs".into()));
        }
        let handle = self
            .group_handles
            .lock()
            .expect("group handles")
            .get(&region)
            .cloned()
            .or_else(|| {
                self.raw_directory.by_region(region).map(|g| {
                    let (driver, engine, _) = g.capture_parts();
                    (engine.clone(), driver.clone())
                })
            });
        let Some((engine, driver)) = handle else {
            return Ok(None);
        };
        let expected = |range: &DataRange| {
            range.root == root && range.region == region && range.creation == creation
        };
        if !scope(&engine)?.as_ref().is_some_and(expected) {
            return Err(Error::Config("membership group creation differs".into()));
        }
        if driver.status().fatal.is_some() {
            return Err(Error::Raft("membership driver failed".into()));
        }
        let page = driver.peer().membership_history_page(cursor)?;
        if !data_checkpoint::owner_matches(self, region, &driver, &engine)
            || !scope(&engine)?.as_ref().is_some_and(expected)
        {
            return Ok(None);
        }
        self.ensure_serving()?;
        remaining(deadline)?;
        Ok(Some(page))
    }
    pub(in crate::runtime) fn collect_data_membership(
        &self,
        ticket: MembershipTicket,
        deadline: Instant,
        _permit: tokio::sync::OwnedSemaphorePermit,
    ) -> Result<DataMembershipObservation> {
        self.read_data_membership_source(ticket, deadline)
    }
    fn read_data_membership_source(
        &self,
        ticket: MembershipTicket,
        deadline: Instant,
    ) -> Result<DataMembershipObservation> {
        self.ensure_serving()?;
        remaining(deadline)?;
        let context = self.ttl_context()?;
        let expected = ticket.identity();
        if expected.root_digest != context.root.digest()
            || expected.cluster_id != context.root.cluster_id
        {
            return Err(Error::Config("membership source root differs".into()));
        }
        self.driver.read_barrier(remaining(deadline)?)?;
        let txn = self.node.meta_raft.store.begin()?;
        let route = node_endpoint(&txn, expected.node_id)?
            .filter(|e| e.active)
            .ok_or_else(|| Error::MetaNotReady("membership source unavailable".into()))?;
        if route.incarnation != expected.store_incarnation
            || route.address.port() == 0
            || route.address.ip().is_unspecified()
            || context
                .root
                .voter(expected.node_id)
                .is_some_and(|v| v.store_incarnation != expected.store_incarnation)
        {
            return Err(Error::Config("membership source store differs".into()));
        }
        drop(txn);
        let reply = tokio::runtime::Handle::current().block_on(async {
            let channel = context.channel(route.address, deadline).await?;
            inspect_on_channel(
                channel,
                &ticket,
                self.node.id,
                Some(context.token.clone()),
                remaining(deadline)?.min(Duration::from_secs(2)),
            )
            .await
        })?;
        self.ensure_serving()?;
        self.driver.read_barrier(remaining(deadline)?)?;
        let txn = self.node.meta_raft.store.begin()?;
        if node_endpoint(&txn, expected.node_id)? != Some(route) {
            return Err(Error::MetaNotReady(
                "membership source endpoint changed during inspection".into(),
            ));
        }
        remaining(deadline)?;
        Ok(DataMembershipObservation {
            endpoint: route,
            identity: expected,
            reply,
        })
    }
}
impl NodeRuntime {
    /// Metadata-leader inspection of one exact source store. Reports local
    /// durable observations only, including explicit gaps/unknown. This grants
    /// no DATA image gate, retirement, release or garbage-collection authority.
    pub async fn inspect_data_membership(
        &self,
        ticket: MembershipTicket,
        budget: Duration,
    ) -> Result<DataMembershipObservation> {
        if budget.is_zero() || budget > Duration::from_secs(30) {
            return Err(Error::Config("invalid membership inspection budget".into()));
        }
        let deadline = Instant::now() + budget;
        let backend = self.rpc_backend.clone();
        let permit = backend
            .ttl_context()?
            .retention_capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::MetaNotReady("membership observation capacity busy".into()))?;
        let job = tokio::task::spawn_blocking(move || {
            backend.collect_data_membership(ticket, deadline, permit)
        });
        tokio::time::timeout(budget, job)
            .await
            .map_err(|_| Error::MetaNotReady("membership inspection deadline elapsed".into()))?
            .map_err(|_| Error::MetaNotReady("membership inspection worker failed".into()))?
    }
}
