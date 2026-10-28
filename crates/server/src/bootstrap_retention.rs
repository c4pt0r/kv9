//! Durable admission and retirement state for metadata bootstrap grants. A receipt
//! becomes readable only after observing the exact Published grant through the
//! live ledger. It is store-bound, not a transferable recovery certificate.
//! Callers hold the exclusive StoreGuard through selection or the entire open.
use crate::checkpoint_retention::CheckpointLedger;
use kv9_common::retention::PinPhase;
use kv9_common::{Error, Result, RootDescriptor, RootDigest, StoreIdentity};
use kv9_engine::checkpoint::{
    CheckpointManifest, CheckpointRestore, ExecutionIdentity, RemoteUploader,
};
use kv9_engine::MemEngine;
use kv9_meta::retention::{
    bootstrap_reader, checkpoint_intent::CheckpointIntent, decode_owner_observation,
    encode_request, LedgerRequest, OwnerKind,
};
use std::path::Path;
#[cfg(test)]
use std::{fs, path::PathBuf};

pub(crate) mod startup;
mod state;
use state::{load, save, Grant, State};
#[cfg(test)]
use state::{path, MAX_BYTES};
fn error(e: impl std::fmt::Display) -> Error {
    Error::Engine(format!("metadata bootstrap reader: {e}"))
}
fn image(manifest: &CheckpointManifest) -> Result<[u8; 32]> {
    Ok(*RootDigest::sha256(&manifest.encode()?).as_bytes())
}

pub(crate) fn local_identity(
    root: &RootDescriptor,
    ledger: &dyn CheckpointLedger,
) -> Result<StoreIdentity> {
    let lifetime = ledger.local_lifetime()?;
    let identity = StoreIdentity {
        root_digest: root.digest(),
        cluster_id: root.cluster_id,
        node_id: kv9_common::NodeId(lifetime.node),
        store_incarnation: kv9_common::StoreIncarnation::from_bytes(lifetime.store),
    };
    identity.verify(root, identity.node_id)?;
    Ok(identity)
}

/// Upgrade legacy confirmed slots using their exact committed descriptors.
/// Missing/mismatching history leaves all old protection and receipts intact.
fn upgrade(
    state: &mut State,
    root: &RootDescriptor,
    history: &mut impl FnMut(u64) -> Result<CheckpointManifest>,
) -> Result<()> {
    for grant in &mut state.active {
        if grant.pending.is_none() {
            let manifest = history(grant.predecessor)?;
            if image(&manifest)? != grant.image
                || manifest.scope.region != kv9_common::META_REGION_0.0
            {
                return Err(error("legacy grant history differs"));
            }
            grant.pending =
                Some(CheckpointIntent::new(root, &manifest, grant.predecessor)?.pending_id());
        }
    }
    Ok(())
}

/// Each persisted fence is replayed verbatim. A lost release receipt keeps the
/// slot and blocks another candidate; no timeout permits readmission or reuse.
fn reconcile(
    dir: &Path,
    identity: StoreIdentity,
    state: &mut State,
    ledger: &dyn CheckpointLedger,
) -> Result<()> {
    if !state.retiring.is_empty() {
        save(dir, identity, state)?;
    }
    while let Some(grant) = state.retiring.first().copied() {
        ledger.apply(encode_request(
            identity.root_digest,
            &LedgerRequest::RetireBootstrapRead {
                pending: kv9_common::retention::OwnerToken {
                    owner: grant
                        .pending
                        .ok_or_else(|| error("retiring grant lacks pending identity"))?,
                    generation: std::num::NonZeroU64::new(1).unwrap(),
                },
                source: identity.node_id,
                store: identity.store_incarnation,
            },
        )?)?;
        state.retiring.remove(0);
        save(dir, identity, state)?;
    }
    Ok(())
}

/// Replay durable closure even when there is no newer selectable publication.
/// A fresh store without a receipt has no retirement debt or lifetime query.
pub(crate) fn resume_retirement(
    dir: &Path,
    root: &RootDescriptor,
    ledger: &dyn CheckpointLedger,
) -> Result<()> {
    match std::fs::symlink_metadata(state::path(dir)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(error(e)),
        Ok(_) => {}
    }
    let identity = local_identity(root, ledger)?;
    let mut state = load(dir, identity)?;
    reconcile(dir, identity, &mut state, ledger)
}

