//! History I/O happens before the catalog lock. A private witness is consumed
//! only after the metadata term drain and an exact source-directory recheck.
use super::*;
use kv9_meta::endpoint::{node_endpoint, NodeEndpoint};
use kv9_meta::retention::checkpoint_intent::{checkpoint_intent, CheckpointIntent};
use kv9_meta::retention::checkpoint_refusal::{self, Refusal};
use kv9_meta::retention::{LedgerPlan, LedgerRequest, OwnerKind};
use kv9_raft::grpc::checkpoint_history::{inspect_on_channel, HistoryTicket};

pub(super) struct PreparedRefusal {
    request: LedgerRequest,
    intent: CheckpointIntent,
    source: Option<NodeEndpoint>,
    existing: Option<Refusal>,
}
impl PreparedRefusal {
    pub(super) fn source(&self) -> Option<NodeEndpoint> {
        self.source
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
            return Err(Error::Config("checkpoint refusal witness differs".into()));
        }
        let LedgerRequest::RecordCheckpointRefusal {
            pending,
            source,
            winner,
        } = request
        else {
            return Err(Error::Config("checkpoint refusal request differs".into()));
        };
        if let Some(fact) = &self.existing {
            if checkpoint_refusal::refusal(view, root, pending.owner)?.as_ref() != Some(fact) {
                return Err(Error::Config(
                    "checkpoint refusal changed during retry".into(),
                ));
            }
            checkpoint_refusal::plan_retry(view, root, *pending, *winner, *source)
        } else {
            checkpoint_refusal::plan_verified(
                view,
                root,
                *pending,
                *winner,
                self.source
                    .ok_or_else(|| Error::Config("checkpoint refusal source absent".into()))?,
            )
        }
    }
}

impl RuntimeBackend {
    pub(super) fn prepare_checkpoint_refusal(
        &self,
        expected_root: RootDigest,
        request: &LedgerRequest,
        deadline: Instant,
    ) -> Result<Option<PreparedRefusal>> {
        let LedgerRequest::RecordCheckpointRefusal {
            pending,
            source,
            winner,
        } = request
        else {
            return Ok(None);
        };
        self.driver.read_barrier(remaining(deadline)?)?;
        let txn = self.node.meta_raft.store.begin()?;
        let root = kv9_meta::root::certified_root(&txn)?
            .ok_or_else(|| Error::MetaNotReady("checkpoint refusal root unavailable".into()))?;
        if root.digest() != expected_root {
            return Err(Error::Config("checkpoint refusal root differs".into()));
        }
        // Look at a persisted fact before requiring the historical source to
        // remain live. A lost receipt must be recoverable after its retirement.
        let view = txn.into_view();
        let intent = checkpoint_intent(view.as_ref(), &root, pending.owner)?
            .ok_or_else(|| Error::Config("checkpoint refusal intent absent".into()))?;
        if intent.binding(OwnerKind::Pending)?.token() != *pending
            || winner.as_slice() == intent.manifest()?.change_id(intent.predecessor())?
        {
            return Err(Error::Config("checkpoint refusal candidate differs".into()));
        }
        if let Some(fact) = checkpoint_refusal::refusal(view.as_ref(), &root, pending.owner)? {
            if fact.winner != *winner || fact.source != *source {
                return Err(Error::Config("immutable checkpoint refusal differs".into()));
            }
            return Ok(Some(PreparedRefusal {
                request: request.clone(),
                intent,
                source: None,
                existing: Some(fact),
            }));
        }
        drop(view);
        let txn = self.node.meta_raft.store.begin()?;
        let route = node_endpoint(&txn, *source)?
            .filter(|route| route.active)
            .ok_or_else(|| Error::MetaNotReady("checkpoint refusal source unavailable".into()))?;
        if route.incarnation.as_bytes() == &[0; 16]
            || root
                .voter(*source)
                .is_some_and(|v| v.store_incarnation != route.incarnation)
            || route.address.port() == 0
            || route.address.ip().is_unspecified()
        {
            return Err(Error::Config(
                "checkpoint refusal source identity differs".into(),
            ));
        }
        drop(txn);
        let context = self.ttl_context()?;
        let identity = StoreIdentity {
            root_digest: root.digest(),
            cluster_id: root.cluster_id,
            node_id: route.node,
            store_incarnation: route.incarnation,
        };
        let region = RegionId(intent.manifest()?.scope.region);
        let observed = if route.node == self.node.id {
            if context.local.identity() != identity {
                return Err(Error::Config(
                    "checkpoint refusal local store differs".into(),
                ));
            }
            checkpoint_sweep::local_history(self, &root, region, intent.predecessor())?
        } else {
            let ticket = HistoryTicket::new(identity, region, intent.predecessor())?;
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
                .winner_change_id()
        };
        remaining(deadline)?;
        self.ensure_serving()?;
        if observed != Some(*winner) {
            return Err(Error::MetaNotReady(
                "checkpoint refusal history is unknown or differs".into(),
            ));
        }
        Ok(Some(PreparedRefusal {
            request: request.clone(),
            intent,
            source: Some(route),
            existing: None,
        }))
    }
}
