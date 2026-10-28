//! Independent live-replica observation before entering the catalog lock.
//! Exact already-durable or superseding observations can acknowledge a retry
//! after the original source has stopped; they never assert current liveness.
use super::*;
use kv9_engine::checkpoint::CheckpointManifest;
use kv9_meta::endpoint::{node_endpoint, NodeEndpoint};
use kv9_meta::retention::checkpoint_intent::{checkpoint_intent, CheckpointIntent};
use kv9_meta::retention::checkpoint_selection::{self, Selection};
use kv9_meta::retention::{LedgerPlan, LedgerRequest, OwnerKind};
use kv9_raft::grpc::checkpoint_history::{inspect_selection_on_channel, HistoryTicket};

pub(super) struct PreparedSelection {
    request: LedgerRequest,
    intent: CheckpointIntent,
    source: Option<NodeEndpoint>,
    existing: Option<Selection>,
}
impl PreparedSelection {
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
            return Err(Error::Config("checkpoint selection witness differs".into()));
        }
        let LedgerRequest::RecordCheckpointSelection {
            pending,
            source,
            store,
        } = request
        else {
            return Err(Error::Config("checkpoint selection request differs".into()));
        };
        if let Some(existing) = &self.existing {
            if checkpoint_selection::selection(
                view,
                root,
                *source,
                *store,
                RegionId(self.intent.manifest()?.scope.region),
            )?
            .as_ref()
                != Some(existing)
            {
                return Err(Error::MetaNotReady(
                    "checkpoint selection advanced during retry".into(),
                ));
            }
            checkpoint_selection::plan_retry(view, root, *pending, *source, *store)
        } else {
            checkpoint_selection::plan_verified(
                view,
                root,
                *pending,
                self.source
                    .ok_or_else(|| Error::Config("checkpoint selection source absent".into()))?,
            )
        }
    }
}

