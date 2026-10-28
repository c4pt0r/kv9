//! Enroll a selected metadata checkpoint while another metadata quorum serves.
//! Local preflight supplies the exact image/window before any GET. It does not
//! grant authority: a full Published Version and confirmed persistent grant do.
use super::*;
use kv9_common::store_lifecycle::StoreGuard;
use kv9_common::{retention::OwnerId, NodeId, META_REGION_0};
use kv9_engine::{EngineReplay, WalEngine};
use kv9_raft::grpc::checkpoint_retention::{self as wire, LedgerQuery, LedgerReply};
use kv9_raft::storage::DiskRaftStorage;
use std::time::{Duration, Instant};

/// The production startup composition, under the original store lock. The
/// factory is lazy: fresh stores and already authorized cold boots never need
/// a network quorum. Full base/WAL/configuration checks still run afterwards.
pub(crate) fn open_metadata<L: CheckpointLedger>(
    guard: &StoreGuard,
    storage: &mut DiskRaftStorage,
    catalog: &Path,
    reader: &BootstrapReader<'_>,
    ledger: impl FnOnce() -> Result<L>,
) -> Result<(
    WalEngine,
    EngineReplay,
    Option<kv9_meta::recovery::LocalRecoveryAnchor>,
)> {
    if reader.directory != guard.directory() || catalog.parent() != Some(guard.directory()) {
        return Err(error("startup store directory differs"));
    }
    reader
        .identity
        .verify(reader.root, reader.identity.node_id)?;
    // Even a plain WAL open must not run an older durable writer format. A
    // missing lifecycle is still handled by certified local recovery below;
    // do not create or adopt a store identity before that recovery succeeds.
    if guard.record().is_some() {
        guard.require_current_writer_format()?;
        guard.verify(&reader.identity)?;
    }
    let plan = kv9_raft::state_machine::checkpoint_recovery::inspect_checkpoint_read(
        storage,
        catalog,
        reader.root.cluster_id.to_string(),
        META_REGION_0.0,
    )?;
    if let Some(manifest) = plan.manifest() {
        let predecessor = plan
            .predecessor()
            .ok_or_else(|| error("publication absent"))?;
        enroll(guard, reader, manifest, predecessor, ledger)?;
    }
    kv9_meta::recovery::open_initial_checkpoint_engine_with_reader(
        storage,
        catalog,
        Some(reader),
        reader.root,
    )
}

pub(super) fn enroll<L: CheckpointLedger>(
    guard: &StoreGuard,
    reader: &BootstrapReader<'_>,
    manifest: &CheckpointManifest,
    predecessor: u64,
    ledger: impl FnOnce() -> Result<L>,
) -> Result<()> {
    let BootstrapReader {
        directory,
        root,
        identity,
        ..
    } = reader;
    if *directory != guard.directory() {
        return Err(error("startup store directory differs"));
    }
    // An existing offline receipt authorizes this exact reader only after the
    // current store format is durable. Check before the idempotent fast path.
    guard.raw_ttl_format_fence(root, identity)?;
    let mut state = load(directory, *identity)?;
    let digest = image(manifest)?;
    let existing = state.active.iter().find(|g| g.image == digest).copied();
    // Preserve the original window when identical bytes were published again.
    let predecessor = existing.map_or(predecessor, |g| g.predecessor);
    let intent = CheckpointIntent::new(root, manifest, predecessor)?;
    if let Some(grant) = existing {
        if grant.pending.is_some_and(|p| p != intent.pending_id()) {
            return Err(error("startup grant pending differs"));
        }
        if grant.authorized {
            return state::sync(directory);
        }
    }
    // Before full recovery this process cannot prove prior reads drained to a
    // remote inspector. Never erase another slot or undo a durable read fence.
    if state.retiring.iter().any(|g| g.image == digest)
        || state.active.iter().any(|g| g.pending.is_none())
        || (existing.is_none()
            && (state.active.len() + state.retiring.len() >= 2
                || state.active.iter().chain(&state.retiring).any(|g| {
                    g.predecessor == predecessor || g.pending == Some(intent.pending_id())
                })))
    {
        return Err(error(
            "untracked selected image or unresolved bootstrap state",
        ));
    }
    let ledger = ledger()?;
    let source = intent.binding(OwnerKind::Version)?;
    let observed = ledger
        .owner(root.digest(), source.descriptor.id)?
        .ok_or_else(|| error("startup requires exact Published Version"))?;
    let owner = decode_owner_observation(&observed, root.digest(), source.descriptor.id)?;
    if owner.binding != source || owner.phase != PinPhase::Published {
        return Err(error("startup Published Version differs"));
    }
    let binding = bootstrap_reader::binding(root, manifest, predecessor, *identity)?;
    if existing.is_none() {
        state.active.push(Grant {
            predecessor,
            image: digest,
            pending: Some(intent.pending_id()),
            authorized: false,
        });
    }
    // Re-sync on retries too: a previous rename/directory-sync outcome may be
    // unknown. Share/Publish can leave only this exact durable prospective slot.
    save(directory, *identity, &state)?;
    publish_grant(
        root,
        &ledger,
        intent.binding(OwnerKind::Pending)?.token(),
        *identity,
        &binding,
    )?;
    state
        .active
        .iter_mut()
        .find(|g| g.image == digest)
        .unwrap()
        .authorized = true;
    save(directory, *identity, &state)
}

