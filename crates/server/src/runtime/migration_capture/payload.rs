//! Durable copies of the original capture at existing data voters. No target
//! learner admission, image replanning, remote pin release or object deletion.
use super::*;
use kv9_engine::checkpoint::{TransferJournal, TransferReader};
use kv9_raft::grpc::migration_payload::{self, PayloadExport, MAX_TRANSFER_TIME};
use std::collections::BTreeSet;
use std::io::Read;

const REPLICA_SLOT: &str = "capture-replica.plan";
fn remaining(deadline: Instant) -> Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| Error::MetaNotReady("capture payload deadline".into()))
}
fn attempt_deadline(deadline: Instant, candidates_left: usize) -> Result<Instant> {
    let divisor = u32::try_from(candidates_left)
        .map_err(|_| Error::Config("too many capture candidates".into()))?;
    let share = remaining(deadline)?
        .checked_div(divisor)
        .filter(|d| d.as_micros() != 0)
        .ok_or_else(|| Error::MetaNotReady("capture candidate budget exhausted".into()))?;
    // Reserve time for later voters when an earlier endpoint is unreachable
    // or accepts the connection but never completes the request.
    Ok((Instant::now() + share).min(deadline))
}
fn voters(descriptor: &CaptureDescriptor) -> BTreeSet<u64> {
    let conf = &descriptor.image().configuration;
    conf.voters
        .iter()
        .chain(&conf.voters_outgoing)
        .copied()
        .collect()
}
fn quorum(voters: &[u64], outgoing: &[u64], copies: &BTreeSet<u64>) -> bool {
    let majority = |ids: &[u64]| {
        !ids.is_empty() && ids.iter().filter(|id| copies.contains(id)).count() > ids.len() / 2
    };
    majority(voters) && (outgoing.is_empty() || majority(outgoing))
}
struct OwnedExport {
    reader: TransferReader,
    _group: Arc<range_api::RawGroup>,
}
impl Read for OwnedExport {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        self.reader.read(bytes)
    }
}

impl RuntimeBackend {
    fn payload_group(
        &self,
        root: RootDigest,
        region: RegionId,
        operation: [u8; 16],
        digest: RootDigest,
    ) -> Result<(Arc<range_api::RawGroup>, CaptureDescriptor)> {
        self.ensure_serving()?;
        let context = self.ttl_context()?;
        if context.root.digest() != root {
            return Err(Error::Config("capture payload root differs".into()));
        }
        let descriptor = kv9_meta::data_groups::capture::committed_capture(
            self.node.meta_raft.store.begin()?.into_view().as_ref(),
            &context.root,
            operation,
        )?
        .ok_or_else(|| Error::MetaNotReady("capture catalog not applied locally".into()))?
        .descriptor()
        .clone();
        if descriptor.digest() != digest
            || descriptor.request().range.region != region
            || !voters(&descriptor).contains(&context.local.identity().node_id.0)
        {
            return Err(Error::Config(
                "capture payload is not assigned to this data voter".into(),
            ));
        }
        if kv9_meta::data_groups::abort::committed_aborts(&self.node.meta_raft.store)?
            .iter()
            .any(|a| a.operation() == operation)
        {
            return Err(Error::Config(
                "aborted capture cannot transfer payload".into(),
            ));
        }
        let group = self
            .raw_directory
            .by_region(region)
            .ok_or_else(|| Error::MetaNotReady("capture voter unavailable locally".into()))?;
        let (driver, engine, binding) = group.capture_parts();
        kv9_raft::snapshot_install::capture::check_capture_replica(
            driver,
            engine,
            context.local.identity(),
            &descriptor,
        )?;
        if binding != &descriptor.request().range {
            return Err(Error::MetaNotReady(
                "capture directory binding differs".into(),
            ));
        }
        let current = kv9_meta::data_groups::ranges::committed_ranges(&self.node.meta_raft.store)?;
        if !current.iter().any(|r| r.range() == binding) {
            return Err(Error::MetaNotReady(
                "capture range is no longer current".into(),
            ));
        }
        Ok((group, descriptor))
    }

    fn local_payload(
        &self,
        group: &range_api::RawGroup,
        descriptor: &CaptureDescriptor,
    ) -> Result<Option<TransferJournal>> {
        let uploader = self
            .uploader
            .as_ref()
            .ok_or_else(|| Error::MetaNotReady("capture uploader unavailable".into()))?;
        let parent = group
            .capture_parts()
            .1
            .path()
            .parent()
            .ok_or_else(|| Error::Config("capture directory missing".into()))?
            .to_path_buf();
        let source = self.ttl_context()?.local.identity() == descriptor.request().source;
        for slot in if source {
            vec!["capture.plan", REPLICA_SLOT]
        } else {
            vec![REPLICA_SLOT]
        } {
            let path = parent.join(slot);
            if !path
                .try_exists()
                .map_err(|e| Error::Engine(e.to_string()))?
            {
                continue;
            }
            let journal = uploader.transfer_journal(path)?;
            if let Some(pending) = journal.load()? {
                if pending.metadata() == descriptor.encode()
                    && pending.manifest() == descriptor.manifest()
                {
                    return Ok(Some(journal));
                }
                let old = CaptureDescriptor::decode(pending.metadata())?;
                if old.request().operation == descriptor.request().operation {
                    return Err(Error::Config(
                        "local capture conflicts with committed descriptor".into(),
                    ));
                }
            }
        }
        Ok(None)
    }

