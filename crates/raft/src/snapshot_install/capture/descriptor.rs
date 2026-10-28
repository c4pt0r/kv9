//! Canonical transfer data for a replicated catalog record. Decoding is not
//! source observation, committed metadata, object availability or GC authority.
use super::*;
use kv9_common::StoreIdentity;
use kv9_engine::{DurableAppliedPosition, Engine, ReplicatedEngine};

pub const MAX_CAPTURE_DESCRIPTOR_BYTES: usize = snapshot::MAX_PROTOCOL_SNAPSHOT_BYTES + 328;

/// Existing voters may retain the exact original image without impersonating
/// its original source. Check the actual local engine/driver and durable cut.
pub fn check_capture_replica(
    driver: &NodeDriver<DiskRaftStorage, WalEngine>,
    engine: &WalEngine,
    local: StoreIdentity,
    descriptor: &CaptureDescriptor,
) -> Result<()> {
    let request = descriptor.request();
    let conf = &descriptor.image().configuration;
    if !driver.owns_engine(engine)
        || driver.async_apply_snapshot().stopped
        || driver.status().fatal.is_some()
        || driver.peer().node_id() != local.node_id
        || driver.peer().region_id() != request.range.region
        || local.root_digest != request.range.root
        || local.cluster_id != request.source.cluster_id
        || !conf
            .voters
            .iter()
            .chain(&conf.voters_outgoing)
            .any(|n| *n == local.node_id.0)
        || engine.get(ColumnFamily::Default, RANGE_KEY)?.as_deref()
            != Some(request.range.encode().as_slice())
    {
        return Err(invalid("capture replica ownership or range differs"));
    }
    let cut = descriptor.image().cut;
    match engine.applied_position()? {
        DurableAppliedPosition::AppliedThrough(at)
            if at.index >= cut.index
                && at.term >= cut.term
                && (at.index != cut.index || at.term == cut.term) =>
        {
            Ok(())
        }
        _ => Err(invalid("capture replica has not applied through cut")),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CaptureDescriptor {
    bytes: Vec<u8>,
    request: CaptureRequest,
    manifest: CheckpointManifest,
    image: CapturedSourceImage,
}
impl CaptureDescriptor {
    /// Reuse a fully validated local replica of this exact transfer file.
    /// The runtime must independently check its current group/store ownership,
    /// committed operation and Published pins before uploading it.
    pub fn resume_replica(
        &self,
        journal: kv9_engine::checkpoint::TransferJournal,
    ) -> Result<PlannedCapture> {
        let pending = journal
            .load()?
            .ok_or_else(|| invalid("capture replica missing"))?;
        if pending.metadata() != self.encode() || pending.manifest() != self.manifest() {
            return Err(invalid("capture replica differs from committed descriptor"));
        }
        Ok(PlannedCapture {
            planned: Objects::Durable { pending, journal },
            range: self.image.range.clone(),
            record: self.image.record.clone(),
            image_digest: self.image.image_digest,
            cut: self.image.cut,
            configuration: self.image.configuration.clone(),
            configuration_applied_at: self.image.configuration_applied_at,
        })
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_CAPTURE_DESCRIPTOR_BYTES {
            return Err(invalid("capture descriptor exceeds bound"));
        }
        let (request, image, manifest) = durable::decode_metadata(bytes)?;
        Ok(Self {
            bytes: bytes.to_vec(),
            request,
            manifest,
            image,
        })
    }
    pub fn encode(&self) -> &[u8] {
        &self.bytes
    }
    pub fn digest(&self) -> RootDigest {
        RootDigest::sha256(&self.bytes)
    }
    pub fn request(&self) -> &CaptureRequest {
        &self.request
    }
    pub fn manifest(&self) -> &CheckpointManifest {
        &self.manifest
    }
    pub fn image(&self) -> &CapturedSourceImage {
        &self.image
    }

    /// Caller must independently confirm committed catalog binding and pins.
    /// Every SST is fetched and checked; this path cannot reconstruct a new cut
    /// or issue a PUT. Hold the caller's actual owner until local jobs drain.
    pub fn verify_uploaded(&self, uploader: &RemoteUploader) -> Result<CapturedSourceImage> {
        uploader.verify_checkpoint_objects(&self.manifest)?;
        Ok(self.image.clone())
    }
}

impl PlannedCapture {
    pub fn descriptor(&self) -> Result<CaptureDescriptor> {
        match &self.planned {
            Objects::Durable { pending, .. } => CaptureDescriptor::decode(pending.metadata()),
            _ => Err(invalid("capture has no durable source binding")),
        }
    }
}

/// Observe only an existing, fully validated slot of the actual healthy
/// source store/driver. Followers can confirm this immutable earlier plan;
/// observation must never generate a replacement plan or perform object I/O.
pub fn observe_durable_capture(
    driver: &NodeDriver<DiskRaftStorage, WalEngine>,
    engine: &WalEngine,
    source: StoreIdentity,
    operation: [u8; 16],
    uploader: &RemoteUploader,
) -> Result<Option<CaptureDescriptor>> {
    if !driver.owns_engine(engine)
        || driver.async_apply_snapshot().stopped
        || driver.status().fatal.is_some()
        || driver.peer().node_id() != source.node_id
    {
        return Err(invalid("capture observation source unavailable"));
    }
    let journal = uploader.transfer_journal(
        engine
            .path()
            .parent()
            .ok_or_else(|| invalid("engine directory missing"))?
            .join("capture.plan"),
    )?;
    let Some(pending) = journal.load()? else {
        return Ok(None);
    };
    let descriptor = CaptureDescriptor::decode(pending.metadata())?;
    let request = descriptor.request();
    if request.operation != operation {
        return Ok(None);
    }
    if request.source != source
        || request.range.region != driver.peer().region_id()
        || pending.manifest() != descriptor.manifest()
        || engine.get(ColumnFamily::Default, RANGE_KEY)?.as_deref()
            != Some(request.range.encode().as_slice())
    {
        return Err(invalid("capture observation binding differs"));
    }
    let cut = descriptor.image.cut;
    match engine.applied_position()? {
        DurableAppliedPosition::AppliedThrough(at)
            if at.index >= cut.index
                && at.term >= cut.term
                && (at.index != cut.index || at.term == cut.term) => {}
        _ => return Err(invalid("capture observation source precedes cut")),
    }
    Ok(Some(descriptor))
}
