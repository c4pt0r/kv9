//! Explicit TTL activation: exact local data peer, metadata-leader snapshots,
//! all-replica format confirmation, and the existing single-use append gate.
use super::*;
use futures_util::{stream, StreamExt};
use kv9_common::data_range::DataRange;
use kv9_meta::endpoint::{node_endpoint, NodeEndpoint};
use kv9_raft::driver::RawTtlActivationSubmission;
use kv9_raft::grpc::ttl_directory::{RawTtlDirectory, RawTtlStore};
use kv9_raft::grpc::{confirm_raw_ttl_format_on_channel, ConfirmedRawTtlFormat};
use kv9_raft::rawnode::ttl_activation::{RawTtlActivationProbe, MAX_ACTIVATION_WINDOW};

fn remaining(deadline: Instant) -> Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| Error::MetaNotReady("Raw TTL activation deadline expired".into()))
}

/// Resolve every full-configuration member in one catalog snapshot. The Raft
/// configuration is the membership authority; the directory supplies only the
/// exact store incarnation and current endpoint, never an extra/missing member.
pub(super) fn endpoints<E: Engine>(
    txn: &kv9_meta::MetaTxn<'_, E>,
    root: &RootDescriptor,
    binding: &DataRange,
    replicas: &[NodeId],
) -> Result<Vec<(StoreIdentity, NodeEndpoint)>> {
    let root_digest = root.digest();
    if binding.root != root_digest {
        return Err(Error::MetaNotReady(
            "Raw TTL group belongs to another root".into(),
        ));
    }
    replicas
        .iter()
        .copied()
        .map(|node| {
            let endpoint = node_endpoint(txn, node)?
                .filter(|endpoint| endpoint.active)
                .ok_or_else(|| {
                    Error::MetaNotReady(format!(
                        "Raw TTL replica {} has no active endpoint",
                        node.0
                    ))
                })?;
            let identity = StoreIdentity {
                cluster_id: root.cluster_id,
                root_digest,
                node_id: node,
                store_incarnation: endpoint.incarnation,
            };
            if endpoint.incarnation.as_bytes() == &[0; 16]
                || root
                    .voter(node)
                    .is_some_and(|voter| voter.store_incarnation != endpoint.incarnation)
            {
                return Err(Error::MetaNotReady(
                    "Raw TTL directory has an invalid store incarnation".into(),
                ));
            }
            Ok((identity, endpoint))
        })
        .collect()
}

pub(super) fn verify_endpoints<E: Engine>(
    txn: &kv9_meta::MetaTxn<'_, E>,
    stores: &[(StoreIdentity, NodeEndpoint)],
) -> Result<()> {
    for (identity, expected) in stores {
        if node_endpoint(txn, identity.node_id)? != Some(*expected) {
            return Err(Error::MetaNotReady(
                "Raw TTL replica directory changed during confirmation".into(),
            ));
        }
    }
    Ok(())
}