    fn payload_endpoint(&self, store: StoreIdentity) -> Result<std::net::SocketAddr> {
        let context = self.ttl_context()?;
        if store.root_digest != context.root.digest() || store.cluster_id != context.root.cluster_id
        {
            return Err(Error::Config("capture payload store scope differs".into()));
        }
        kv9_meta::endpoint::node_endpoint(&self.node.meta_raft.store.begin()?, store.node_id)?
            .filter(|e| e.active && e.incarnation == store.store_incarnation)
            .map(|e| e.address)
            .ok_or_else(|| Error::MetaNotReady("capture payload endpoint unavailable".into()))
    }
    fn payload_candidates(&self, descriptor: &CaptureDescriptor) -> Result<Vec<StoreIdentity>> {
        let context = self.ttl_context()?;
        let txn = self.node.meta_raft.store.begin()?;
        let mut stores = Vec::new();
        for id in voters(descriptor) {
            if id == self.node.id.0 {
                continue;
            }
            if let Some(route) =
                kv9_meta::endpoint::node_endpoint(&txn, NodeId(id))?.filter(|e| e.active)
            {
                let store = StoreIdentity {
                    cluster_id: context.root.cluster_id,
                    root_digest: context.root.digest(),
                    node_id: route.node,
                    store_incarnation: route.incarnation,
                };
                if store.node_id == descriptor.request().source.node_id
                    && store != descriptor.request().source
                {
                    continue;
                }
                stores.push(store);
            }
        }
        Ok(stores)
    }
    fn payload_terminal(&self, old: &CaptureDescriptor) -> Result<bool> {
        let request = old.request();
        let subject = *RootDigest::sha256(&old.manifest().encode()?).as_bytes();
        let exact =
            |op, root, region, destination: kv9_meta::data_groups::InitialReplica, selected| {
                op == request.operation
                    && root == request.range.root
                    && region == request.range.region
                    && destination.node == request.destination.node_id
                    && destination.incarnation == request.destination.store_incarnation
                    && selected == subject
            };
        Ok(
            kv9_meta::data_groups::abort::committed_aborts(&self.node.meta_raft.store)?
                .iter()
                .any(|a| {
                    exact(
                        a.operation(),
                        a.root(),
                        a.region(),
                        a.destination(),
                        a.subject(),
                    )
                })
                || kv9_meta::data_groups::evidence::committed_install_evidence(
                    &self.node.meta_raft.store,
                )?
                .iter()
                .any(|e| {
                    exact(
                        e.operation(),
                        e.root(),
                        e.region(),
                        e.destination(),
                        e.subject(),
                    ) && e.image_digest() == old.image().image_digest
                        && e.cut() == old.manifest().position()
                }),
        )
    }

    pub(in crate::runtime) fn export_capture_payload(
        &self,
        root: RootDigest,
        region: RegionId,
        operation: [u8; 16],
        digest: RootDigest,
        deadline: Instant,
    ) -> Result<PayloadExport> {
        remaining(deadline)?;
        let (group, descriptor) = self.payload_group(root, region, operation, digest)?;
        let journal = self
            .local_payload(&group, &descriptor)?
            .ok_or_else(|| Error::MetaNotReady("capture payload absent on voter".into()))?;
        let reader = journal
            .into_reader()?
            .ok_or_else(|| Error::MetaNotReady("capture payload disappeared".into()))?;
        let length = TransferJournal::encoded_len(descriptor.encode(), descriptor.manifest())?;
        if reader.file_len() != length {
            return Err(Error::Config("capture payload length differs".into()));
        }
        remaining(deadline)?;
        Ok(PayloadExport {
            reader: Box::new(OwnedExport {
                reader,
                _group: group,
            }),
            length,
        })
    }

