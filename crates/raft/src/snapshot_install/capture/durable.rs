//! A fixed, source-store-bound capture survives writes and process restarts.
//! The journal is retry state, never portable installation or GC authority.
use super::*;
use kv9_common::StoreIdentity;
use kv9_engine::checkpoint::{PendingTransfer, TransferJournal};
use kv9_engine::{DurableAppliedPosition, Engine, ReplicatedEngine};

const MAGIC: &[u8; 8] = b"KV9CAP01";
const STORE_BYTES: usize = 80;
const HEADER: usize = MAGIC.len() + 2 * STORE_BYTES + 16 + 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureRequest {
    pub source: StoreIdentity,
    pub destination: StoreIdentity,
    pub operation: [u8; 16],
    pub range: DataRange,
}
impl CaptureRequest {
    fn validate(&self) -> Result<()> {
        self.range.validate()?;
        StoreIdentity::decode(&self.source.canonical_bytes())?;
        StoreIdentity::decode(&self.destination.canonical_bytes())?;
        if self.source.root_digest != self.range.root
            || self.destination.root_digest != self.range.root
            || self.source.cluster_id != self.destination.cluster_id
            || self.source.node_id == self.destination.node_id
            || self.source.node_id.0 == 0
            || self.destination.node_id.0 == 0
            || self.source.cluster_id.as_bytes() == &[0; 16]
            || self.source.store_incarnation.as_bytes() == &[0; 16]
            || self.destination.store_incarnation.as_bytes() == &[0; 16]
            || self.source.store_incarnation == self.destination.store_incarnation
            || self.operation == [0; 16]
        {
            return Err(invalid("invalid durable capture binding"));
        }
        Ok(())
    }
}

/// Load the exact old plan, or durably select a new one before returning its
/// manifest. `allow_create` is false for an already installed operation.
/// Reusing a slot for ANOTHER operation requires the caller to check committed
/// terminal metadata for the old binding AND image; absence/timeouts/PUT errors
/// are never settlement. Pins are independently checked before uploading.
pub fn plan_capture_durable(
    driver: &NodeDriver<DiskRaftStorage, WalEngine>,
    engine: &WalEngine,
    request: &CaptureRequest,
    uploader: &RemoteUploader,
    allow_create: bool,
    superseded: impl FnOnce(&CaptureRequest, &CheckpointManifest, RootDigest) -> Result<bool>,
) -> Result<PlannedCapture> {
    request.validate()?;
    check_live(driver, engine)?;
    if driver.peer().node_id() != request.source.node_id
        || driver.peer().region_id() != request.range.region
        || engine.get(ColumnFamily::Default, RANGE_KEY)?.as_deref()
            != Some(request.range.encode().as_slice())
    {
        return Err(invalid("durable capture source or live range differs"));
    }
    let path = engine
        .path()
        .parent()
        .ok_or_else(|| invalid("engine directory missing"))?
        .join("capture.plan");
    let mut journal = uploader.transfer_journal(path)?;
    if let Some(pending) = journal.load()? {
        let (old, planned) = decode(pending, journal)?;
        if &old == request {
            match engine.applied_position()? {
                DurableAppliedPosition::AppliedThrough(at)
                    if at.index >= planned.cut.index
                        && at.term >= planned.cut.term
                        && (at.index != planned.cut.index || at.term == planned.cut.term) => {}
                _ => return Err(invalid("source has not recovered through the capture cut")),
            }
            return Ok(planned);
        }
        if !allow_create
            || old.operation == request.operation
            || old.source != request.source
            || old.range.root != request.range.root
            || old.range.region != request.range.region
            || old.range.creation != request.range.creation
            || !superseded(&old, planned.manifest(), planned.image_digest)?
        {
            return Err(invalid("durable slot belongs to another unsettled binding"));
        }
        let Objects::Durable {
            pending,
            journal: old_journal,
        } = planned.planned
        else {
            unreachable!()
        };
        drop(pending);
        journal = old_journal;
        // Local retry payload only. Published owners remain untouched.
        journal.clear_settled()?;
    }
    if !allow_create {
        return Err(invalid("installed operation has no reusable local capture"));
    }
    let mut planned = plan_capture_spooled(driver, engine, request.source.cluster_id, uploader)?;
    if planned.range() != &request.range {
        return Err(invalid(
            "frozen range differs from committed capture binding",
        ));
    }
    if !planned
        .configuration
        .learners
        .contains(&request.destination.node_id.0)
    {
        return Err(invalid(
            "capture cut does not contain the destination learner",
        ));
    }
    let metadata = encode(request, &planned)?;
    let Objects::Spooled(spool) = &mut planned.planned else {
        unreachable!()
    };
    journal.stage_spooled(&metadata, spool)?;
    drop(planned);
    let pending = journal
        .load()?
        .ok_or_else(|| invalid("published capture disappeared"))?;
    let (stored, planned) = decode(pending, journal)?;
    if &stored != request {
        return Err(invalid("published capture binding differs"));
    }
    Ok(planned)
}

