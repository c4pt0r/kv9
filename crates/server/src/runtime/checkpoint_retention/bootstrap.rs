//! Independent observation of a durable read-admission fence. A fully serving
//! source has finished its bootstrap GET scope while retaining its StoreGuard.
use super::*;
use kv9_common::retention::OwnerToken;
use kv9_common::StoreIdentity;
use kv9_meta::endpoint::{node_endpoint, NodeEndpoint};
use kv9_meta::retention::{
    bootstrap_reader,
    checkpoint_intent::{checkpoint_intent, CheckpointIntent},
    LedgerPlan, LedgerRequest,
};
use kv9_raft::grpc::checkpoint_history::{inspect_bootstrap_fence_on_channel, HistoryTicket};

pub(super) struct PreparedRetirement {
    request: LedgerRequest,
    identity: StoreIdentity,
    proof: RetirementProof,
}
enum RetirementProof {
    Released,
    Fence {
        intent: CheckpointIntent,
        source: NodeEndpoint,
    },
}
impl PreparedRetirement {
    pub(super) fn source(&self) -> Option<NodeEndpoint> {
        match &self.proof {
            RetirementProof::Released => None,
            RetirementProof::Fence { source, .. } => Some(*source),
        }
    }
    pub(super) fn plan(
        &self,
        view: &dyn kv9_engine::ReadView,
        root: &RootDescriptor,
        request: &LedgerRequest,
    ) -> Result<LedgerPlan> {
        if request != &self.request {
            return Err(Error::Config("bootstrap retirement witness differs".into()));
        }
        let LedgerRequest::RetireBootstrapRead { pending, .. } = request else {
            unreachable!()
        };
        match &self.proof {
            RetirementProof::Released => {
                bootstrap_reader::plan_retry(view, root, *pending, self.identity)
            }
            RetirementProof::Fence { intent, .. } => {
                if checkpoint_intent(view, root, intent.pending_id())?.as_ref() != Some(intent) {
                    return Err(Error::Config("bootstrap retirement intent changed".into()));
                }
                bootstrap_reader::plan_verified(view, root, *pending, self.identity)
            }
        }
    }
}
impl RuntimeBackend {
    pub(super) fn prepare_bootstrap_retirement(
        &self,
        expected_root: RootDigest,
        request: &LedgerRequest,
        deadline: Instant,
    ) -> Result<Option<PreparedRetirement>> {
        let LedgerRequest::RetireBootstrapRead {
            pending,
            source,
            store,
        } = request
        else {
            return Ok(None);
        };
        self.driver.read_barrier(remaining(deadline)?)?;
        let txn = self.node.meta_raft.store.begin()?;
        let root = kv9_meta::root::certified_root(&txn)?
            .ok_or_else(|| Error::MetaNotReady("bootstrap retirement root absent".into()))?;
        if root.digest() != expected_root {
            return Err(Error::Config("bootstrap retirement root differs".into()));
        }
        let view = txn.into_view();
        let identity = StoreIdentity {
            root_digest: root.digest(),
            cluster_id: root.cluster_id,
            node_id: *source,
            store_incarnation: *store,
        };
        identity.verify(&root, *source)?;
        if bootstrap_reader::plan_retry(view.as_ref(), &root, *pending, identity).is_ok() {
            return Ok(Some(PreparedRetirement {
                request: request.clone(),
                identity,
                proof: RetirementProof::Released,
            }));
        }
        let intent = checkpoint_intent(view.as_ref(), &root, pending.owner)?
            .ok_or_else(|| Error::Config("bootstrap retirement intent absent".into()))?;
        let manifest = intent.manifest()?;
        if *pending
            != (OwnerToken {
                owner: intent.pending_id(),
                generation: std::num::NonZeroU64::new(1).unwrap(),
            })
            || manifest.scope.region != META_REGION_0.0
        {
            return Err(Error::Config(
                "bootstrap retirement candidate differs".into(),
            ));
        }
        drop(view);
        let txn = self.node.meta_raft.store.begin()?;
        let route = node_endpoint(&txn, *source)?
            .filter(|r| r.active)
            .ok_or_else(|| Error::MetaNotReady("bootstrap fence source unavailable".into()))?;
        if route.incarnation != *store
            || route.address.port() == 0
            || route.address.ip().is_unspecified()
        {
            return Err(Error::Config("bootstrap fence source differs".into()));
        }
        drop(txn);
        let context = self.ttl_context()?;
        let observed = if *source == self.node.id {
            if context.local.identity() != identity {
                return Err(Error::Config(
                    "bootstrap fence local identity differs".into(),
                ));
            }
            self.observe_bootstrap_read_fence(root.digest(), intent.predecessor(), deadline)?
        } else {
            let ticket = HistoryTicket::new(identity, META_REGION_0, intent.predecessor())?;
            let end = deadline.min(Instant::now() + Duration::from_secs(1));
            tokio::runtime::Handle::current()
                .block_on(async {
                    let channel = context.channel(route.address, end).await?;
                    inspect_bootstrap_fence_on_channel(
                        channel,
                        &ticket,
                        self.node.id,
                        Some(context.token.clone()),
                        remaining(end)?,
                    )
                    .await
                })?
                .image_digest()
        };
        self.ensure_serving()?;
        remaining(deadline)?;
        if observed != Some(*RootDigest::sha256(&manifest.encode()?).as_bytes()) {
            return Err(Error::MetaNotReady(
                "store has not durably fenced the exact bootstrap grant".into(),
            ));
        }
        Ok(Some(PreparedRetirement {
            request: request.clone(),
            identity,
            proof: RetirementProof::Fence {
                intent,
                source: route,
            },
        }))
    }

    pub(in crate::runtime) fn observe_bootstrap_read_fence(
        &self,
        root: RootDigest,
        predecessor: u64,
        deadline: Instant,
    ) -> Result<Option<[u8; 32]>> {
        self.ensure_serving()?;
        remaining(deadline)?;
        let context = self.ttl_context()?;
        if root != context.root.digest() || predecessor == u64::MAX {
            return Err(Error::Config("bootstrap fence scope differs".into()));
        }
        let path = context
            .bootstrap_directory
            .as_ref()
            .ok_or_else(|| Error::MetaNotReady("bootstrap fence directory unavailable".into()))?;
        let engine = self.node.meta_raft.store.engine();
        let selected = engine.selected_checkpoint()?;
        let observed = crate::bootstrap_retention::fenced_image(
            path,
            context.local.identity(),
            predecessor,
            selected.as_ref(),
        )?;
        let observed = if let Some(fence) = observed {
            let Some((term, index, bytes)) =
                self.driver.manifest_at(META_REGION_0.0, predecessor + 1)?
            else {
                return Ok(None);
            };
            let manifest = kv9_engine::checkpoint::CheckpointManifest::decode(&bytes)?;
            let intent = CheckpointIntent::new(&context.root, &manifest, predecessor)?;
            if manifest.term != term
                || manifest.index != index
                || manifest.scope.region != META_REGION_0.0
                || manifest.encode()? != bytes
                || fence.pending != intent.pending_id()
                || fence.image != *RootDigest::sha256(&bytes).as_bytes()
            {
                return Err(Error::Engine(
                    "bootstrap fence and committed descriptor differ".into(),
                ));
            }
            Some(fence.image)
        } else {
            None
        };
        if engine.selected_checkpoint()? != selected {
            return Err(Error::MetaNotReady(
                "metadata selection changed during fence observation".into(),
            ));
        }
        self.ensure_serving()?;
        remaining(deadline)?;
        Ok(observed)
    }
}