fn fence_other_images(state: &mut State, current: Option<[u8; 32]>) {
    let mut kept = Vec::new();
    for mut grant in state.active.drain(..) {
        if Some(grant.image) == current {
            kept.push(grant);
        } else {
            grant.authorized = false;
            state.retiring.push(grant);
        }
    }
    state.active = kept;
}

/// Prospective grant is durable BEFORE typed admission, closing the previous orphan-pin
/// window. Existing selected and candidate grants survive an interrupted switch.
/// The caller owns this store and supplies its actual committed image history.
pub(crate) fn before_selection(
    dir: &Path,
    root: &RootDescriptor,
    ledger: &dyn CheckpointLedger,
    selected: Option<&CheckpointManifest>,
    manifest: &CheckpointManifest,
    predecessor: u64,
    mut history: impl FnMut(u64) -> Result<CheckpointManifest>,
) -> Result<()> {
    let identity = local_identity(root, ledger)?;
    let mut state = load(dir, identity)?;
    upgrade(&mut state, root, &mut history)?;
    reconcile(dir, identity, &mut state, ledger)?;
    let digest = image(manifest)?;
    let existing = state.active.iter().find(|g| g.image == digest).copied();
    if existing.is_some_and(|g| g.authorized) {
        return save(dir, identity, &state);
    }
    let predecessor = existing.map_or(predecessor, |g| g.predecessor);
    let intent = CheckpointIntent::new(root, manifest, predecessor)?;
    let source = intent.binding(OwnerKind::Version)?;
    let binding = bootstrap_reader::binding(root, manifest, predecessor, identity)?;
    let observed = ledger
        .owner(root.digest(), source.descriptor.id)?
        .ok_or_else(|| error("Published Version absent"))?;
    let owner = decode_owner_observation(&observed, root.digest(), source.descriptor.id)?;
    if owner.binding != source || owner.phase != PinPhase::Published {
        return Err(error("exact Published Version unavailable"));
    }
    if let Some(grant) = existing {
        if grant.pending != Some(intent.pending_id()) {
            return Err(error("prospective grant differs"));
        }
    } else {
        fence_other_images(&mut state, selected.map(image).transpose()?);
        save(dir, identity, &state)?;
        reconcile(dir, identity, &mut state, ledger)?;
        state.active.push(Grant {
            predecessor,
            image: digest,
            pending: Some(intent.pending_id()),
            authorized: false,
        });
        save(dir, identity, &state)?;
    }
    publish_grant(
        root,
        ledger,
        intent.binding(OwnerKind::Pending)?.token(),
        identity,
        &binding,
    )?;
    state
        .active
        .iter_mut()
        .find(|g| g.image == digest)
        .unwrap()
        .authorized = true;
    save(dir, identity, &state)
}

fn publish_grant(
    root: &RootDescriptor,
    ledger: &dyn CheckpointLedger,
    pending: kv9_common::retention::OwnerToken,
    identity: StoreIdentity,
    binding: &kv9_meta::retention::OwnerBinding,
) -> Result<()> {
    ledger.apply(encode_request(
        root.digest(),
        &LedgerRequest::BeginBootstrapRead {
            pending,
            source: identity.node_id,
            store: identity.store_incarnation,
        },
    )?)?;
    ledger.apply(encode_request(
        root.digest(),
        &LedgerRequest::Publish(binding.token()),
    )?)?;
    let observed = ledger
        .owner(root.digest(), binding.descriptor.id)?
        .ok_or_else(|| error("bootstrap grant receipt absent"))?;
    let owner = decode_owner_observation(&observed, root.digest(), binding.descriptor.id)?;
    if owner.binding != *binding || owner.phase != PinPhase::Published {
        return Err(error("bootstrap grant receipt differs"));
    }
    Ok(())
}

/// Called only after the actual local selector is healthy and the complete
/// boot scope has drained. Persist read closure BEFORE requesting retirement.
pub(crate) fn after_selection(
    dir: &Path,
    root: &RootDescriptor,
    ledger: &dyn CheckpointLedger,
    selected: Option<&CheckpointManifest>,
) -> Result<()> {
    let identity = local_identity(root, ledger)?;
    let mut state = load(dir, identity)?;
    if state.active.iter().any(|g| g.pending.is_none()) {
        return Err(error("legacy grants need history upgrade"));
    }
    reconcile(dir, identity, &mut state, ledger)?;
    fence_other_images(&mut state, selected.map(image).transpose()?);
    save(dir, identity, &state)?;
    reconcile(dir, identity, &mut state, ledger)
}