fn encode(request: &CaptureRequest, planned: &PlannedCapture) -> Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(HEADER + planned.record.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend(request.source.canonical_bytes());
    bytes.extend(request.destination.canonical_bytes());
    bytes.extend(request.operation);
    let at = planned
        .configuration_applied_at
        .unwrap_or(AppliedPosition { term: 0, index: 0 });
    bytes.extend(at.term.to_be_bytes());
    bytes.extend(at.index.to_be_bytes());
    bytes.extend(&planned.record);
    Ok(bytes)
}

fn decode(
    pending: PendingTransfer,
    journal: TransferJournal,
) -> Result<(CaptureRequest, PlannedCapture)> {
    let (request, image, manifest) = decode_metadata(pending.metadata())?;
    if &manifest != pending.manifest() {
        return Err(invalid("durable capture manifest differs"));
    }
    Ok((
        request,
        PlannedCapture {
            planned: Objects::Durable { pending, journal },
            range: image.range,
            record: image.record,
            image_digest: image.image_digest,
            cut: image.cut,
            configuration: image.configuration,
            configuration_applied_at: image.configuration_applied_at,
        },
    ))
}

pub(super) fn decode_metadata(
    bytes: &[u8],
) -> Result<(CaptureRequest, CapturedSourceImage, CheckpointManifest)> {
    if bytes.len() <= HEADER || &bytes[..8] != MAGIC {
        return Err(invalid("invalid durable capture metadata"));
    }
    let source = StoreIdentity::decode(&bytes[8..88])?;
    let destination = StoreIdentity::decode(&bytes[88..168])?;
    let operation = bytes[168..184].try_into().unwrap();
    let at = AppliedPosition {
        term: u64::from_be_bytes(bytes[184..192].try_into().unwrap()),
        index: u64::from_be_bytes(bytes[192..200].try_into().unwrap()),
    };
    let record = bytes[HEADER..].to_vec();
    let (image, hs) = snapshot::decode(&record)?;
    let (range, manifest) = super::super::parse_image(&image)?;
    let cut = AppliedPosition {
        term: image.get_metadata().term,
        index: image.get_metadata().index,
    };
    let configuration_applied_at = if at.term == 0 && at.index == 0 {
        None
    } else {
        if at.term == 0
            || at.index == 0
            || at.term > cut.term
            || at.index > cut.index
            || (at.index == cut.index && at.term != cut.term)
        {
            return Err(invalid("durable configuration position differs from cut"));
        }
        Some(at)
    };
    let request = CaptureRequest {
        source,
        destination,
        operation,
        range: range.clone(),
    };
    request.validate()?;
    let configuration = image.get_metadata().get_conf_state().clone();
    if manifest.scope.cluster != source.cluster_id.to_string()
        || hs.term != cut.term
        || hs.vote != 0
        || !configuration.learners.contains(&destination.node_id.0)
    {
        return Err(invalid("durable capture manifest/configuration differs"));
    }
    Ok((
        request,
        CapturedSourceImage {
            objects: manifest.files.len(),
            object_bytes: manifest.files.iter().map(|f| f.size).sum(),
            range,
            image_digest: RootDigest::sha256(&record),
            record,
            cut,
            configuration,
            configuration_applied_at,
        },
        manifest,
    ))
}
