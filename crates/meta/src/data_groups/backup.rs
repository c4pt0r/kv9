//! Committed backups of a keyspace (docs/BACKUP-RESTORE.md §1): four immutable
//! row kinds that together make a retention point.
//!
//! - **119, the intent** (`KV9BKP01`): one keyspace, its bound ranges frozen as
//!   `(region, creation task, bind task, epoch)` at the moment of the intent, and
//!   one derived capture operation per range. The intent moves no data; it is
//!   what each range leader captures for and what the metadata leader bundles.
//! - **120, an anchor** (`KV9BKA01`): one range's exact cut, its image and manifest
//!   digests, its object closure's size, the rows and their chained digest, and
//!   the ledger owner that pins it. One per range, committed by the range leader
//!   from its own receipt through the shared evidence path.
//! - **122, the completion** (`KV9BKC01`): the bundle object the metadata leader
//!   wrote once every range was anchored and verified. A backup is `complete`
//!   when this row exists and nothing else records it.
//! - **121, the retirement** (`KV9BKR01`): the only release of the pins; the
//!   bundle stays as a tombstone.
//!
//! Identities derive from committed facts: the backup operation from
//! `(root, keyspace, sequence)`, a range's capture operation from the backup
//! operation and the region, the synthetic capture destination from the range
//! operation. A resubmission of `backup create` confirms the keyspace's live
//! backup; once that one is complete, aborted or retired, the next sequence is
//! free. Nothing here uploads, pins or deletes an object.

use super::*;
use kv9_common::{AppliedPosition, KeyspaceId, TenantId};

const BACKUP_INTENT: u64 = 119;
const BACKUP_ANCHOR: u64 = 120;
const BACKUP_RETIREMENT: u64 = 121;
const BACKUP_COMPLETION: u64 = 122;
const BKP_MAGIC: &[u8; 8] = b"KV9BKP01";
const BKA_MAGIC: &[u8; 8] = b"KV9BKA01";
const BKR_MAGIC: &[u8; 8] = b"KV9BKR01";
const BKC_MAGIC: &[u8; 8] = b"KV9BKC01";

/// A keyspace of more ranges than this is refused by `backup create` in this
/// increment (the intent row stays a few kilobytes).
pub const MAX_BACKUP_RANGES: usize = 64;
pub const MAX_LABEL_BYTES: usize = 64;
pub const MAX_BUNDLE_KEY_BYTES: usize = 256;
const INTENT_HEADER: usize = 8 + 32 + 16 + 8 + 4 + 8 + 8 + 8 + 2 + 2;
const RANGE_BYTES: usize = 8 + 8 + 8 + 8 + 8 + 16;
pub const MAX_BACKUP_INTENT_BYTES: usize = INTENT_HEADER + MAX_LABEL_BYTES + MAX_BACKUP_RANGES * RANGE_BYTES;
pub const ANCHOR_BYTES: usize = 8 + 32 + 16 + 8 + 16 + 8 + 16 + 1 + 16 + 32 + 32 + 8 + 8 + 8 + 32 + 1 + 16;
pub const RETIREMENT_BYTES: usize = 8 + 32 + 16 + 8 + 8;
const COMPLETION_HEADER: usize = 8 + 32 + 16 + 8 + 32 + 8 + 2 + 8 + 2;
pub const MAX_COMPLETION_BYTES: usize = COMPLETION_HEADER + MAX_BUNDLE_KEY_BYTES;

/// The node id of the synthetic capture destination every backup range names: a
/// capture request needs a destination distinct from its source, and a backup
/// has none. No node may be admitted under this id.
pub use kv9_common::BACKUP_DESTINATION_NODE;

fn sha16(parts: &[&[u8]]) -> [u8; 16] {
    let mut bytes = Vec::new();
    for part in parts {
        bytes.extend_from_slice(part);
    }
    let digest = RootDigest::sha256(&bytes);
    let mut out = [0u8; 16];
    out.copy_from_slice(&digest.as_bytes()[..16]);
    out
}

/// `sha16("kv9-backup/v1" ++ root ++ keyspace ++ sequence)`.
pub fn backup_operation(root: RootDigest, keyspace: KeyspaceId, sequence: u64) -> [u8; 16] {
    sha16(&[b"kv9-backup/v1", root.as_bytes(), &(keyspace.0 as u64).to_be_bytes(), &sequence.to_be_bytes()])
}

/// `sha16("kv9-backup-range/v1" ++ backup operation ++ region)`: the capture
/// operation of one range of one backup, the identity the capture catalog, the
/// ledger owners and the anchor all share.
pub fn range_operation(backup: [u8; 16], region: RegionId) -> [u8; 16] {
    sha16(&[b"kv9-backup-range/v1", &backup, &region.0.to_be_bytes()])
}