/// Local immutable root and format facts. The guard publishes this only after
/// the durable floor is active; no configuration flag can fabricate it.
pub(super) struct ActivationContext {
    pub(super) bootstrap_directory: Option<PathBuf>,
    pub(super) root: RootDescriptor,
    pub(super) local: ConfirmedRawTtlFormat,
    pub(super) data_read_fences: Arc<kv9_common::data_read_fence::DataReadFences>,
    pub(super) lifetime: kv9_meta::retention::LocalLifetime,
    pub(super) token: String,
    pub(super) retention_capacity: Arc<tokio::sync::Semaphore>,
    pub(super) drain_outbox: Option<Arc<crate::checkpoint_retention::drain_outbox::DrainOutbox>>,
    #[cfg(test)]
    channels: Mutex<BTreeMap<std::net::SocketAddr, tonic::transport::Channel>>,
}
impl ActivationContext {
    pub(super) fn with_drain_outbox(mut self, store: &std::path::Path) -> Result<Self> {
        self.bootstrap_directory = Some(store.to_path_buf());
        self.drain_outbox = Some(
            crate::checkpoint_retention::drain_outbox::DrainOutbox::open(
                store,
                self.local.identity(),
            )?,
        );
        Ok(self)
    }
    pub(super) fn new(
        root: RootDescriptor,
        fence: kv9_common::store_lifecycle::RawTtlFormatFence,
        token: String,
    ) -> Result<Self> {
        let data_read_fences = fence.data_read_fences();
        let local = ConfirmedRawTtlFormat::from_local(fence);
        if local.identity().root_digest != root.digest()
            || local.identity().cluster_id != root.cluster_id
            || token.is_empty()
        {
            return Err(Error::Config(
                "TTL activation context identity differs".into(),
            ));
        }
        Ok(Self {
            bootstrap_directory: None,
            lifetime: kv9_meta::retention::LocalLifetime {
                node: local.identity().node_id.0,
                store: *local.identity().store_incarnation.as_bytes(),
                process: *kv9_common::ProcessIncarnation::mint()?.as_bytes(),
            },
            root,
            local,
            data_read_fences,
            token,
            retention_capacity: Arc::new(tokio::sync::Semaphore::new(2)),
            drain_outbox: None,
            #[cfg(test)]
            channels: Mutex::default(),
        })
    }
    pub(super) async fn channel(
        &self,
        address: std::net::SocketAddr,
        deadline: Instant,
    ) -> Result<tonic::transport::Channel> {
        #[cfg(test)]
        if let Some(channel) = self.channels.lock().unwrap().get(&address) {
            return Ok(channel.clone());
        }
        let budget = remaining(deadline)?;
        let endpoint = tonic::transport::Endpoint::from_shared(format!("http://{address}"))
            .map_err(|_| Error::MetaNotReady("invalid TTL control endpoint".into()))?
            .connect_timeout(budget);
        tokio::time::timeout(budget, endpoint.connect())
            .await
            .map_err(|_| Error::MetaNotReady("TTL control connection timed out".into()))?
            .map_err(|_| Error::MetaNotReady("TTL control endpoint unavailable".into()))
    }
}