/// Runtime inspector calls this only for a currently serving, fully opened
/// metadata engine at this exact store. That implies its bootstrap scope has
/// actually drained; the StoreGuard still excludes all other startup readers.
pub(crate) struct BootstrapFence {
    pub pending: kv9_common::retention::OwnerId,
    pub image: [u8; 32],
}
pub(crate) fn fenced_image(
    dir: &Path,
    identity: StoreIdentity,
    predecessor: u64,
    selected: Option<&CheckpointManifest>,
) -> Result<Option<BootstrapFence>> {
    let state = load(dir, identity)?;
    let Some(grant) = state.retiring.iter().find(|g| g.predecessor == predecessor) else {
        return Ok(None);
    };
    if grant.authorized
        || grant.pending.is_none()
        || selected.map(image).transpose()? == Some(grant.image)
    {
        return Err(error("retiring grant is still selected or readable"));
    }
    state::sync(dir)?;
    Ok(Some(BootstrapFence {
        pending: grant.pending.unwrap(),
        image: grant.image,
    }))
}

/// Local durable authorization for the actual selected metadata image. This
/// does not assert a catalog fact; the leader independently queries this store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BootstrapSelection {
    pub predecessor: u64,
    pub pending: kv9_common::retention::OwnerToken,
    pub image: [u8; 32],
}
pub(crate) fn selected_grant(
    dir: &Path,
    root: &RootDescriptor,
    identity: StoreIdentity,
    manifest: &CheckpointManifest,
) -> Result<Option<BootstrapSelection>> {
    identity.verify(root, identity.node_id)?;
    if manifest.scope.region != kv9_common::META_REGION_0.0 {
        return Err(error("bootstrap selection is only for metadata"));
    }
    let digest = image(manifest)?;
    let state = load(dir, identity)?;
    let Some(grant) = state
        .active
        .iter()
        .find(|g| g.image == digest && g.authorized)
    else {
        return Ok(None);
    };
    let intent = CheckpointIntent::new(root, manifest, grant.predecessor)?;
    if grant.pending.is_some_and(|p| p != intent.pending_id()) {
        return Err(error("bootstrap selection Pending differs"));
    }
    state::sync(dir)?;
    Ok(Some(BootstrapSelection {
        predecessor: grant.predecessor,
        image: digest,
        pending: intent.binding(OwnerKind::Pending)?.token(),
    }))
}

pub(crate) struct BootstrapReader<'a> {
    pub directory: &'a Path,
    pub root: &'a RootDescriptor,
    pub identity: StoreIdentity,
    pub uploader: &'a RemoteUploader,
}
impl CheckpointRestore for BootstrapReader<'_> {
    fn restore(&self, manifest: &CheckpointManifest) -> Result<MemEngine> {
        self.identity.verify(self.root, self.identity.node_id)?;
        let digest = image(manifest)?;
        let grant = load(self.directory, self.identity)?
            .active
            .into_iter()
            .find(|g| g.image == digest && g.authorized)
            .ok_or_else(|| error("selected image has no durable bootstrap grant"))?;
        let intent = CheckpointIntent::new(self.root, manifest, grant.predecessor)?;
        if grant.pending.is_some_and(|p| p != intent.pending_id()) {
            return Err(error("bootstrap grant pending differs"));
        }
        let binding =
            bootstrap_reader::binding(self.root, manifest, grant.predecessor, self.identity)?;
        let identity = ExecutionIdentity {
            root: self.root.digest(),
            pending: intent.pending_id(),
            executor: binding.token(),
        };
        let (result, report) = self.uploader.with_local_read_report(|reader| {
            reader.bind_execution(identity)?;
            reader.restore(manifest)
        })?;
        if report.identity() != Some(identity) {
            return Err(error("bootstrap read drain identity differs"));
        }
        // No ledger RPC on the bootstrap path. The persistent grant stays
        // Published on both success and error, including an uncertain GET.
        result
    }
}

#[cfg(test)]
mod tests;