/// The synthetic destination a backup capture names (see [`BACKUP_DESTINATION_NODE`]).
pub fn backup_destination(range_operation: [u8; 16]) -> InitialReplica {
    InitialReplica {
        node: BACKUP_DESTINATION_NODE,
        incarnation: StoreIncarnation::from_bytes(sha16(&[b"kv9-backup-destination/v1", &range_operation])),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupRange {
    region: RegionId,
    creation_task: u64,
    bind_task: u64,
    conf_ver: u64,
    version: u64,
    operation: [u8; 16],
}

impl BackupRange {
    pub fn region(&self) -> RegionId {
        self.region
    }
    pub fn creation_task(&self) -> u64 {
        self.creation_task
    }
    pub fn bind_task(&self) -> u64 {
        self.bind_task
    }
    pub fn conf_ver(&self) -> u64 {
        self.conf_ver
    }
    pub fn version(&self) -> u64 {
        self.version
    }
    /// The capture operation of this range (see [`range_operation`]).
    pub fn operation(&self) -> [u8; 16] {
        self.operation
    }
    pub fn destination(&self) -> InitialReplica {
        backup_destination(self.operation)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupIntent {
    root: RootDigest,
    operation: [u8; 16],
    task: u64,
    keyspace: KeyspaceId,
    tenant: TenantId,
    sequence: u64,
    requested_unix: u64,
    label: String,
    ranges: Vec<BackupRange>,
}

impl BackupIntent {
    pub fn root(&self) -> RootDigest {
        self.root
    }
    pub fn operation(&self) -> [u8; 16] {
        self.operation
    }
    pub fn task(&self) -> u64 {
        self.task
    }
    pub fn keyspace(&self) -> KeyspaceId {
        self.keyspace
    }
    pub fn tenant(&self) -> TenantId {
        self.tenant
    }
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    pub fn requested_unix(&self) -> u64 {
        self.requested_unix
    }
    pub fn label(&self) -> &str {
        &self.label
    }
    pub fn ranges(&self) -> &[BackupRange] {
        &self.ranges
    }
    pub fn range_by_operation(&self, operation: [u8; 16]) -> Option<&BackupRange> {
        self.ranges.iter().find(|r| r.operation == operation)
    }
    pub fn range_of_region(&self, region: RegionId) -> Option<&BackupRange> {
        self.ranges.iter().find(|r| r.region == region)
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(MAX_BACKUP_INTENT_BYTES);
        bytes.extend_from_slice(BKP_MAGIC);
        bytes.extend_from_slice(self.root.as_bytes());
        bytes.extend_from_slice(&self.operation);
        bytes.extend_from_slice(&self.task.to_be_bytes());
        bytes.extend_from_slice(&self.keyspace.0.to_be_bytes());
        bytes.extend_from_slice(&self.tenant.0.to_be_bytes());
        bytes.extend_from_slice(&self.sequence.to_be_bytes());
        bytes.extend_from_slice(&self.requested_unix.to_be_bytes());
        bytes.extend_from_slice(&(self.label.len() as u16).to_be_bytes());
        bytes.extend_from_slice(&(self.ranges.len() as u16).to_be_bytes());
        bytes.extend_from_slice(self.label.as_bytes());
        for range in &self.ranges {
            bytes.extend_from_slice(&range.region.0.to_be_bytes());
            bytes.extend_from_slice(&range.creation_task.to_be_bytes());
            bytes.extend_from_slice(&range.bind_task.to_be_bytes());
            bytes.extend_from_slice(&range.conf_ver.to_be_bytes());
            bytes.extend_from_slice(&range.version.to_be_bytes());
            bytes.extend_from_slice(&range.operation);
        }
        bytes
    }

    /// Decode data only; this mints no committed authority.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < INTENT_HEADER || bytes.len() > MAX_BACKUP_INTENT_BYTES || &bytes[..8] != BKP_MAGIC {
            return Err(invalid("invalid backup intent encoding"));
        }
        let label_len = usize::from(u16::from_be_bytes(bytes[92..94].try_into().unwrap()));
        let count = usize::from(u16::from_be_bytes(bytes[94..96].try_into().unwrap()));
        if label_len > MAX_LABEL_BYTES || count == 0 || count > MAX_BACKUP_RANGES || bytes.len() != INTENT_HEADER + label_len + count * RANGE_BYTES {
            return Err(invalid("invalid backup intent shape"));
        }
        let label = std::str::from_utf8(&bytes[96..96 + label_len])
            .map_err(|_| invalid("backup label is not UTF-8"))?
            .to_owned();
        let intent = Self {
            root: RootDigest::from_bytes(bytes[8..40].try_into().unwrap()),
            operation: bytes[40..56].try_into().unwrap(),
            task: u64::from_be_bytes(bytes[56..64].try_into().unwrap()),
            keyspace: KeyspaceId(u32::from_be_bytes(bytes[64..68].try_into().unwrap())),
            tenant: TenantId(u64::from_be_bytes(bytes[68..76].try_into().unwrap())),
            sequence: u64::from_be_bytes(bytes[76..84].try_into().unwrap()),
            requested_unix: u64::from_be_bytes(bytes[84..92].try_into().unwrap()),
            label,
            ranges: bytes[96 + label_len..]
                .chunks_exact(RANGE_BYTES)
                .map(|c| BackupRange {
                    region: RegionId(u64::from_be_bytes(c[..8].try_into().unwrap())),
                    creation_task: u64::from_be_bytes(c[8..16].try_into().unwrap()),
                    bind_task: u64::from_be_bytes(c[16..24].try_into().unwrap()),
                    conf_ver: u64::from_be_bytes(c[24..32].try_into().unwrap()),
                    version: u64::from_be_bytes(c[32..40].try_into().unwrap()),
                    operation: c[40..56].try_into().unwrap(),
                })
                .collect(),
        };
        if intent.root.as_bytes() == &[0; 32]
            || intent.task < FIRST_DYNAMIC_ID
            || intent.operation != backup_operation(intent.root, intent.keyspace, intent.sequence)
            || intent.ranges.iter().any(|r| {
                r.creation_task < FIRST_DYNAMIC_ID
                    || r.bind_task < FIRST_DYNAMIC_ID
                    || r.operation != range_operation(intent.operation, r.region)
            })
            || (1..intent.ranges.len()).any(|i| intent.ranges[..i].iter().any(|p| p.region == intent.ranges[i].region))
        {
            return Err(invalid("invalid backup intent identities"));
        }
        Ok(intent)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupAnchor {
    root: RootDigest,
    /// The backup operation (kind 119).
    operation: [u8; 16],
    task: u64,
    range_operation: [u8; 16],
    region: RegionId,
    cut: AppliedPosition,
    configuration_at: Option<AppliedPosition>,
    image_digest: RootDigest,
    manifest_digest: RootDigest,
    objects: u64,
    object_bytes: u64,
    rows: u64,
    rows_digest: RootDigest,
    /// The rows and their digest were computed from the image (it fit the image
    /// budget); a restore can verify against them.
    restorable: bool,
    owner: [u8; 16],
}

impl BackupAnchor {
    pub fn root(&self) -> RootDigest {
        self.root
    }
    pub fn operation(&self) -> [u8; 16] {
        self.operation
    }
    pub fn task(&self) -> u64 {
        self.task
    }
    pub fn range_operation(&self) -> [u8; 16] {
        self.range_operation
    }
    pub fn region(&self) -> RegionId {
        self.region
    }
    pub fn cut(&self) -> AppliedPosition {
        self.cut
    }
    pub fn configuration_at(&self) -> Option<AppliedPosition> {
        self.configuration_at
    }
    pub fn image_digest(&self) -> RootDigest {
        self.image_digest
    }
    pub fn manifest_digest(&self) -> RootDigest {
        self.manifest_digest
    }
    pub fn objects(&self) -> u64 {
        self.objects
    }
    pub fn object_bytes(&self) -> u64 {
        self.object_bytes
    }
    pub fn rows(&self) -> u64 {
        self.rows
    }
    pub fn rows_digest(&self) -> RootDigest {
        self.rows_digest
    }
    pub fn restorable(&self) -> bool {
        self.restorable
    }
    pub fn owner(&self) -> [u8; 16] {
        self.owner
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(ANCHOR_BYTES);
        bytes.extend_from_slice(BKA_MAGIC);
        bytes.extend_from_slice(self.root.as_bytes());
        bytes.extend_from_slice(&self.operation);
        bytes.extend_from_slice(&self.task.to_be_bytes());
        bytes.extend_from_slice(&self.range_operation);
        bytes.extend_from_slice(&self.region.0.to_be_bytes());
        bytes.extend_from_slice(&self.cut.term.to_be_bytes());
        bytes.extend_from_slice(&self.cut.index.to_be_bytes());
        bytes.push(u8::from(self.configuration_at.is_some()));
        let at = self.configuration_at.unwrap_or(AppliedPosition { term: 0, index: 0 });
        bytes.extend_from_slice(&at.term.to_be_bytes());
        bytes.extend_from_slice(&at.index.to_be_bytes());
        bytes.extend_from_slice(self.image_digest.as_bytes());
        bytes.extend_from_slice(self.manifest_digest.as_bytes());
        bytes.extend_from_slice(&self.objects.to_be_bytes());
        bytes.extend_from_slice(&self.object_bytes.to_be_bytes());
        bytes.extend_from_slice(&self.rows.to_be_bytes());
        bytes.extend_from_slice(self.rows_digest.as_bytes());
        bytes.push(u8::from(self.restorable));
        bytes.extend_from_slice(&self.owner);
        bytes
    }

    fn decode_inner(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != ANCHOR_BYTES || &bytes[..8] != BKA_MAGIC {
            return Err(invalid("invalid backup anchor encoding"));
        }
        let u = |at: usize| u64::from_be_bytes(bytes[at..at + 8].try_into().unwrap());
        let has_configuration = match bytes[104] {
            0 => false,
            1 => true,
            _ => return Err(invalid("invalid backup anchor configuration flag")),
        };
        let restorable = match bytes[241] {
            0 => false,
            1 => true,
            _ => return Err(invalid("invalid backup anchor restorable flag")),
        };
        let anchor = Self {
            root: RootDigest::from_bytes(bytes[8..40].try_into().unwrap()),
            operation: bytes[40..56].try_into().unwrap(),
            task: u(56),
            range_operation: bytes[64..80].try_into().unwrap(),
            region: RegionId(u(80)),
            cut: AppliedPosition { term: u(88), index: u(96) },
            configuration_at: has_configuration.then(|| AppliedPosition { term: u(105), index: u(113) }),
            image_digest: RootDigest::from_bytes(bytes[121..153].try_into().unwrap()),
            manifest_digest: RootDigest::from_bytes(bytes[153..185].try_into().unwrap()),
            objects: u(185),
            object_bytes: u(193),
            rows: u(201),
            rows_digest: RootDigest::from_bytes(bytes[209..241].try_into().unwrap()),
            restorable,
            owner: bytes[242..258].try_into().unwrap(),
        };
        if anchor.root.as_bytes() == &[0; 32]
            || anchor.operation == [0; 16]
            || anchor.range_operation != range_operation(anchor.operation, anchor.region)
            || anchor.cut.index == 0
            || anchor.image_digest.as_bytes() == &[0; 32]
            || anchor.manifest_digest.as_bytes() == &[0; 32]
            || anchor.owner == [0; 16]
            || (!has_configuration && (u(105) != 0 || u(113) != 0))
            || (!anchor.restorable && (anchor.rows != 0 || anchor.rows_digest.as_bytes() != &[0; 32]))
        {
            return Err(invalid("invalid backup anchor identities"));
        }
        Ok(anchor)
    }

    /// Decode a committed row (task allocated).
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let anchor = Self::decode_inner(bytes)?;
        if anchor.task < FIRST_DYNAMIC_ID {
            return Err(invalid("invalid backup anchor task"));
        }
        Ok(anchor)
    }

    /// Decode a leader-emitted receipt: the same layout with the task still zero.
    pub fn decode_receipt(bytes: &[u8]) -> Result<Self> {
        let receipt = Self::decode_inner(bytes)?;
        if receipt.task != 0 {
            return Err(invalid("a backup anchor receipt cannot supply its own task"));
        }
        Ok(receipt)
    }

    /// Canonical receipt bytes a range leader emits after its upload and digest.
    #[allow(clippy::too_many_arguments)]
    pub fn receipt(
        intent: &BackupIntent,
        range: &BackupRange,
        cut: AppliedPosition,
        configuration_at: Option<AppliedPosition>,
        image_digest: RootDigest,
        manifest_digest: RootDigest,
        objects: u64,
        object_bytes: u64,
        rows: Option<(u64, RootDigest)>,
        owner: [u8; 16],
    ) -> Result<Vec<u8>> {
        let (restorable, rows, rows_digest) = match rows {
            Some((rows, digest)) => (true, rows, digest),
            None => (false, 0, RootDigest::from_bytes([0; 32])),
        };
        let receipt = Self {
            root: intent.root,
            operation: intent.operation,
            task: 0,
            range_operation: range.operation,
            region: range.region,
            cut,
            configuration_at,
            image_digest,
            manifest_digest,
            objects,
            object_bytes,
            rows,
            rows_digest,
            restorable,
            owner,
        };
        let bytes = receipt.encode();
        Self::decode_receipt(&bytes)?;
        Ok(bytes)
    }
}

/// Is this receipt a backup anchor (the shared evidence path dispatches on it)?
pub fn is_anchor_receipt(bytes: &[u8]) -> bool {
    bytes.len() == ANCHOR_BYTES && &bytes[..8] == BKA_MAGIC
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupCompletion {
    root: RootDigest,
    operation: [u8; 16],
    task: u64,
    bundle_digest: RootDigest,
    bundle_bytes: u64,
    ranges: u16,
    completed_unix: u64,
    bundle_key: String,
}

impl BackupCompletion {
    pub fn root(&self) -> RootDigest {
        self.root
    }
    pub fn operation(&self) -> [u8; 16] {
        self.operation
    }
    pub fn task(&self) -> u64 {
        self.task
    }
    pub fn bundle_digest(&self) -> RootDigest {
        self.bundle_digest
    }
    pub fn bundle_bytes(&self) -> u64 {
        self.bundle_bytes
    }
    pub fn ranges(&self) -> u16 {
        self.ranges
    }
    pub fn completed_unix(&self) -> u64 {
        self.completed_unix
    }
    pub fn bundle_key(&self) -> &str {
        &self.bundle_key
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(MAX_COMPLETION_BYTES);
        bytes.extend_from_slice(BKC_MAGIC);
        bytes.extend_from_slice(self.root.as_bytes());
        bytes.extend_from_slice(&self.operation);
        bytes.extend_from_slice(&self.task.to_be_bytes());
        bytes.extend_from_slice(self.bundle_digest.as_bytes());
        bytes.extend_from_slice(&self.bundle_bytes.to_be_bytes());
        bytes.extend_from_slice(&self.ranges.to_be_bytes());
        bytes.extend_from_slice(&self.completed_unix.to_be_bytes());
        bytes.extend_from_slice(&(self.bundle_key.len() as u16).to_be_bytes());
        bytes.extend_from_slice(self.bundle_key.as_bytes());
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < COMPLETION_HEADER || bytes.len() > MAX_COMPLETION_BYTES || &bytes[..8] != BKC_MAGIC {
            return Err(invalid("invalid backup completion encoding"));
        }
        let key_len = usize::from(u16::from_be_bytes(bytes[114..116].try_into().unwrap()));
        if key_len == 0 || key_len > MAX_BUNDLE_KEY_BYTES || bytes.len() != COMPLETION_HEADER + key_len {
            return Err(invalid("invalid backup completion shape"));
        }
        let completion = Self {
            root: RootDigest::from_bytes(bytes[8..40].try_into().unwrap()),
            operation: bytes[40..56].try_into().unwrap(),
            task: u64::from_be_bytes(bytes[56..64].try_into().unwrap()),
            bundle_digest: RootDigest::from_bytes(bytes[64..96].try_into().unwrap()),
            bundle_bytes: u64::from_be_bytes(bytes[96..104].try_into().unwrap()),
            ranges: u16::from_be_bytes(bytes[104..106].try_into().unwrap()),
            completed_unix: u64::from_be_bytes(bytes[106..114].try_into().unwrap()),
            bundle_key: std::str::from_utf8(&bytes[116..])
                .map_err(|_| invalid("bundle key is not UTF-8"))?
                .to_owned(),
        };
        if completion.root.as_bytes() == &[0; 32]
            || completion.operation == [0; 16]
            || completion.task < FIRST_DYNAMIC_ID
            || completion.bundle_digest.as_bytes() == &[0; 32]
            || completion.bundle_bytes == 0
            || completion.ranges == 0
        {
            return Err(invalid("invalid backup completion identities"));
        }
        Ok(completion)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupRetirement {
    root: RootDigest,
    operation: [u8; 16],
    task: u64,
    retired_unix: u64,
}

impl BackupRetirement {
    pub fn root(&self) -> RootDigest {
        self.root
    }
    pub fn operation(&self) -> [u8; 16] {
        self.operation
    }
    pub fn task(&self) -> u64 {
        self.task
    }
    pub fn retired_unix(&self) -> u64 {
        self.retired_unix
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(RETIREMENT_BYTES);
        bytes.extend_from_slice(BKR_MAGIC);
        bytes.extend_from_slice(self.root.as_bytes());
        bytes.extend_from_slice(&self.operation);
        bytes.extend_from_slice(&self.task.to_be_bytes());
        bytes.extend_from_slice(&self.retired_unix.to_be_bytes());
        bytes
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != RETIREMENT_BYTES || &bytes[..8] != BKR_MAGIC {
            return Err(invalid("invalid backup retirement encoding"));
        }
        let retirement = Self {
            root: RootDigest::from_bytes(bytes[8..40].try_into().unwrap()),
            operation: bytes[40..56].try_into().unwrap(),
            task: u64::from_be_bytes(bytes[56..64].try_into().unwrap()),
            retired_unix: u64::from_be_bytes(bytes[64..72].try_into().unwrap()),
        };
        if retirement.root.as_bytes() == &[0; 32] || retirement.operation == [0; 16] || retirement.task < FIRST_DYNAMIC_ID {
            return Err(invalid("invalid backup retirement identities"));
        }
        Ok(retirement)
    }
}

fn typed_row<T>(row: &Row, root: RootDigest, kind: u64, what: &str, decode: impl FnOnce(&[u8]) -> Result<T>, bind: impl FnOnce(&T) -> (RootDigest, u64)) -> Result<Option<T>> {
    if row.value.get(ColumnId(2)) != Some(&ColumnValue::Uint(kind)) {
        return Ok(None);
    }
    let Some(ColumnValue::Bytes(bytes)) = row.value.get(ColumnId(3)) else {
        return Err(invalid(&format!("{what} payload is missing")));
    };
    let value = decode(bytes)?;
    let (value_root, task) = bind(&value);
    if value_root != root
        || row.pk != [memcmp_uint(task)]
        || row.value.get(ColumnId(1)) != Some(&ColumnValue::Uint(task))
        || row.value.get(ColumnId(4)) != Some(&ColumnValue::Uint(0))
        || row.value.get(ColumnId(5)) != Some(&ColumnValue::Uint(0))
    {
        return Err(invalid(&format!("{what} row binding differs")));
    }
    Ok(Some(value))
}

pub(crate) fn from_backup_row(row: &Row, root: RootDigest) -> Result<Option<BackupIntent>> {
    typed_row(row, root, BACKUP_INTENT, "backup intent", BackupIntent::decode, |i| (i.root, i.task))
}
pub(crate) fn from_anchor_row(row: &Row, root: RootDigest) -> Result<Option<BackupAnchor>> {
    typed_row(row, root, BACKUP_ANCHOR, "backup anchor", BackupAnchor::decode, |a| (a.root, a.task))
}
pub(crate) fn from_completion_row(row: &Row, root: RootDigest) -> Result<Option<BackupCompletion>> {
    typed_row(row, root, BACKUP_COMPLETION, "backup completion", BackupCompletion::decode, |c| (c.root, c.task))
}
pub(crate) fn from_retirement_row(row: &Row, root: RootDigest) -> Result<Option<BackupRetirement>> {
    typed_row(row, root, BACKUP_RETIREMENT, "backup retirement", BackupRetirement::decode, |r| (r.root, r.task))
}

fn insert_row<E: Engine>(txn: &mut MetaTxn<'_, E>, task: u64, kind: u64, payload: Vec<u8>) -> Result<()> {
    let mut row = RowValue::new();
    for (column, value) in [
        (ColumnId(1), ColumnValue::Uint(task)),
        (ColumnId(2), ColumnValue::Uint(kind)),
        (ColumnId(3), ColumnValue::Bytes(payload)),
        (ColumnId(4), ColumnValue::Uint(0)),
        (ColumnId(5), ColumnValue::Uint(0)),
    ] {
        row.set(column, value);
    }
    txn.insert(&TASKS_DESC, &[memcmp_uint(task)], row)
}

/// Everything committed about backups, from one applied view.
#[derive(Debug, Clone, Default)]
pub struct BackupRows {
    pub intents: Vec<BackupIntent>,
    pub anchors: Vec<BackupAnchor>,
    pub completions: Vec<BackupCompletion>,
    pub retirements: Vec<BackupRetirement>,
    /// Backup operations with an effective kind-117 abort.
    pub aborted: std::collections::BTreeSet<[u8; 16]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupStage {
    /// Committed, no range anchored yet.
    Intended,
    /// Some ranges anchored.
    Capturing,
    /// Every range anchored, bundle not yet written.
    Anchored,
    Complete,
    Aborted,
    Retired,
}

impl BackupStage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Intended => "intended",
            Self::Capturing => "capturing",
            Self::Anchored => "anchored",
            Self::Complete => "complete",
            Self::Aborted => "aborted",
            Self::Retired => "retired",
        }
    }
    /// The backup holds its ranges: no split, merge or plan may start on them.
    pub fn holds_ranges(self) -> bool {
        matches!(self, Self::Intended | Self::Capturing)
    }
}

impl BackupRows {
    pub fn anchors_of(&self, intent: &BackupIntent) -> Vec<&BackupAnchor> {
        self.anchors.iter().filter(|a| a.operation == intent.operation).collect()
    }
    pub fn anchor_of_range(&self, range: &BackupRange) -> Option<&BackupAnchor> {
        self.anchors.iter().find(|a| a.range_operation == range.operation)
    }
    pub fn completion_of(&self, intent: &BackupIntent) -> Option<&BackupCompletion> {
        self.completions.iter().find(|c| c.operation == intent.operation)
    }
    pub fn retirement_of(&self, intent: &BackupIntent) -> Option<&BackupRetirement> {
        self.retirements.iter().find(|r| r.operation == intent.operation)
    }
    pub fn stage(&self, intent: &BackupIntent) -> BackupStage {
        if self.retirement_of(intent).is_some() {
            return BackupStage::Retired;
        }
        if self.aborted.contains(&intent.operation) {
            return BackupStage::Aborted;
        }
        if self.completion_of(intent).is_some() {
            return BackupStage::Complete;
        }
        let anchored = intent.ranges.iter().filter(|r| self.anchor_of_range(r).is_some()).count();
        if anchored == 0 {
            BackupStage::Intended
        } else if anchored < intent.ranges.len() {
            BackupStage::Capturing
        } else {
            BackupStage::Anchored
        }
    }
    pub fn intent_by_operation(&self, operation: [u8; 16]) -> Option<&BackupIntent> {
        self.intents.iter().find(|i| i.operation == operation)
    }
    /// The intent and range whose capture operation is `operation`.
    pub fn range_by_capture_operation(&self, operation: [u8; 16]) -> Option<(&BackupIntent, &BackupRange)> {
        self.intents
            .iter()
            .find_map(|i| i.range_by_operation(operation).map(|r| (i, r)))
    }
}

pub fn committed_backups_in_txn<E: Engine>(txn: &MetaTxn<'_, E>) -> Result<BackupRows> {
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("backup readback requires a certified metadata root"))?
        .digest();
    let mut rows = BackupRows {
        aborted: super::range_abort::effective_aborts_in_txn(txn, root)?,
        ..BackupRows::default()
    };
    for row in task_rows(txn) {
        let row = row?;
        if let Some(intent) = from_backup_row(&row, root)? {
            if rows.intents.iter().any(|p| p.task == intent.task || p.operation == intent.operation) {
                return Err(invalid("duplicate backup identity"));
            }
            rows.intents.push(intent);
        } else if let Some(anchor) = from_anchor_row(&row, root)? {
            rows.anchors.push(anchor);
        } else if let Some(completion) = from_completion_row(&row, root)? {
            rows.completions.push(completion);
        } else if let Some(retirement) = from_retirement_row(&row, root)? {
            rows.retirements.push(retirement);
        }
    }
    for anchor in &rows.anchors {
        let intent = rows
            .intent_by_operation(anchor.operation)
            .ok_or_else(|| invalid("backup anchor requires its committed intent"))?;
        if intent.range_by_operation(anchor.range_operation).is_none_or(|r| r.region != anchor.region) {
            return Err(invalid("backup anchor names a range its intent does not"));
        }
    }
    for completion in &rows.completions {
        rows.intent_by_operation(completion.operation)
            .ok_or_else(|| invalid("backup completion requires its committed intent"))?;
    }
    for retirement in &rows.retirements {
        rows.intent_by_operation(retirement.operation)
            .ok_or_else(|| invalid("backup retirement requires its committed intent"))?;
    }
    Ok(rows)
}

pub fn committed_backups<E: Engine>(store: &MetaStore<E>) -> Result<BackupRows> {
    committed_backups_in_txn(&store.begin()?)
}

/// The name of `keyspace` in this catalog view, if it exists.
pub fn keyspace_name_in<E: Engine>(txn: &MetaTxn<'_, E>, keyspace: KeyspaceId) -> Result<Option<String>> {
    Ok(crate::tables::Tables::<E>::keyspace_in(txn, keyspace)?.map(|k| k.name))
}

/// The capture operation of a backup that still holds `region` (intended or
/// capturing, the range not yet anchored), if any. Splits, merges and replica
/// plans refuse while one exists (docs/BACKUP-RESTORE.md §11).
pub(crate) fn live_backup_on_region<E: Engine>(txn: &MetaTxn<'_, E>, region: RegionId) -> Result<Option<[u8; 16]>> {
    let rows = committed_backups_in_txn(txn)?;
    for intent in &rows.intents {
        if !rows.stage(intent).holds_ranges() {
            continue;
        }
        if let Some(range) = intent.range_of_region(region) {
            if rows.anchor_of_range(range).is_none() {
                return Ok(Some(range.operation));
            }
        }
    }
    Ok(None)
}

/// Stage one backup intent for `keyspace` in the serialized, term-fenced catalog
/// transaction. The keyspace's live backup (not complete, aborted or retired)
/// confirms; otherwise the next sequence is committed with the keyspace's bound
/// ranges frozen as they are now. Refuses a range under a live split, merge or
/// replica plan, a sealed range, and a keyspace with no bound range.
pub fn plan_backup<E: Engine>(txn: &mut MetaTxn<'_, E>, keyspace: KeyspaceId, label: &str, now_unix: u64) -> Result<(BackupIntent, bool)> {
    if label.len() > MAX_LABEL_BYTES {
        return Err(invalid("backup label exceeds 64 bytes"));
    }
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("backup requires a certified metadata root"))?
        .digest();
    let keyspace_row = crate::tables::Tables::<E>::keyspace_in(txn, keyspace)?
        .ok_or_else(|| invalid("backup names a keyspace that does not exist"))?;
    let rows = committed_backups_in_txn(txn)?;
    let mut sequence = 0u64;
    for previous in rows.intents.iter().filter(|i| i.keyspace == keyspace) {
        if rows.stage(previous).holds_ranges() || rows.stage(previous) == BackupStage::Anchored {
            return Ok((previous.clone(), false));
        }
        sequence = sequence.max(previous.sequence + 1);
    }
    // A sealed binding is the published parent of a split or merge: history with
    // live successors in the same catalog state (publication seals and binds in one
    // transaction), so it is skipped, never backed up.
    let bindings: Vec<_> = super::ranges::read_in(txn)?
        .into_iter()
        .filter(|b| b.range().keyspace == keyspace && !b.range().sealed)
        .collect();
    if bindings.is_empty() {
        return Err(invalid("backup requires a keyspace bound to at least one data range"));
    }
    if bindings.len() > MAX_BACKUP_RANGES {
        return Err(invalid("backup refuses a keyspace of more than 64 ranges in this increment"));
    }
    let operation = backup_operation(root, keyspace, sequence);
    let mut ranges = Vec::with_capacity(bindings.len());
    for binding in &bindings {
        let range = binding.range();
        if let Some(what) = super::live_range_operation(txn, root, range.region)? {
            return Err(invalid(&format!("a live {what} holds range {}; back up after it publishes", range.region.0)));
        }
        if super::live_plan_on_region(txn, root, range.region)? {
            return Err(invalid(&format!("range {} has an unsettled replica plan; back up after it settles", range.region.0)));
        }
        ranges.push(BackupRange {
            region: range.region,
            creation_task: binding.creation().intent().task(),
            bind_task: binding.bind_task(),
            conf_ver: range.conf_ver,
            version: range.version,
            operation: range_operation(operation, range.region),
        });
    }
    ranges.sort_by_key(|r| r.region.0);
    let intent = BackupIntent {
        root,
        operation,
        task: txn.allocate_id(SequenceKind::Task)?,
        keyspace,
        tenant: keyspace_row.tenant_id,
        sequence,
        requested_unix: now_unix,
        label: label.to_owned(),
        ranges,
    };
    BackupIntent::decode(&intent.encode())?;
    insert_row(txn, intent.task, BACKUP_INTENT, intent.encode())?;
    Ok((intent, true))
}

/// Commit one range's anchor from its leader's receipt. Idempotent per range; a
/// second receipt with other contents refuses (a range is anchored once).
pub fn plan_backup_anchor<E: Engine>(txn: &mut MetaTxn<'_, E>, receipt: &[u8]) -> Result<(BackupAnchor, bool)> {
    let receipt = BackupAnchor::decode_receipt(receipt)?;
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("backup anchor requires a certified metadata root"))?
        .digest();
    if receipt.root != root {
        return Err(invalid("backup anchor root differs from the certified root"));
    }
    let rows = committed_backups_in_txn(txn)?;
    let intent = rows
        .intent_by_operation(receipt.operation)
        .ok_or_else(|| invalid("backup anchor requires the committed intent"))?;
    match rows.stage(intent) {
        BackupStage::Intended | BackupStage::Capturing | BackupStage::Anchored => {}
        other => return Err(invalid(&format!("the backup is {}; its anchor is refused", other.label()))),
    }
    let range = intent
        .range_by_operation(receipt.range_operation)
        .filter(|r| r.region == receipt.region)
        .ok_or_else(|| invalid("backup anchor names a range its intent does not"))?;
    if let Some(previous) = rows.anchor_of_range(range) {
        let expected = BackupAnchor { task: previous.task, ..receipt.clone() };
        if &expected != previous {
            return Err(invalid("a backup anchor for this range conflicts"));
        }
        return Ok((previous.clone(), false));
    }
    let anchor = BackupAnchor { task: txn.allocate_id(SequenceKind::Task)?, ..receipt };
    BackupAnchor::decode(&anchor.encode())?;
    insert_row(txn, anchor.task, BACKUP_ANCHOR, anchor.encode())?;
    Ok((anchor, true))
}

/// Commit the completion once every range of `backup_task` is anchored. Idempotent.
pub fn plan_backup_completion<E: Engine>(
    txn: &mut MetaTxn<'_, E>,
    backup_task: u64,
    bundle_key: &str,
    bundle_digest: RootDigest,
    bundle_bytes: u64,
    now_unix: u64,
) -> Result<(BackupCompletion, bool)> {
    if bundle_key.is_empty() || bundle_key.len() > MAX_BUNDLE_KEY_BYTES {
        return Err(invalid("bundle key must contain 1..256 bytes"));
    }
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("backup completion requires a certified metadata root"))?
        .digest();
    let rows = committed_backups_in_txn(txn)?;
    let intent = rows
        .intents
        .iter()
        .find(|i| i.task == backup_task)
        .ok_or_else(|| invalid("backup completion names a task that is not a backup"))?;
    if let Some(previous) = rows.completion_of(intent) {
        if previous.bundle_key != bundle_key || previous.bundle_digest != bundle_digest || previous.bundle_bytes != bundle_bytes {
            return Err(invalid("a different bundle already completes this backup"));
        }
        return Ok((previous.clone(), false));
    }
    match rows.stage(intent) {
        BackupStage::Anchored => {}
        other => return Err(invalid(&format!("the backup is {}; completion needs every range anchored", other.label()))),
    }
    let completion = BackupCompletion {
        root,
        operation: intent.operation,
        task: txn.allocate_id(SequenceKind::Task)?,
        bundle_digest,
        bundle_bytes,
        ranges: intent.ranges.len() as u16,
        completed_unix: now_unix,
        bundle_key: bundle_key.to_owned(),
    };
    BackupCompletion::decode(&completion.encode())?;
    insert_row(txn, completion.task, BACKUP_COMPLETION, completion.encode())?;
    Ok((completion, true))
}

