//! Read one exact immutable applied transition from an actual local driver.
//! No proposal/seam capability is minted or consumed, and no object is read.
use super::*;
use kv9_engine::checkpoint::CheckpointManifest;
use kv9_meta::retention::checkpoint_intent::CheckpointIntent;

pub(super) enum Settlement {
    Applied,
    WindowRefused { winner: [u8; 32], source: NodeId },
}

/// Applied is this identity's committed transition; WindowRefused is another
/// identity in its exact CAS window. Neither absence nor a newer current pair
/// supplies a negative. This does not inspect later positive effect coverage.
pub(super) fn inspect(
    backend: &RuntimeBackend,
    root: &RootDescriptor,
    intent: &CheckpointIntent,
) -> Result<Option<Settlement>> {
    if intent.root() != root.digest() {
        return Err(Error::Engine("checkpoint settlement root differs".into()));
    }
    let candidate = intent.manifest()?;
    let Some(winner) = local_history(
        backend,
        root,
        RegionId(candidate.scope.region),
        intent.predecessor(),
    )?
    else {
        return Ok(None);
    };
    Ok(Some(
        if winner.as_slice() == candidate.change_id(intent.predecessor())? {
            Settlement::Applied
        } else {
            Settlement::WindowRefused {
                winner,
                source: backend.node.id,
            }
        },
    ))
}

pub(in crate::runtime) fn local_history(
    backend: &RuntimeBackend,
    root: &RootDescriptor,
    region: RegionId,
    predecessor: u64,
) -> Result<Option<[u8; 32]>> {
    let Some(window) = predecessor.checked_add(1) else {
        return Ok(None);
    };
    // Handles are installed by this runtime's group lifecycle. Raw serving is
    // not required: an importing/sealed group may also have checkpoint history.
    // A missing replica supplies no local evidence; the caller may query an
    // admitted remote store. Never substitute another region's history.
    let handle = {
        backend
            .group_handles
            .lock()
            .expect("group handles")
            .get(&region)
            .cloned()
    };
    let driver = if region == META_REGION_0 {
        backend.driver.clone()
    } else if let Some((_, driver)) = handle {
        driver
    } else if let Some(group) = backend.raw_directory.by_region(region) {
        if group.range_binding().root != root.digest() {
            return Err(Error::Engine(
                "checkpoint settlement group root differs".into(),
            ));
        }
        group.capture_parts().0.clone()
    } else {
        return Ok(None);
    };
    let current = driver.manifest_pair(region.0)?;
    if current.generation < window {
        return Ok(None);
    }
    let Some((term, index, bytes)) = driver.manifest_at(region.0, window)? else {
        return Ok(None);
    };
    if !CheckpointManifest::is_checkpoint(&bytes) {
        return Ok(None); // Legacy opaque history is not canonical evidence.
    }
    let winner = CheckpointManifest::decode(&bytes)?;
    if winner.scope.cluster != root.cluster_id.to_string()
        || winner.scope.region != region.0
        || winner.term != term
        || winner.index != index
    {
        return Err(Error::Engine(
            "checkpoint settlement history binding differs".into(),
        ));
    }
    let winner_change = winner.change_id(predecessor)?;
    if current.generation == window && current.last_change_id != winner_change {
        return Err(Error::Engine(
            "checkpoint settlement pair and history disagree".into(),
        ));
    }
    Ok(Some(winner_change.try_into().map_err(|_| {
        Error::Engine("invalid checkpoint change ID".into())
    })?))
}
