//! Independent durable DATA-store observation before the catalog transaction.
use super::*;
use kv9_common::data_read_fence::ReadFloor;
use kv9_meta::endpoint::{node_endpoint, NodeEndpoint};
use kv9_meta::retention::{
    checkpoint_intent::{checkpoint_intent, CheckpointIntent},
    data_read_fence, LedgerPlan, LedgerRequest, OwnerKind,
};
use kv9_raft::grpc::data_read_fence::{inspect_on_channel, FenceTicket};

pub(super) struct PreparedDataFence {
    request: LedgerRequest,
    intent: CheckpointIntent,
    proof: Proof,
}
#[derive(Clone, Copy)]
enum Proof {
    Existing(data_read_fence::Observation),
    Confirmed {
        source: NodeEndpoint,
        floor: ReadFloor,
    },
}
impl PreparedDataFence {
    pub(super) fn source(&self) -> Option<NodeEndpoint> {
        match self.proof {
            Proof::Existing(_) => None,
            Proof::Confirmed { source, .. } => Some(source),
        }
    }
    pub(super) fn plan(
        &self,
        view: &dyn kv9_engine::ReadView,
        root: &RootDescriptor,
        request: &LedgerRequest,
    ) -> Result<LedgerPlan> {
        if request != &self.request
            || checkpoint_intent(view, root, self.intent.pending_id())?.as_ref()
                != Some(&self.intent)
        {
            return Err(Error::Config("DATA fence witness differs".into()));
        }
        let LedgerRequest::RecordDataReadFence {
            pending,
            source,
            store,
            creation,
        } = request
        else {
            unreachable!()
        };
        match self.proof {
            Proof::Existing(existing) => {
                if data_read_fence::observation(
                    view,
                    root,
                    *source,
                    *store,
                    existing.floor.image.region,
                )? != Some(existing)
                {
                    return Err(Error::MetaNotReady(
                        "DATA floor advanced during retry".into(),
                    ));
                }
                data_read_fence::plan_retry(view, root, *pending, *source, *store, *creation)
            }
            Proof::Confirmed { source, floor } => {
                data_read_fence::plan_verified(view, root, *pending, source, floor)
            }
        }
    }
}
impl RuntimeBackend {
    pub(super) fn prepare_data_read_fence(
        &self,
        expected_root: RootDigest,
        request: &LedgerRequest,
        deadline: Instant,
    ) -> Result<Option<PreparedDataFence>> {
        let LedgerRequest::RecordDataReadFence {
            pending,
            source,
            store,
            creation,
        } = request
        else {
            return Ok(None);
        };
        self.driver.read_barrier(remaining(deadline)?)?;
        let txn = self.node.meta_raft.store.begin()?;
        let root = kv9_meta::root::certified_root(&txn)?
            .ok_or_else(|| Error::MetaNotReady("DATA fence root unavailable".into()))?;
        if root.digest() != expected_root {
            return Err(Error::Config("DATA fence root differs".into()));
        }
        let view = txn.into_view();
        let intent = checkpoint_intent(view.as_ref(), &root, pending.owner)?
            .ok_or_else(|| Error::Config("DATA fence intent absent".into()))?;
        if intent.binding(OwnerKind::Pending)?.token() != *pending {
            return Err(Error::Config("DATA fence token differs".into()));
        }
        let expected = data_read_fence::expected(&root, &intent, *source, *store, *creation)?;
        if let Some(existing) = data_read_fence::observation(
            view.as_ref(),
            &root,
            *source,
            *store,
            expected.image.region,
        )? {
            if existing.floor.covers(&expected) {
                return Ok(Some(PreparedDataFence {
                    request: request.clone(),
                    intent,
                    proof: Proof::Existing(existing),
                }));
            }
        }
        drop(view);
        let txn = self.node.meta_raft.store.begin()?;
        let route = node_endpoint(&txn, *source)?
            .filter(|r| r.active)
            .ok_or_else(|| Error::MetaNotReady("DATA fence source unavailable".into()))?;
        if route.incarnation != *store
            || route.address.port() == 0
            || route.address.ip().is_unspecified()
            || root
                .voter(*source)
                .is_some_and(|v| v.store_incarnation != *store)
        {
            return Err(Error::Config("DATA fence source store differs".into()));
        }
        drop(txn);
        let context = self.ttl_context()?;
        let observed = if *source == self.node.id {
            if context.local.identity() != expected.identity {
                return Err(Error::Config("DATA fence local store differs".into()));
            }
            self.observe_data_read_fence(root.digest(), expected.image.region, *creation, deadline)?
        } else {
            let ticket = FenceTicket::new(expected.identity, expected.image.region, *creation)?;
            let end = deadline.min(Instant::now() + Duration::from_secs(1));
            tokio::runtime::Handle::current()
                .block_on(async {
                    let channel = context.channel(route.address, end).await?;
                    inspect_on_channel(
                        channel,
                        &ticket,
                        self.node.id,
                        Some(context.token.clone()),
                        remaining(end)?,
                    )
                    .await
                })?
                .floor()
        };
        self.ensure_serving()?;
        remaining(deadline)?;
        if observed != Some(expected) {
            return Err(Error::MetaNotReady(
                "store has not confirmed the exact durable DATA floor".into(),
            ));
        }
        Ok(Some(PreparedDataFence {
            request: request.clone(),
            intent,
            proof: Proof::Confirmed {
                source: route,
                floor: expected,
            },
        }))
    }
    pub(in crate::runtime) fn observe_data_read_fence(
        &self,
        root: RootDigest,
        region: RegionId,
        creation: RootDigest,
        deadline: Instant,
    ) -> Result<Option<ReadFloor>> {
        self.ensure_serving()?;
        remaining(deadline)?;
        let context = self.ttl_context()?;
        if root != context.root.digest()
            || region.0 < 100
            || creation.as_bytes() == &[0; 32]
            || context.data_read_fences.identity() != context.local.identity()
        {
            return Err(Error::Config("DATA fence scope differs".into()));
        }
        // Read the durable provider even if this group has no active handle.
        // Selection/history records and caller assertions cannot substitute.
        let floor = context.data_read_fences.floor(region)?;
        if floor.is_some_and(|f| f.creation != creation) {
            return Err(Error::Config("DATA floor creation differs".into()));
        }
        self.ensure_serving()?;
        remaining(deadline)?;
        Ok(floor)
    }
}