/// Retire a COMPLETE backup: the only release of its pins. An incomplete backup
/// is aborted instead (`task abort`). Idempotent.
pub fn plan_backup_retirement<E: Engine>(txn: &mut MetaTxn<'_, E>, backup_task: u64, now_unix: u64) -> Result<(BackupRetirement, bool)> {
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("backup retirement requires a certified metadata root"))?
        .digest();
    let rows = committed_backups_in_txn(txn)?;
    let intent = rows
        .intents
        .iter()
        .find(|i| i.task == backup_task)
        .ok_or_else(|| invalid("backup retire names a task that is not a backup"))?;
    if let Some(previous) = rows.retirement_of(intent) {
        return Ok((previous.clone(), false));
    }
    match rows.stage(intent) {
        BackupStage::Complete => {}
        BackupStage::Aborted => return Err(invalid("the backup is aborted; nothing to retire")),
        other => return Err(invalid(&format!("the backup is {}; abort it instead of retiring it", other.label()))),
    }
    let retirement = BackupRetirement {
        root,
        operation: intent.operation,
        task: txn.allocate_id(SequenceKind::Task)?,
        retired_unix: now_unix,
    };
    BackupRetirement::decode(&retirement.encode())?;
    insert_row(txn, retirement.task, BACKUP_RETIREMENT, retirement.encode())?;
    Ok((retirement, true))
}