impl RuntimeBackend {
    pub(super) fn prepare_checkpoint_selection(
        &self,
        expected_root: RootDigest,
        request: &LedgerRequest,
        deadline: Instant,
    ) -> Result<Option<PreparedSelection>> {
        let LedgerRequest::RecordCheckpointSelection {
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
            .ok_or_else(|| Error::MetaNotReady("checkpoint selection root unavailable".into()))?;
        if root.digest() != expected_root {
            return Err(Error::Config("checkpoint selection root differs".into()));
        }
        let view = txn.into_view();
        let intent = checkpoint_intent(view.as_ref(), &root, pending.owner)?
            .ok_or_else(|| Error::Config("checkpoint selection intent absent".into()))?;
        let manifest = intent.manifest()?;
        let region = RegionId(manifest.scope.region);
        if intent.binding(OwnerKind::Pending)?.token() != *pending
            || (region != META_REGION_0 && region.0 < 100)
            || (region == META_REGION_0 && root.voter(*source).is_none())
        {
            return Err(Error::Config(
                "checkpoint selection candidate differs".into(),
            ));
        }
        if let Some(existing) =
            checkpoint_selection::selection(view.as_ref(), &root, *source, *store, region)?
        {
            if existing.covers(&intent)? {
                return Ok(Some(PreparedSelection {
                    request: request.clone(),
                    intent,
                    source: None,
                    existing: Some(existing),
                }));
            }
        }
        drop(view);
        let txn = self.node.meta_raft.store.begin()?;
        let route = node_endpoint(&txn, *source)?
            .filter(|r| r.active)
            .ok_or_else(|| Error::MetaNotReady("checkpoint selection source unavailable".into()))?;
        if route.incarnation != *store
            || root
                .voter(*source)
                .is_some_and(|v| v.store_incarnation != *store)
            || route.address.port() == 0
            || route.address.ip().is_unspecified()
        {
            return Err(Error::Config(
                "checkpoint selection source store differs".into(),
            ));
        }
        drop(txn);
        let context = self.ttl_context()?;
        let identity = StoreIdentity {
            root_digest: root.digest(),
            cluster_id: root.cluster_id,
            node_id: *source,
            store_incarnation: *store,
        };
        let observed = if *source == self.node.id {
            if context.local.identity() != identity {
                return Err(Error::Config(
                    "checkpoint selection local store differs".into(),
                ));
            }
            self.observe_checkpoint_selection(
                root.digest(),
                region,
                intent.predecessor(),
                deadline,
            )?
        } else {
            let ticket = HistoryTicket::new(identity, region, intent.predecessor())?;
            let end = deadline.min(Instant::now() + Duration::from_secs(1));
            tokio::runtime::Handle::current()
                .block_on(async {
                    let channel = context.channel(route.address, end).await?;
                    inspect_selection_on_channel(
                        channel,
                        &ticket,
                        self.node.id,
                        Some(context.token.clone()),
                        remaining(end)?,
                    )
                    .await
                })?
                .selected_change_id()
        };
        remaining(deadline)?;
        self.ensure_serving()?;
        if observed.as_ref().map(|b| b.as_slice())
            != Some(manifest.change_id(intent.predecessor())?.as_slice())
        {
            return Err(Error::MetaNotReady(
                "replica has not confirmed exact checkpoint selection".into(),
            ));
        }
        Ok(Some(PreparedSelection {
            request: request.clone(),
            intent,
            source: Some(route),
            existing: None,
        }))
    }

    pub(in crate::runtime) fn observe_checkpoint_selection(
        &self,
        root: RootDigest,
        region: RegionId,
        predecessor: u64,
        deadline: Instant,
    ) -> Result<Option<[u8; 32]>> {
        self.ensure_serving()?;
        let context = self.ttl_context()?;
        remaining(deadline)?;
        if root != context.root.digest()
            || (region != META_REGION_0 && region.0 < 100)
            || predecessor == u64::MAX
        {
            return Err(Error::Config("checkpoint selection query differs".into()));
        }
        let metadata = region == META_REGION_0;
        let handle = if metadata {
            Some((
                self.node.meta_raft.store.engine().clone(),
                self.driver.clone(),
            ))
        } else {
            self.group_handles
                .lock()
                .expect("group handles")
                .get(&region)
                .cloned()
        };
        let handle = handle.or_else(|| {
            self.raw_directory.by_region(region).map(|g| {
                let (driver, engine, _) = g.capture_parts();
                (engine.clone(), driver.clone())
            })
        });
        let Some((engine, driver)) = handle else {
            return Ok(None);
        };
        let Some(selected) = engine.selected_checkpoint()? else {
            return Ok(None);
        };
        let bootstrap = if metadata {
            let directory = context.bootstrap_directory.as_ref().ok_or_else(|| {
                Error::MetaNotReady("metadata selection directory unavailable".into())
            })?;
            let grant = crate::bootstrap_retention::selected_grant(
                directory,
                &context.root,
                context.local.identity(),
                &selected,
            )?;
            let Some(grant) = grant.filter(|g| g.predecessor == predecessor) else {
                return Ok(None);
            };
            Some((directory, grant))
        } else {
            None
        };
        let pair = driver.manifest_pair(region.0)?;
        if pair.generation < predecessor + 1 {
            return Ok(None);
        }
        let Some((term, index, bytes)) = driver.manifest_at(region.0, predecessor + 1)? else {
            return Ok(None);
        };
        let manifest = CheckpointManifest::decode(&bytes)?;
        if manifest.encode()? != bytes
            || manifest.term != term
            || manifest.index != index
            || manifest.scope.cluster != context.root.cluster_id.to_string()
            || manifest.scope.region != region.0
        {
            return Err(Error::Engine("checkpoint selection history differs".into()));
        }
        if manifest != selected {
            return Ok(None);
        }
        let change = manifest.change_id(predecessor)?;
        if pair.generation == predecessor + 1 && pair.last_change_id != change {
            return Err(Error::Engine("checkpoint selection pair differs".into()));
        }
        if !matches!(
            driver
                .peer()
                .configuration_at_committed(manifest.position())?,
            kv9_raft::storage::ConfigurationLookup::Found(_)
        ) {
            return Err(Error::MetaNotReady(
                "checkpoint selection history unavailable".into(),
            ));
        }
        if (!metadata && !data_checkpoint::owner_matches(self, region, &driver, &engine))
            || engine.selected_checkpoint()?.as_ref() != Some(&manifest)
        {
            return Ok(None);
        }
        if let Some((directory, grant)) = bootstrap {
            if crate::bootstrap_retention::selected_grant(
                directory,
                &context.root,
                context.local.identity(),
                &manifest,
            )?
            .as_ref()
                != Some(&grant)
            {
                return Ok(None);
            }
        }
        self.ensure_serving()?;
        remaining(deadline)?;
        Ok(Some(change.try_into().map_err(|_| {
            Error::Engine("checkpoint selection change ID differs".into())
        })?))
    }
}