/// Bootstrap-only internal ledger client. It has no local backend/engine and
/// never depends on this node's Raft driver, listener, or serving registration.
/// Only connection failure or a typed NotLeader refusal permits another seed.
/// Unknown writes return immediately; the next startup retries the same slot.
pub(crate) struct StartupLedger {
    root: RootDigest,
    node: NodeId,
    token: String,
    seeds: Vec<std::net::SocketAddr>,
    runtime: tokio::runtime::Handle,
    deadline: Instant,
    #[cfg(test)]
    channels: std::collections::BTreeMap<std::net::SocketAddr, tonic::transport::Channel>,
}
impl StartupLedger {
    pub(crate) fn new(
        root: &RootDescriptor,
        identity: StoreIdentity,
        token: String,
        runtime: tokio::runtime::Handle,
    ) -> Result<Self> {
        identity.verify(root, identity.node_id)?;
        if token.is_empty() {
            return Err(error("startup cluster credential absent"));
        }
        Ok(Self {
            root: root.digest(),
            node: identity.node_id,
            token,
            seeds: root
                .voters
                .iter()
                .filter(|v| v.node_id != identity.node_id)
                .map(|v| v.addr)
                .collect(),
            runtime,
            deadline: Instant::now() + Duration::from_secs(10),
            #[cfg(test)]
            channels: Default::default(),
        })
    }
    #[cfg(test)]
    pub(crate) fn with_channels(
        mut self,
        channels: impl IntoIterator<Item = (std::net::SocketAddr, tonic::transport::Channel)>,
    ) -> Self {
        self.channels.extend(channels);
        self
    }
    fn remaining(&self) -> Result<Duration> {
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| error("startup enrollment deadline expired"))
    }
    async fn channel(&self, address: std::net::SocketAddr) -> Result<tonic::transport::Channel> {
        #[cfg(test)]
        if let Some(channel) = self.channels.get(&address) {
            return Ok(channel.clone());
        }
        let budget = self.remaining()?.min(Duration::from_millis(250));
        let endpoint = tonic::transport::Endpoint::from_shared(format!("http://{address}"))
            .map_err(error)?
            .connect_timeout(budget);
        tokio::time::timeout(budget, endpoint.connect())
            .await
            .map_err(error)?
            .map_err(error)
    }
    fn call(&self, query: LedgerQuery) -> Result<LedgerReply> {
        query.validate(self.root)?;
        self.remaining()?;
        self.runtime.block_on(async {
            for address in &self.seeds {
                self.remaining()?;
                let channel = match self.channel(*address).await {
                    Ok(c) => c,
                    Err(_) => continue, // No request was submitted.
                };
                match wire::on_channel(
                    channel,
                    self.root,
                    &query,
                    self.node,
                    Some(self.token.clone()),
                    self.remaining()?,
                )
                .await
                {
                    Err(Error::NotLeader { .. }) => continue,
                    result => return result,
                }
            }
            Err(Error::MetaNotReady(
                "metadata bootstrap enrollment needs a reachable serving quorum".into(),
            ))
        })
    }
}
impl CheckpointLedger for StartupLedger {
    fn apply(&self, bytes: Vec<u8>) -> Result<()> {
        kv9_meta::retention::decode_request(&bytes, self.root)?;
        match self.call(LedgerQuery::Apply(bytes))? {
            LedgerReply::Applied { .. } => Ok(()),
            _ => Err(error("startup apply receipt differs")),
        }
    }
    fn owner(&self, root: RootDigest, owner: OwnerId) -> Result<Option<Vec<u8>>> {
        if root != self.root {
            return Err(error("startup ledger root differs"));
        }
        match self.call(LedgerQuery::Owner(owner))? {
            LedgerReply::Owner(bytes) => {
                if let Some(bytes) = &bytes {
                    decode_owner_observation(bytes, root, owner)?;
                }
                Ok(bytes)
            }
            _ => Err(error("startup owner receipt differs")),
        }
    }
}
