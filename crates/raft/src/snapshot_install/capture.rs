//! Unified-cut source capture for one live data group, under its driver owner.
//!
//! Capture produces the canonical offline image the joint installer consumes:
//! one exact engine cut, the checkpoint manifest planned from that frozen
//! view, and the committed Raft configuration AT-OR-BEFORE that cut from
//! durable history. The engine's applied position may lag no-op and
//! configuration entries; a configuration committed past the cut is a typed
//! refusal here, never silently attached to an older image.
//!
//! Planning is I/O-free: the manifest — and therefore the owner subject and
//! complete object closure — is fully known before any upload. The caller
//! must commit its retention owners for exactly that manifest between
//! planning and uploading. A capture receipt grants no serving, voting,
//! installation or pin-release capability, and nothing here mutates the
//! source group.

use kv9_common::data_range::{DataRange, RANGE_KEY};
use kv9_common::{AppliedPosition, ClusterId, Error, Result, RootDigest};
use kv9_engine::checkpoint::{CheckpointManifest, PlannedFlush, RemoteUploader};
use kv9_engine::{ColumnFamily, WalEngine};
use raft::prelude::{ConfState, HardState, Snapshot};

use crate::driver::NodeDriver;
use crate::storage::{snapshot, ConfigurationLookup, DiskRaftStorage};

fn invalid(message: &str) -> Error {
    Error::Raft(format!("source capture: {message}"))
}

/// One planned, not-yet-uploaded capture. The manifest and the encoded
/// protocol record are final; uploading changes nothing but object presence.
pub struct PlannedCapture {
    // Debug intentionally omits object payloads; see the manual impl below.
    planned: PlannedFlush,
    range: DataRange,
    record: Vec<u8>,
    image_digest: RootDigest,
    cut: AppliedPosition,
    configuration: ConfState,
    configuration_applied_at: Option<AppliedPosition>,
}

/// Diagnostics plus the canonical record. No storage handle is returned.
#[derive(Debug, Clone, PartialEq)]
pub struct CapturedSourceImage {
    pub record: Vec<u8>,
    pub image_digest: RootDigest,
    pub range: DataRange,
    pub cut: AppliedPosition,
    pub configuration: ConfState,
    /// Where the attached configuration was committed; `None` for the
    /// group's initial configuration.
    pub configuration_applied_at: Option<AppliedPosition>,
    pub object_bytes: u64,
    pub objects: usize,
}

impl std::fmt::Debug for PlannedCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlannedCapture")
            .field("cut", &self.cut)
            .field("image_digest", &self.image_digest)
            .field("range", &self.range)
            .finish_non_exhaustive()
    }
}

impl PlannedCapture {
    pub fn manifest(&self) -> &CheckpointManifest {
        self.planned.manifest()
    }
    pub fn range(&self) -> &DataRange {
        &self.range
    }
    pub fn cut(&self) -> AppliedPosition {
        self.cut
    }

    /// Upload exactly the planned closure (PUT plus verified GET per object)
    /// and return the receipt. Owners for `manifest()` must already be
    /// committed; this function performs no retention operation itself.
    pub fn upload(self, uploader: &RemoteUploader) -> Result<CapturedSourceImage> {
        let manifest = self.planned.manifest().clone();
        uploader.upload_planned(self.planned)?;
        Ok(CapturedSourceImage {
            record: self.record,
            image_digest: self.image_digest,
            range: self.range,
            cut: self.cut,
            configuration: self.configuration,
            configuration_applied_at: self.configuration_applied_at,
            object_bytes: manifest.files.iter().map(|f| f.size).sum(),
            objects: manifest.files.len(),
        })
    }
}

/// Plan one capture of a live data group at its current durable applied cut.
///
/// Leader-only under the driver owner: a follower or fatal driver refuses.
/// The frozen view supplies both the data and the authoritative `DataRange`
/// (and therefore the manifest scope) at the same instant; the configuration
/// comes from durable history at-or-before the same cut or the capture
/// refuses with the storage's typed reason.
pub fn plan_capture(
    driver: &NodeDriver<DiskRaftStorage, WalEngine>,
    engine: &WalEngine,
    cluster: ClusterId,
) -> Result<PlannedCapture> {
    let status = driver.status();
    if status.fatal.is_some() {
        return Err(invalid("driver is fatally stopped"));
    }
    if status.role != crate::Role::Leader {
        return Err(Error::NotLeader {
            leader: status.leader_id,
        });
    }
    let mut sealed_range: Option<DataRange> = None;
    let frozen = engine.freeze_with_scope(|view| {
        let bytes = view
            .get(ColumnFamily::Default, RANGE_KEY)?
            .ok_or_else(|| invalid("group has no applied range ownership"))?;
        let range = DataRange::decode(&bytes)?;
        range.validate()?;
        let scope = kv9_engine::checkpoint::FlushScope {
            cluster: cluster.to_string(),
            region: range.region.0,
            conf_ver: range.conf_ver,
            version: range.version,
        };
        sealed_range = Some(range);
        Ok(scope)
    })?;
    let cut = frozen.position();
    let range = sealed_range.expect("scope callback ran");
    let committed = match driver.peer().configuration_at_committed(cut)? {
        ConfigurationLookup::Found(committed) => committed,
        ConfigurationLookup::Unavailable(reason) => {
            return Err(invalid(&format!(
                "no defensible configuration at the cut: {reason:?}"
            )))
        }
    };
    let configuration = committed.state().clone();
    let configuration_applied_at = committed.applied_at();
    let planned = RemoteUploader::plan(frozen)?;
    let manifest = planned.manifest();
    if manifest.index != cut.index || manifest.term != cut.term {
        return Err(invalid("planned manifest cut differs from the frozen cut"));
    }
    let data = super::data_image(&range, manifest)?;
    let mut image = Snapshot::default();
    image.set_data(data.into());
    let metadata = image.mut_metadata();
    metadata.index = cut.index;
    metadata.term = cut.term;
    metadata.set_conf_state(configuration.clone());
    // The record claims the cut and the configuration, nothing about
    // elections: term equals the entry term at the cut and the vote is
    // empty. The installer's transition rules still apply unchanged.
    let hs = HardState {
        term: cut.term,
        vote: 0,
        commit: cut.index,
        ..Default::default()
    };
    let record = snapshot::encode(&image, &hs)?;
    Ok(PlannedCapture {
        planned,
        range,
        image_digest: RootDigest::sha256(&record),
        record,
        cut,
        configuration,
        configuration_applied_at,
    })
}

#[cfg(test)]
mod tests;