/// The bundle object (`KV9BND01`, docs/BACKUP-RESTORE.md §2): everything a
/// restore needs, in this cluster or another one that can read the bucket. It
/// references the capture objects where the capture put them; it copies none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupBundle {
    pub cluster_id: [u8; 16],
    pub root: RootDigest,
    pub task: u64,
    pub operation: [u8; 16],
    pub keyspace: KeyspaceId,
    pub tenant: TenantId,
    pub sequence: u64,
    pub requested_unix: u64,
    pub label: String,
    pub keyspace_name: String,
    pub ranges: Vec<BundleRange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleRange {
    pub region: RegionId,
    pub creation_task: u64,
    pub bind_task: u64,
    pub conf_ver: u64,
    pub version: u64,
    pub start: Vec<u8>,
    pub end: Vec<u8>,
    pub anchor: BackupAnchor,
    /// The committed capture descriptor (request, image cut and configuration,
    /// manifest with the SST closure), opaque here; the installer and the
    /// restore decode it.
    pub descriptor: Vec<u8>,
}

const BND_MAGIC: &[u8; 8] = b"KV9BND01";
pub const MAX_BUNDLE_BYTES: usize = 4 << 20;
const MAX_BUNDLE_KEY_LEN: usize = 4096;
const MAX_DESCRIPTOR_BYTES: usize = 1 << 20;

