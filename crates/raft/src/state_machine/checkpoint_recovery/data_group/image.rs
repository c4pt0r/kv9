//! Image inspection validates contents against the group's bound ownership.
use super::*;

/// Validated image descriptor, not publication or serving authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataCheckpointState {
    Bound(DataRange),
}
impl DataCheckpointState {
    /// Scope descriptor only, never a binding by itself.
    pub fn target(&self) -> &DataRange {
        match self {
            Self::Bound(range) => range,
        }
    }
    pub fn bound_range(&self) -> Option<&DataRange> {
        match self {
            Self::Bound(range) => Some(range),
        }
    }
}

pub struct InspectedCheckpoint {
    state: DataCheckpointState,
    watermark: Option<ClockWatermark>,
    raw_values: u64,
}
impl InspectedCheckpoint {
    pub fn state(&self) -> &DataCheckpointState {
        &self.state
    }
    pub fn watermark(&self) -> Option<ClockWatermark> {
        self.watermark
    }
    pub fn into_state(self) -> DataCheckpointState {
        self.state
    }
}

fn descriptor(
    view: &dyn ReadView,
    identity: DataCheckpointIdentity,
) -> Result<(DataCheckpointState, Option<ClockWatermark>)> {
    identity.validate()?;
    let range = view
        .get(ColumnFamily::Default, RANGE_KEY)?
        .map(|b| DataRange::decode(&b))
        .transpose()?;
    let clock = view
        .get(ColumnFamily::Default, CLOCK_KEY)?
        .map(|b| ClockWatermark::decode(&b))
        .transpose()?;
    validate_local(
        (identity.root, identity.region, identity.creation),
        range.as_ref(),
        clock,
    )?;
    let Some(range) = range else {
        return Err(invalid("image has no bound ownership"));
    };
    let state = DataCheckpointState::Bound(range);
    identity.check_range(state.target())?;
    Ok((state, clock))
}

/// Descriptor validation from one actual snapshot. Counts/hash are verified by
/// inspect_checkpoint after restoring the complete verified SST closure.
pub fn checkpoint_state(
    view: &dyn ReadView,
    identity: DataCheckpointIdentity,
) -> Result<(DataCheckpointState, kv9_engine::checkpoint::FlushScope, u64)> {
    let (state, _) = descriptor(view, identity)?;
    let range = state.target();
    let generation = view
        .get(ColumnFamily::Default, &manifest_pair_key(range.region.0))?
        .map(|b| ManifestPair::decode(&b))
        .transpose()?
        .map_or(0, |p| p.generation);
    let scope = kv9_engine::checkpoint::FlushScope {
        cluster: identity.cluster.to_string(),
        region: range.region.0,
        conf_ver: range.conf_ver,
        version: range.version,
    };
    Ok((state, scope, generation))
}

/// Scope and predecessor from the same actual frozen image. This is descriptor
/// validation only, not an upload or publication capability.
pub fn checkpoint_scope(
    view: &dyn ReadView,
    identity: DataCheckpointIdentity,
) -> Result<(DataRange, kv9_engine::checkpoint::FlushScope, u64)> {
    let (state, scope, generation) = checkpoint_state(view, identity)?;
    let DataCheckpointState::Bound(range) = state;
    Ok((range, scope, generation))
}

/// Validate a verified restored SST view before publication/adoption or recovery.
/// The caller must first verify all manifest SSTs (e.g. RemoteUploader::restore). Counts and nonoverlap were
/// already validated by restore; inspect every CF and retain all expiry bytes,
/// including expired versions and source-group identities. Cross-group write
/// indices are not comparable with this group's image cut.
pub fn inspect_base(
    view: &dyn ReadView,
    manifest: &CheckpointManifest,
    identity: DataCheckpointIdentity,
) -> Result<(DataRange, Option<ClockWatermark>)> {
    let image = inspect_image(view, manifest, identity)?;
    Ok((image.range, image.watermark))
}

/// Shared image inspection for local checkpoint recovery and destination
/// installation. It validates contents, never their transfer/publication authority.
pub(crate) struct InspectedDataImage {
    pub range: DataRange,
    pub watermark: Option<ClockWatermark>,
    pub raw_values: u64,
}

pub(crate) fn inspect_image(
    view: &dyn ReadView,
    manifest: &CheckpointManifest,
    identity: DataCheckpointIdentity,
) -> Result<InspectedDataImage> {
    let inspected = inspect_checkpoint(view, manifest, identity)?;
    let DataCheckpointState::Bound(range) = inspected.state;
    Ok(InspectedDataImage {
        range,
        watermark: inspected.watermark,
        raw_values: inspected.raw_values,
    })
}