impl RuntimeBackend {
    pub(super) fn ttl_context(&self) -> Result<Arc<ActivationContext>> {
        self.ttl_activation.clone().ok_or_else(|| {
            Error::MetaNotReady("TTL activation requires a durable local format context".into())
        })
    }
    pub(super) fn ttl_directory_snapshot(
        &self,
        binding: &DataRange,
        replicas: &[NodeId],
        deadline: Instant,
    ) -> Result<RawTtlDirectory> {
        kv9_raft::grpc::ttl_directory::validate_query(binding, replicas)?;
        self.ensure_catalog_ready()?;
        self.driver.read_barrier(remaining(deadline)?)?;
        let txn = self.node.meta_raft.store.begin()?;
        let root = kv9_meta::root::certified_root(&txn)?
            .ok_or_else(|| Error::MetaNotReady("TTL directory root certificate missing".into()))?;
        if kv9_meta::data_groups::ranges::route_in(&txn, binding.keyspace, &binding.start)?
            .map(|(r, _)| r)
            != Some(binding.clone())
        {
            return Err(Error::StaleEpoch {
                region: binding.region,
            });
        }
        let stores = endpoints(&txn, &root, binding, replicas)?
            .into_iter()
            .map(|(_, e)| RawTtlStore {
                node: e.node,
                incarnation: e.incarnation,
                address: e.address,
                generation: e.generation,
                previous_address: e.previous_address,
            })
            .collect();
        let directory = RawTtlDirectory {
            root: root.digest(),
            cluster: root.cluster_id,
            binding_digest: binding.digest(),
            stores,
        };
        directory.validate(binding, replicas)?;
        remaining(deadline)?;
        Ok(directory)
    }
    pub(super) fn begin_ttl_activation(
        &self,
        binding: &DataRange,
        deadline: Instant,
    ) -> Result<(Arc<range_api::RawGroup>, RawTtlActivationProbe)> {
        self.ensure_serving()?;
        let context = self.ttl_context()?;
        if context.local.identity().node_id != self.node.id || binding.root != context.root.digest()
        {
            return Err(Error::Config(
                "TTL activation root or local store differs".into(),
            ));
        }
        let group = self
            .raw_directory
            .by_region(binding.region)
            .filter(|g| g.range_binding() == binding)
            .ok_or(Error::StaleEpoch {
                region: binding.region,
            })?;
        let probe = group.capture_parts().0.begin_raw_ttl_activation(deadline)?;
        if probe.range() != binding {
            return Err(Error::StaleEpoch {
                region: binding.region,
            });
        }
        Ok((group, probe))
    }
    pub(super) fn ttl_metadata_candidates(&self) -> Vec<std::net::SocketAddr> {
        let status = self.driver.status();
        let mut candidates = Vec::new();
        for node in status
            .leader_id
            .into_iter()
            .chain(status.voters.into_iter().map(NodeId))
            .chain(self.initial_voters.iter().map(|(node, _)| *node))
        {
            if node == self.node.id {
                continue;
            }
            if let Some(address) = self
                .resolve_registration_endpoint(node)
                .and_then(|a| a.parse().ok())
            {
                if !candidates.contains(&address) {
                    candidates.push(address);
                }
            }
            if candidates.len() == crate::client::MAX_PEERS {
                break;
            }
        }
        candidates
    }
    async fn ttl_directory(
        self: &Arc<Self>,
        binding: &DataRange,
        replicas: &[NodeId],
        deadline: Instant,
    ) -> Result<RawTtlDirectory> {
        let context = self.ttl_context()?;
        let backend = self.clone();
        let b = binding.clone();
        let ids = replicas.to_vec();
        let (local, candidates) = tokio::task::spawn_blocking(move || {
            let local = if backend.driver.status().role == Role::Leader {
                Some(backend.ttl_directory_snapshot(&b, &ids, deadline))
            } else {
                None
            };
            (local, backend.ttl_metadata_candidates())
        })
        .await
        .map_err(|_| Error::Raft("TTL directory preparation worker failed".into()))?;
        if let Some(Ok(directory)) = local {
            return Ok(directory);
        }
        for address in candidates {
            let per_peer = deadline.min(Instant::now() + Duration::from_secs(1));
            let result = async {
                let channel = context.channel(address, per_peer).await?;
                kv9_raft::grpc::ttl_directory::resolve_on_channel(
                    channel,
                    binding,
                    replicas,
                    self.node.id,
                    Some(context.token.clone()),
                    remaining(per_peer)?,
                )
                .await
            }
            .await;
            if let Ok(directory) = result {
                if directory.cluster != context.root.cluster_id {
                    return Err(Error::MetaNotReady(
                        "TTL metadata directory cluster differs".into(),
                    ));
                }
                return Ok(directory);
            }
            remaining(deadline)?;
        }
        Err(Error::MetaNotReady(
            "TTL metadata-leader snapshot unavailable".into(),
        ))
    }
    pub(super) async fn collect_ttl_activation(
        self: Arc<Self>,
        group: Arc<range_api::RawGroup>,
        probe: RawTtlActivationProbe,
    ) -> Result<RawTtlActivationSubmission> {
        let context = self.ttl_context()?;
        let deadline = probe.deadline();
        let binding = probe.range().clone();
        let replicas = probe.replicas().collect::<Vec<_>>();
        let directory = self.ttl_directory(&binding, &replicas, deadline).await?;
        let identities = directory.identities();
        let mut confirmations = probe.bind_stores(&identities)?;
        let requests = stream::iter(directory.stores.clone().into_iter().zip(identities).map(
            |(store, identity)| {
                let context = context.clone();
                async move {
                    if identity.node_id == context.local.identity().node_id {
                        if identity != context.local.identity() {
                            return Err(Error::MetaNotReady(
                                "TTL directory names another local store".into(),
                            ));
                        }
                        Ok(context.local)
                    } else {
                        let channel = context.channel(store.address, deadline).await?;
                        confirm_raw_ttl_format_on_channel(
                            channel,
                            identity,
                            context.local.identity().node_id,
                            Some(context.token.clone()),
                            remaining(deadline)?,
                        )
                        .await
                    }
                }
            },
        ))
        .buffer_unordered(8);
        tokio::pin!(requests);
        while let Some(reply) = requests.next().await {
            confirmations.confirm(reply?)?;
        }
        let prepared = confirmations.finish()?;
        // The two logs do not form an atomic directory revocation protocol.
        // Durable format facts are monotonic for each exact incarnation; any
        // observed directory transition here requires a fresh collection.
        if self.ttl_directory(&binding, &replicas, deadline).await? != directory {
            return Err(Error::MetaNotReady(
                "TTL directory changed during confirmation".into(),
            ));
        }
        tokio::task::spawn_blocking(move || {
            self.ensure_serving()?;
            if !self
                .raw_directory
                .by_region(binding.region)
                .is_some_and(|g| g.same_owner(&group))
            {
                return Err(Error::StaleEpoch {
                    region: binding.region,
                });
            }
            remaining(deadline)?;
            group.capture_parts().0.propose_raw_ttl_activation(prepared)
        })
        .await
        .map_err(|_| Error::Raft("TTL activation submission worker failed".into()))?
    }
}