fn put_bytes(out: &mut Vec<u8>, bytes: &[u8], width: usize) {
    match width {
        2 => out.extend_from_slice(&(bytes.len() as u16).to_be_bytes()),
        _ => out.extend_from_slice(&(bytes.len() as u32).to_be_bytes()),
    }
    out.extend_from_slice(bytes);
}

struct Cursor<'a>(&'a [u8], usize);
impl Cursor<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        if self.1 + n > self.0.len() {
            return Err(invalid("truncated backup bundle"));
        }
        let out = &self.0[self.1..self.1 + n];
        self.1 += n;
        Ok(out)
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn bytes(&mut self, width: usize, max: usize) -> Result<Vec<u8>> {
        let len = match width {
            2 => usize::from(u16::from_be_bytes(self.take(2)?.try_into().unwrap())),
            _ => u32::from_be_bytes(self.take(4)?.try_into().unwrap()) as usize,
        };
        if len > max {
            return Err(invalid("backup bundle field exceeds its bound"));
        }
        Ok(self.take(len)?.to_vec())
    }
    fn text(&mut self, max: usize) -> Result<String> {
        String::from_utf8(self.bytes(2, max)?).map_err(|_| invalid("backup bundle text is not UTF-8"))
    }
}

impl BackupBundle {
    /// The object key of a backup's bundle: `backups/<cluster-id>/<task>/bundle`.
    pub fn object_key(cluster_id: &[u8; 16], task: u64) -> String {
        let hex: String = cluster_id.iter().map(|b| format!("{b:02x}")).collect();
        format!("backups/{hex}/{task}/bundle")
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4096);
        out.extend_from_slice(BND_MAGIC);
        out.extend_from_slice(&self.cluster_id);
        out.extend_from_slice(self.root.as_bytes());
        out.extend_from_slice(&self.task.to_be_bytes());
        out.extend_from_slice(&self.operation);
        out.extend_from_slice(&(self.keyspace.0 as u64).to_be_bytes());
        out.extend_from_slice(&self.tenant.0.to_be_bytes());
        out.extend_from_slice(&self.sequence.to_be_bytes());
        out.extend_from_slice(&self.requested_unix.to_be_bytes());
        put_bytes(&mut out, self.label.as_bytes(), 2);
        put_bytes(&mut out, self.keyspace_name.as_bytes(), 2);
        out.extend_from_slice(&(self.ranges.len() as u16).to_be_bytes());
        for r in &self.ranges {
            out.extend_from_slice(&r.region.0.to_be_bytes());
            out.extend_from_slice(&r.creation_task.to_be_bytes());
            out.extend_from_slice(&r.bind_task.to_be_bytes());
            out.extend_from_slice(&r.conf_ver.to_be_bytes());
            out.extend_from_slice(&r.version.to_be_bytes());
            put_bytes(&mut out, &r.start, 2);
            put_bytes(&mut out, &r.end, 2);
            put_bytes(&mut out, &r.anchor.encode(), 2);
            put_bytes(&mut out, &r.descriptor, 4);
        }
        let digest = RootDigest::sha256(&out);
        out.extend_from_slice(digest.as_bytes());
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 8 + 32 || bytes.len() > MAX_BUNDLE_BYTES || &bytes[..8] != BND_MAGIC {
            return Err(invalid("invalid backup bundle encoding"));
        }
        let (body, trailer) = bytes.split_at(bytes.len() - 32);
        if RootDigest::sha256(body).as_bytes() != trailer {
            return Err(invalid("backup bundle digest mismatch"));
        }
        let mut c = Cursor(body, 8);
        let cluster_id: [u8; 16] = c.take(16)?.try_into().unwrap();
        let root = RootDigest::from_bytes(c.take(32)?.try_into().unwrap());
        let task = c.u64()?;
        let operation: [u8; 16] = c.take(16)?.try_into().unwrap();
        let keyspace = KeyspaceId(u32::try_from(c.u64()?).map_err(|_| invalid("bundle keyspace id out of range"))?);
        let tenant = TenantId(c.u64()?);
        let sequence = c.u64()?;
        let requested_unix = c.u64()?;
        let label = c.text(MAX_LABEL_BYTES)?;
        let keyspace_name = c.text(1024)?;
        let count = usize::from(u16::from_be_bytes(c.take(2)?.try_into().unwrap()));
        if count == 0 || count > MAX_BACKUP_RANGES {
            return Err(invalid("invalid backup bundle range count"));
        }
        let mut ranges = Vec::with_capacity(count);
        for _ in 0..count {
            let region = RegionId(c.u64()?);
            let creation_task = c.u64()?;
            let bind_task = c.u64()?;
            let conf_ver = c.u64()?;
            let version = c.u64()?;
            let start = c.bytes(2, MAX_BUNDLE_KEY_LEN)?;
            let end = c.bytes(2, MAX_BUNDLE_KEY_LEN)?;
            let anchor = BackupAnchor::decode(&c.bytes(2, ANCHOR_BYTES)?)?;
            let descriptor = c.bytes(4, MAX_DESCRIPTOR_BYTES)?;
            if anchor.region() != region || anchor.operation() != operation || anchor.root() != root || descriptor.is_empty() {
                return Err(invalid("backup bundle range differs from its anchor"));
            }
            ranges.push(BundleRange { region, creation_task, bind_task, conf_ver, version, start, end, anchor, descriptor });
        }
        if c.1 != body.len() {
            return Err(invalid("backup bundle has trailing bytes"));
        }
        if operation != backup_operation(root, keyspace, sequence) || cluster_id == [0; 16] {
            return Err(invalid("invalid backup bundle identities"));
        }
        Ok(Self { cluster_id, root, task, operation, keyspace, tenant, sequence, requested_unix, label, keyspace_name, ranges })
    }

    /// Every range's anchor carries rows and a digest a restore can verify against.
    pub fn restorable(&self) -> bool {
        self.ranges.iter().all(|r| r.anchor.restorable())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bound keyspace on a group of 1,2,3 (the fixture registers them).
    fn bound_keyspace(store: &MetaStore<kv9_engine::MemEngine>) -> (KeyspaceId, RegionId, u64) {
        let mut txn = store.begin().unwrap();
        let mut tenant = RowValue::new();
        tenant.set(ColumnId(2), ColumnValue::Text("tenant".into()));
        tenant.set(ColumnId(3), ColumnValue::Uint(0));
        tenant.set(ColumnId(4), ColumnValue::Uint(0));
        txn.insert(&crate::schema::TENANTS_DESC, &[memcmp_uint(1)], tenant).unwrap();
        let creation = plan_empty_group(&mut txn, [1; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        let root = crate::root::certified_root(&txn).unwrap().unwrap().digest();
        let (range, _) = super::super::ranges::plan_keyspace(&mut txn, root, creation.task(), "orders", TenantId(1)).unwrap();
        txn.commit().unwrap();
        (range.keyspace, range.region, creation.task())
    }

    fn anchor_receipt(intent: &BackupIntent, range: &BackupRange, rows: Option<(u64, RootDigest)>) -> Vec<u8> {
        BackupAnchor::receipt(
            intent,
            range,
            AppliedPosition { term: 2, index: 40 },
            None,
            RootDigest::sha256(b"image"),
            RootDigest::sha256(b"manifest"),
            3,
            4096,
            rows,
            [9; 16],
        )
        .unwrap()
    }

    #[test]
    fn a_backup_freezes_the_shape_anchors_each_range_completes_and_retires() {
        let store = super::super::tests::fixture();
        let (keyspace, region, creation_task) = bound_keyspace(&store);
        let mut txn = store.begin().unwrap();
        assert!(plan_backup(&mut txn, KeyspaceId(9), "x", 1).is_err(), "unknown keyspace");
        let (intent, changed) = plan_backup(&mut txn, keyspace, "nightly", 1_793_300_000).unwrap();
        assert!(changed);
        assert_eq!(intent.ranges().len(), 1);
        assert_eq!(intent.ranges()[0].region(), region);
        assert_eq!(intent.ranges()[0].creation_task(), creation_task);
        assert_eq!(intent.label(), "nightly");
        txn.commit().unwrap();

        // The live backup confirms; it holds the range for splits, merges and plans.
        let mut txn = store.begin().unwrap();
        let (again, changed) = plan_backup(&mut txn, keyspace, "other label", 2).unwrap();
        assert!(!changed && again == intent);
        assert_eq!(live_backup_on_region(&txn, region).unwrap(), Some(intent.ranges()[0].operation()));
        assert_eq!(super::super::live_range_operation(&txn, intent.root(), region).unwrap(), Some("backup"));
        assert!(
            super::super::migration::plan_migration(&mut txn, [5; 16], creation_task, NodeId(2)).is_err(),
            "a plan waits for the backup"
        );
        drop(txn);
        let rows = committed_backups(&store).unwrap();
        assert_eq!(rows.stage(&intent), BackupStage::Intended);

        // A tampered intent is refused by decode; completion and retirement wait for anchors.
        let mut bytes = intent.encode();
        bytes[40] ^= 1;
        assert!(BackupIntent::decode(&bytes).is_err());
        let mut txn = store.begin().unwrap();
        assert!(plan_backup_completion(&mut txn, intent.task(), "k", RootDigest::sha256(b"b"), 1, 3).is_err());
        assert!(plan_backup_retirement(&mut txn, intent.task(), 3).is_err());

        // The range leader's receipt anchors the range; a conflicting one refuses.
        let receipt = anchor_receipt(&intent, &intent.ranges()[0], Some((10, RootDigest::sha256(b"rows"))));
        assert!(is_anchor_receipt(&receipt));
        let (anchor, changed) = plan_backup_anchor(&mut txn, &receipt).unwrap();
        assert!(changed && anchor.restorable() && anchor.rows() == 10);
        let (same, changed) = plan_backup_anchor(&mut txn, &receipt).unwrap();
        assert!(!changed && same == anchor);
        let other = anchor_receipt(&intent, &intent.ranges()[0], None);
        assert!(plan_backup_anchor(&mut txn, &other).is_err(), "a range is anchored once");
        txn.commit().unwrap();
        let rows = committed_backups(&store).unwrap();
        assert_eq!(rows.stage(&intent), BackupStage::Anchored);
        assert_eq!(live_backup_on_region(&store.begin().unwrap(), region).unwrap(), None, "anchored: the range is free");

        // Completion records the bundle; a second backup of the keyspace is a new sequence.
        let mut txn = store.begin().unwrap();
        let (completion, changed) = plan_backup_completion(&mut txn, intent.task(), "backups/c/100/bundle", RootDigest::sha256(b"bundle"), 512, 4).unwrap();
        assert!(changed && completion.ranges() == 1);
        txn.commit().unwrap();
        let rows = committed_backups(&store).unwrap();
        assert_eq!(rows.stage(&intent), BackupStage::Complete);
        let mut txn = store.begin().unwrap();
        let (second, changed) = plan_backup(&mut txn, keyspace, "", 5).unwrap();
        assert!(changed && second.sequence() == 1 && second.operation() != intent.operation());
        // Abort the second: it is history; the first retires.
        let (abort, _) = super::super::range_abort::plan_range_abort(&mut txn, second.task()).unwrap();
        assert_eq!(abort.target(), super::super::range_abort::AbortTarget::Backup);
        assert!(super::super::range_abort::plan_range_abort(&mut txn, intent.task()).is_err(), "a complete backup retires, not aborts");
        let (retirement, changed) = plan_backup_retirement(&mut txn, intent.task(), 6).unwrap();
        assert!(changed && retirement.operation() == intent.operation());
        txn.commit().unwrap();
        let rows = committed_backups(&store).unwrap();
        assert_eq!(rows.stage(&intent), BackupStage::Retired);
        assert_eq!(rows.stage(&second), BackupStage::Aborted);
        assert!(plan_backup_anchor(&mut store.begin().unwrap(), &anchor_receipt(&second, &second.ranges()[0], None)).is_err());
        // A third backup is sequence 2.
        let mut txn = store.begin().unwrap();
        let (third, _) = plan_backup(&mut txn, keyspace, "", 7).unwrap();
        assert_eq!(third.sequence(), 2);
    }

    #[test]
    fn a_bundle_round_trips_and_refuses_tampering() {
        let root = RootDigest::sha256(b"root");
        let operation = backup_operation(root, KeyspaceId(100), 3);
        let range_op = range_operation(operation, RegionId(7));
        let intent = BackupIntent {
            root,
            operation,
            task: 300,
            keyspace: KeyspaceId(100),
            tenant: TenantId(1),
            sequence: 3,
            requested_unix: 9,
            label: "l".into(),
            ranges: vec![BackupRange { region: RegionId(7), creation_task: 150, bind_task: 151, conf_ver: 1, version: 2, operation: range_op }],
        };
        let anchor = BackupAnchor::decode_receipt(&anchor_receipt(&intent, &intent.ranges[0], None)).unwrap();
        let anchor = BackupAnchor { task: 301, ..anchor };
        let bundle = BackupBundle {
            cluster_id: [4; 16],
            root,
            task: 300,
            operation,
            keyspace: KeyspaceId(100),
            tenant: TenantId(1),
            sequence: 3,
            requested_unix: 9,
            label: "l".into(),
            keyspace_name: "orders".into(),
            ranges: vec![BundleRange {
                region: RegionId(7),
                creation_task: 150,
                bind_task: 151,
                conf_ver: 1,
                version: 2,
                start: b"a".to_vec(),
                end: Vec::new(),
                anchor,
                descriptor: vec![1, 2, 3],
            }],
        };
        let bytes = bundle.encode();
        assert_eq!(BackupBundle::decode(&bytes).unwrap(), bundle);
        assert!(!bundle.restorable());
        let mut tampered = bytes.clone();
        tampered[60] ^= 1;
        assert!(BackupBundle::decode(&tampered).is_err());
        assert_eq!(BackupBundle::object_key(&[4; 16], 300), format!("backups/{}/300/bundle", "04".repeat(16)));
    }

    #[test]
    fn identities_are_derived_and_the_destination_is_synthetic() {
        let root = RootDigest::sha256(b"root");
        let op = backup_operation(root, KeyspaceId(100), 0);
        let range = range_operation(op, RegionId(100));
        assert_ne!(op, range);
        assert_ne!(range, range_operation(op, RegionId(101)));
        let destination = backup_destination(range);
        assert_eq!(destination.node, BACKUP_DESTINATION_NODE);
        assert_ne!(destination.incarnation.as_bytes(), &[0; 16]);
        assert_ne!(destination.incarnation, backup_destination(range_operation(op, RegionId(101))).incarnation);
    }
}