/// Inspect an entire image after RemoteUploader::restore has verified the
/// manifest's complete SST closure and counts; this is not serving/install authority.
pub fn inspect_checkpoint(
    view: &dyn ReadView,
    manifest: &CheckpointManifest,
    identity: DataCheckpointIdentity,
) -> Result<InspectedCheckpoint> {
    let (state, watermark) = descriptor(view, identity)?;
    let range = state.target();
    if manifest.scope.cluster != identity.cluster.to_string()
        || manifest.scope.region != range.region.0
        || manifest.scope.conf_ver != range.conf_ver
        || manifest.scope.version != range.version
    {
        return Err(invalid("manifest scope differs from its actual image"));
    }
    let pair_key = manifest_pair_key(range.region.0);
    let pair = view
        .get(ColumnFamily::Default, &pair_key)?
        .map(|v| ManifestPair::decode(&v))
        .transpose()?;
    let mut prefix = manifest_gen_key(range.region.0, 0);
    prefix.truncate(prefix.len() - 8);
    let mut history_count = 0_u64;
    let mut last_change = None;
    let mut raw_values = 0_u64;

    for file in &manifest.files {
        let cf = ColumnFamily::ALL[file.cf as usize];
        let mut end = file.largest.clone();
        end.push(0);
        let mut count = 0;
        for entry in view.iter(cf, &file.smallest, &end)? {
            let (encoded, bytes) = entry?;
            count += 1;
            if cf == ColumnFamily::Default {
                if encoded == RANGE_KEY || encoded == CLOCK_KEY || encoded == pair_key {
                    continue;
                }
                if encoded == COMPACTION_CONFIRMED_KEY {
                    if bytes.len() != 16 {
                        return Err(invalid("malformed compaction confirmation"));
                    }
                    let at = AppliedPosition {
                        term: u64::from_be_bytes(bytes[..8].try_into().unwrap()),
                        index: u64::from_be_bytes(bytes[8..].try_into().unwrap()),
                    };
                    if at.term == 0
                        || at.index == 0
                        || at.term > manifest.term
                        || at.index > manifest.index
                    {
                        return Err(invalid("compaction confirmation exceeds image cut"));
                    }
                    continue;
                }
                if encoded.starts_with(&prefix) && encoded.len() == prefix.len() + 8 {
                    let generation =
                        u64::from_be_bytes(encoded[prefix.len()..].try_into().unwrap());
                    if generation != history_count + 1 || bytes.len() < 16 {
                        return Err(invalid("manifest history is incomplete"));
                    }
                    let old = CheckpointManifest::decode(&bytes[16..])?;
                    if old.scope.cluster != manifest.scope.cluster
                        || old.scope.region != range.region.0
                        || old.scope.conf_ver > range.conf_ver
                        || old.scope.version > range.version
                        || old.term > manifest.term
                        || old.index > manifest.index
                        || old.term.to_be_bytes() != bytes[..8]
                        || old.index.to_be_bytes() != bytes[8..16]
                    {
                        return Err(invalid("historical manifest is foreign or ahead of image"));
                    }
                    last_change = Some(old.change_id(generation - 1)?);
                    history_count = generation;
                    continue;
                }
            }
            let key = decode_key(&encoded)?;
            if key.mode != KeyMode::Raw
                || key.keyspace != range.keyspace
                || !range.contains(key.user_key)
            {
                return Err(invalid("image contains foreign user keys"));
            }
            match cf {
                ColumnFamily::Default => raw_values += 1,
                ColumnFamily::Write if watermark.is_some() => {
                    ExpiryRecord::decode(&bytes)?;
                    if view.get(ColumnFamily::Default, &encoded)?.is_none() {
                        return Err(invalid("expiry record has no corresponding value"));
                    }
                }
                _ => return Err(invalid("image contains unsupported column-family data")),
            }
        }
        if count != file.count {
            return Err(invalid("image record count differs from verified SST"));
        }
    }
    match (pair, last_change) {
        (None, None) if history_count == 0 => {}
        (Some(pair), Some(change))
            if pair.generation == history_count && pair.last_change_id == change => {}
        _ => return Err(invalid("manifest pair differs from retained image history")),
    }
    Ok(InspectedCheckpoint {
        state,
        watermark,
        raw_values,
    })
}