    pub(in crate::runtime) fn copy_capture_payload(
        &self,
        root: RootDigest,
        region: RegionId,
        operation: [u8; 16],
        digest: RootDigest,
        source: StoreIdentity,
        deadline: Instant,
    ) -> Result<()> {
        remaining(deadline)?;
        let (group, descriptor) = self.payload_group(root, region, operation, digest)?;
        if source.node_id == self.node.id || !voters(&descriptor).contains(&source.node_id.0) {
            return Err(Error::Config(
                "capture payload source is not another image voter".into(),
            ));
        }
        if self.local_payload(&group, &descriptor)?.is_some() {
            return Ok(());
        }
        let uploader = self
            .uploader
            .as_ref()
            .ok_or_else(|| Error::MetaNotReady("capture uploader unavailable".into()))?;
        let parent = group
            .capture_parts()
            .1
            .path()
            .parent()
            .ok_or_else(|| Error::Config("capture directory absent".into()))?
            .to_path_buf();
        let mut journal = uploader.transfer_journal(parent.join(REPLICA_SLOT))?;
        if let Some(old) = journal.load()? {
            let old_descriptor = CaptureDescriptor::decode(old.metadata())?;
            if old.manifest() != old_descriptor.manifest()
                || old_descriptor.request().operation == operation
                || old_descriptor.request().range.root != root
                || old_descriptor.request().range.region != region
                || old_descriptor.request().range.creation != descriptor.request().range.creation
                || !self.payload_terminal(&old_descriptor)?
            {
                return Err(Error::MetaNotReady(
                    "capture replica slot is still owned by another operation".into(),
                ));
            }
            drop(old);
            journal.clear_settled()?;
        }
        let address = self.payload_endpoint(source)?;
        let context = self.ttl_context()?;
        let reader = tokio::runtime::Handle::current().block_on(async {
            let channel = context.channel(address, deadline).await?;
            migration_payload::reader_on_channel(
                channel,
                source,
                &descriptor,
                self.node.id,
                Some(context.token.clone()),
                remaining(deadline)?,
            )
            .await
        })?;
        journal.stage_received(descriptor.encode(), descriptor.manifest(), reader)?;
        remaining(deadline)?;
        // Recheck actual ownership before acknowledging. Publication remains
        // valid retry data even if this later check or the receipt is lost.
        self.payload_group(root, region, operation, digest)?;
        Ok(())
    }

    pub(super) fn ensure_local_capture(
        &self,
        descriptor: &CaptureDescriptor,
    ) -> Result<TransferJournal> {
        let request = descriptor.request();
        let (group, current) = self.payload_group(
            request.range.root,
            request.range.region,
            request.operation,
            descriptor.digest(),
        )?;
        if let Some(journal) = self.local_payload(&group, &current)? {
            return Ok(journal);
        }
        let deadline = Instant::now() + MAX_TRANSFER_TIME;
        let candidates = self.payload_candidates(descriptor)?;
        for (i, source) in candidates.iter().copied().enumerate() {
            let attempt = attempt_deadline(deadline, candidates.len() - i)?;
            if self
                .copy_capture_payload(
                    request.range.root,
                    request.range.region,
                    request.operation,
                    descriptor.digest(),
                    source,
                    attempt,
                )
                .is_ok()
            {
                return self
                    .local_payload(&group, descriptor)?
                    .ok_or_else(|| Error::MetaNotReady("capture copy disappeared".into()));
            }
        }
        Err(Error::MetaNotReady(
            "no complete original capture payload is available".into(),
        ))
    }

    pub(in crate::runtime) fn ensure_capture_replicas(
        &self,
        descriptor: &CaptureDescriptor,
    ) -> Result<()> {
        // Release every file lock before the remote voter calls back to Fetch.
        drop(self.ensure_local_capture(descriptor)?);
        let local = self.ttl_context()?.local.identity();
        let mut copies = BTreeSet::from([local.node_id.0]);
        let conf = &descriptor.image().configuration;
        if quorum(&conf.voters, &conf.voters_outgoing, &copies) {
            return Ok(());
        }
        let context = self.ttl_context()?;
        let deadline = Instant::now() + MAX_TRANSFER_TIME;
        let candidates = self.payload_candidates(descriptor)?;
        for (i, store) in candidates.iter().copied().enumerate() {
            let attempt = attempt_deadline(deadline, candidates.len() - i)?;
            let Ok(address) = self.payload_endpoint(store) else {
                continue;
            };
            let result = tokio::runtime::Handle::current().block_on(async {
                let channel = context.channel(address, attempt).await?;
                migration_payload::replicate_on_channel(
                    channel,
                    store,
                    local,
                    descriptor,
                    self.node.id,
                    Some(context.token.clone()),
                    remaining(attempt)?,
                )
                .await
            });
            if result.is_ok() {
                copies.insert(store.node_id.0);
            }
            if quorum(&conf.voters, &conf.voters_outgoing, &copies) {
                return Ok(());
            }
        }
        Err(Error::MetaNotReady(
            "original capture payload has not reached its voter quorums".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durable_copies_require_both_joint_majorities_and_never_count_learners() {
        let voters = &[1, 2, 3];
        assert!(!quorum(voters, &[], &BTreeSet::from([1, 4, 5])));
        assert!(quorum(voters, &[], &BTreeSet::from([1, 2])));
        let outgoing = &[3, 6, 7];
        assert!(!quorum(voters, outgoing, &BTreeSet::from([1, 2, 6])));
        assert!(!quorum(voters, outgoing, &BTreeSet::from([3, 6, 7])));
        assert!(quorum(voters, outgoing, &BTreeSet::from([1, 3, 6])));
        assert!(!quorum(&[], outgoing, &BTreeSet::from([1, 3, 6])));
    }
}