impl NodeRuntime {
    /// Blocking embedded equivalent of the explicit public activation RPC.
    /// Remote metadata leadership is supported. Await the returned exact receipt;
    /// an accepted position alone is not success and is never auto-retried.
    pub fn activate_raw_ttl_format(
        &self,
        region: RegionId,
        budget: Duration,
    ) -> Result<RawTtlActivationSubmission> {
        if budget.is_zero() || budget > MAX_ACTIVATION_WINDOW {
            return Err(Error::Config(
                "Raw TTL activation budget must be within (0, 30s]".into(),
            ));
        }
        let deadline = Instant::now() + budget;
        let binding = self
            .rpc_backend
            .raw_directory
            .by_region(region)
            .ok_or(Error::RegionNotFound)?
            .range_binding()
            .clone();
        let (group, probe) = self.rpc_backend.begin_ttl_activation(&binding, deadline)?;
        self.grpc_runtime.block_on(
            self.rpc_backend
                .clone()
                .collect_ttl_activation(group, probe),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kv9_common::data_range::DataRange;
    use kv9_common::{BootstrapGeneration, RootVoter};
    use kv9_engine::MemEngine;
    use kv9_meta::MetaStore;
    use kv9_raft::rawnode::MemStorage;
    use kv9_raft::transport::{InProcHub, RaftTransport};
    use raft::prelude::ConfState;

    fn fixture() -> (MetaStore<MemEngine>, RootDescriptor, RawTtlActivationProbe) {
        let root = RootDescriptor::new(
            ClusterId::mint().unwrap(),
            BootstrapGeneration::mint().unwrap(),
            vec![RootVoter {
                node_id: NodeId(1),
                addr: "127.0.0.1:20201".parse().unwrap(),
                store_incarnation: StoreIncarnation::mint().unwrap(),
            }],
            b"ttl-directory-test",
        )
        .unwrap();
        let range = DataRange {
            root: root.digest(),
            creation: RootDigest::from_bytes([2; 32]),
            region: RegionId(100),
            keyspace: KeyspaceId(100),
            tenant: TenantId(1),
            conf_ver: 1,
            version: 1,
            start: vec![],
            end: vec![],
            sealed: false,
        };
        let mut sm = MemStateMachine::new();
        sm.set_data_group(range.root, range.region, range.creation)
            .unwrap();
        let hub = InProcHub::new();
        let peer = RaftPeer::with_storage(
            NodeId(1),
            range.region,
            MemStorage::new_with_conf_state(ConfState::from((vec![1], vec![2, 3]))),
        )
        .unwrap();
        let driver = NodeDriver::new(
            Arc::new(peer),
            Arc::new(hub.endpoint(NodeId(1))) as Arc<dyn RaftTransport>,
            sm,
        )
        .unwrap();
        driver.peer().campaign().unwrap();
        for _ in 0..5 {
            driver.step().unwrap();
        }
        driver
            .propose(&Command::DataRange {
                expected: None,
                next: range,
            })
            .unwrap();
        for _ in 0..5 {
            driver.step().unwrap();
        }
        let probe = driver
            .begin_raw_ttl_activation(Instant::now() + Duration::from_secs(10))
            .unwrap();
        let store = MetaStore::new(Arc::new(MemEngine::new()));
        let mut txn = store.begin().unwrap();
        for id in 1..=3 {
            let incarnation = if id == 1 {
                root.voters[0].store_incarnation
            } else {
                StoreIncarnation::mint().unwrap()
            };
            let mut row = RowValue::new();
            row.set(ColumnId(1), ColumnValue::Uint(id));
            row.set(
                ColumnId(2),
                ColumnValue::Text(format!("127.0.0.1:{}", 20200 + id)),
            );
            row.set(ColumnId(3), ColumnValue::Uint(2));
            row.set(ColumnId(4), ColumnValue::Uint(0));
            row.set(
                ColumnId(5),
                ColumnValue::Bytes(incarnation.as_bytes().to_vec()),
            );
            txn.insert(&NODES_DESC, &[memcmp_uint(id)], row).unwrap();
        }
        txn.commit().unwrap();
        (store, root, probe)
    }

    #[test]
    fn directory_collection_uses_one_snapshot_and_detects_endpoint_or_incarnation_changes() {
        let (store, root, probe) = fixture();
        let snapshot = store.begin().unwrap();
        let original = endpoints(
            &snapshot,
            &root,
            probe.range(),
            &probe.replicas().collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!(original.len(), 3); // Learners count even though they are not root voters.
        assert!(verify_endpoints(&snapshot, &original).is_ok());
        let mut concurrent = store.begin().unwrap();
        concurrent
            .update(
                &NODES_DESC,
                &[memcmp_uint(3)],
                vec![(
                    ColumnId(5),
                    ColumnValue::Bytes(StoreIncarnation::mint().unwrap().as_bytes().to_vec()),
                )],
            )
            .unwrap();
        concurrent.commit().unwrap();
        assert_eq!(
            endpoints(
                &snapshot,
                &root,
                probe.range(),
                &probe.replicas().collect::<Vec<_>>()
            )
            .unwrap(),
            original
        );
        assert!(verify_endpoints(&store.begin().unwrap(), &original).is_err());
        let current = endpoints(
            &store.begin().unwrap(),
            &root,
            probe.range(),
            &probe.replicas().collect::<Vec<_>>(),
        )
        .unwrap();
        let mut concurrent = store.begin().unwrap();
        concurrent
            .update(
                &NODES_DESC,
                &[memcmp_uint(2)],
                vec![
                    (ColumnId(2), ColumnValue::Text("127.0.0.1:21202".into())),
                    (ColumnId(6), ColumnValue::Uint(1)),
                    (ColumnId(7), ColumnValue::Text("127.0.0.1:20202".into())),
                ],
            )
            .unwrap();
        concurrent.commit().unwrap();
        assert!(verify_endpoints(&store.begin().unwrap(), &current).is_err());
    }

    #[test]
    fn inactive_missing_and_root_voter_replacement_cannot_be_confirmed() {
        for mode in 0..3 {
            let (store, root, probe) = fixture();
            let mut txn = store.begin().unwrap();
            match mode {
                0 => txn
                    .update(
                        &NODES_DESC,
                        &[memcmp_uint(3)],
                        vec![(ColumnId(3), ColumnValue::Uint(1))],
                    )
                    .unwrap(),
                1 => txn.delete(&NODES_DESC, &[memcmp_uint(3)]).unwrap(),
                _ => txn
                    .update(
                        &NODES_DESC,
                        &[memcmp_uint(1)],
                        vec![(
                            ColumnId(5),
                            ColumnValue::Bytes(
                                StoreIncarnation::mint().unwrap().as_bytes().to_vec(),
                            ),
                        )],
                    )
                    .unwrap(),
            }
            txn.commit().unwrap();
            assert!(endpoints(
                &store.begin().unwrap(),
                &root,
                probe.range(),
                &probe.replicas().collect::<Vec<_>>()
            )
            .is_err());
        }
    }
}

#[cfg(test)]
mod integration;
