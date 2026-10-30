//! Restore of a backup into a NEW keyspace (docs/BACKUP-RESTORE.md §5): two
//! immutable row kinds.
//!
//! - **123, the intent** (`KV9RST01`): the complete backup it restores, the new
//!   keyspace's reserved id, name and tenant, and one target per backed-up range —
//!   an empty group created and activated in the same catalog transaction, with
//!   the range's boundaries. The new keyspace does not exist for clients until the
//!   publication; the targets carry descriptors for the reserved id before that.
//! - **124, the evidence** (`KV9RSE01`): a target's leader copied its range from the
//!   anchored image through the target's own log and digested the result; the row
//!   is committed only when rows and digest equal the anchor's (B5). Publication
//!   needs every target's row and inserts the keyspace, its bindings and routing
//!   rows at once (B4); readback refuses a partial shape.
//!
//! Restore writes only new identities (B3): the source keyspace, its groups and
//! the backup are read, never modified. A restore aborts through kind 117 before
//! publication; its targets then retire like an aborted split's children (B6).

use super::*;
use kv9_common::{KeyspaceId, TenantId};

const RESTORE_INTENT: u64 = 123;
const RESTORE_EVIDENCE: u64 = 124;
const RST_MAGIC: &[u8; 8] = b"KV9RST01";
const RSE_MAGIC: &[u8; 8] = b"KV9RSE01";
const MAX_NAME_BYTES: usize = 1024;
const MAX_BOUND_BYTES: usize = 1024;
const INTENT_HEADER: usize = 8 + 32 + 16 + 8 + 8 + 16 + 4 + 8 + 8 + 8 + 2 + 2;
const TARGET_FIXED: usize = 8 + 16 + 8 + 8 + 2 + 2;
pub const MAX_RESTORE_INTENT_BYTES: usize =
    INTENT_HEADER + MAX_NAME_BYTES + super::backup::MAX_BACKUP_RANGES * (TARGET_FIXED + 2 * MAX_BOUND_BYTES);
pub const EVIDENCE_BYTES: usize = 8 + 32 + 16 + 8 + 8 + 8 + 8 + 32;

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

/// `sha16("kv9-restore/v1" ++ root ++ backup operation ++ name ++ sequence)`.
pub fn restore_operation(root: RootDigest, backup: [u8; 16], name: &str, sequence: u64) -> [u8; 16] {
    sha16(&[b"kv9-restore/v1", root.as_bytes(), &backup, name.as_bytes(), &sequence.to_be_bytes()])
}

/// The creation operation of one target group: `sha16("kv9-restore-target/v1" ++ restore operation ++ source region)`.
pub fn target_operation(restore: [u8; 16], source_region: RegionId) -> [u8; 16] {
    sha16(&[b"kv9-restore-target/v1", &restore, &source_region.0.to_be_bytes()])
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreTarget {
    source_region: RegionId,
    /// The source range's capture operation: the anchor and the image to copy from.
    source_operation: [u8; 16],
    creation_task: u64,
    region: RegionId,
    start: Vec<u8>,
    end: Vec<u8>,
}

impl RestoreTarget {
    pub fn source_region(&self) -> RegionId {
        self.source_region
    }
    pub fn source_operation(&self) -> [u8; 16] {
        self.source_operation
    }
    pub fn creation_task(&self) -> u64 {
        self.creation_task
    }
    pub fn region(&self) -> RegionId {
        self.region
    }
    pub fn start(&self) -> &[u8] {
        &self.start
    }
    pub fn end(&self) -> &[u8] {
        &self.end
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreIntent {
    root: RootDigest,
    operation: [u8; 16],
    task: u64,
    backup_task: u64,
    sequence: u64,
    backup_operation: [u8; 16],
    keyspace: KeyspaceId,
    tenant: TenantId,
    requested_unix: u64,
    name: String,
    targets: Vec<RestoreTarget>,
}

impl RestoreIntent {
    pub fn root(&self) -> RootDigest {
        self.root
    }
    pub fn operation(&self) -> [u8; 16] {
        self.operation
    }
    pub fn task(&self) -> u64 {
        self.task
    }
    pub fn backup_task(&self) -> u64 {
        self.backup_task
    }
    pub fn backup_operation(&self) -> [u8; 16] {
        self.backup_operation
    }
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    /// The NEW keyspace's id, reserved at intent, published later.
    pub fn keyspace(&self) -> KeyspaceId {
        self.keyspace
    }
    pub fn tenant(&self) -> TenantId {
        self.tenant
    }
    pub fn requested_unix(&self) -> u64 {
        self.requested_unix
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn targets(&self) -> &[RestoreTarget] {
        &self.targets
    }
    pub fn target_by_region(&self, region: RegionId) -> Option<&RestoreTarget> {
        self.targets.iter().find(|t| t.region == region)
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(512);
        bytes.extend_from_slice(RST_MAGIC);
        bytes.extend_from_slice(self.root.as_bytes());
        bytes.extend_from_slice(&self.operation);
        bytes.extend_from_slice(&self.task.to_be_bytes());
        bytes.extend_from_slice(&self.backup_task.to_be_bytes());
        bytes.extend_from_slice(&self.backup_operation);
        bytes.extend_from_slice(&self.keyspace.0.to_be_bytes());
        bytes.extend_from_slice(&self.tenant.0.to_be_bytes());
        bytes.extend_from_slice(&self.sequence.to_be_bytes());
        bytes.extend_from_slice(&self.requested_unix.to_be_bytes());
        bytes.extend_from_slice(&(self.name.len() as u16).to_be_bytes());
        bytes.extend_from_slice(&(self.targets.len() as u16).to_be_bytes());
        bytes.extend_from_slice(self.name.as_bytes());
        for t in &self.targets {
            bytes.extend_from_slice(&t.source_region.0.to_be_bytes());
            bytes.extend_from_slice(&t.source_operation);
            bytes.extend_from_slice(&t.creation_task.to_be_bytes());
            bytes.extend_from_slice(&t.region.0.to_be_bytes());
            bytes.extend_from_slice(&(t.start.len() as u16).to_be_bytes());
            bytes.extend_from_slice(&(t.end.len() as u16).to_be_bytes());
            bytes.extend_from_slice(&t.start);
            bytes.extend_from_slice(&t.end);
        }
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < INTENT_HEADER || bytes.len() > MAX_RESTORE_INTENT_BYTES || &bytes[..8] != RST_MAGIC {
            return Err(invalid("invalid restore intent encoding"));
        }
        let u = |at: usize| u64::from_be_bytes(bytes[at..at + 8].try_into().unwrap());
        let name_len = usize::from(u16::from_be_bytes(bytes[116..118].try_into().unwrap()));
        let count = usize::from(u16::from_be_bytes(bytes[118..120].try_into().unwrap()));
        if name_len == 0 || name_len > MAX_NAME_BYTES || count == 0 || count > super::backup::MAX_BACKUP_RANGES || bytes.len() < INTENT_HEADER + name_len {
            return Err(invalid("invalid restore intent shape"));
        }
        let name = std::str::from_utf8(&bytes[120..120 + name_len])
            .map_err(|_| invalid("restore name is not UTF-8"))?
            .to_owned();
        let mut at = 120 + name_len;
        let mut targets = Vec::with_capacity(count);
        for _ in 0..count {
            if bytes.len() < at + TARGET_FIXED {
                return Err(invalid("truncated restore target"));
            }
            let source_region = RegionId(u(at));
            let source_operation: [u8; 16] = bytes[at + 8..at + 24].try_into().unwrap();
            let creation_task = u(at + 24);
            let region = RegionId(u(at + 32));
            let start_len = usize::from(u16::from_be_bytes(bytes[at + 40..at + 42].try_into().unwrap()));
            let end_len = usize::from(u16::from_be_bytes(bytes[at + 42..at + 44].try_into().unwrap()));
            at += TARGET_FIXED;
            if start_len > MAX_BOUND_BYTES || end_len > MAX_BOUND_BYTES || bytes.len() < at + start_len + end_len {
                return Err(invalid("invalid restore target bounds"));
            }
            let start = bytes[at..at + start_len].to_vec();
            let end = bytes[at + start_len..at + start_len + end_len].to_vec();
            at += start_len + end_len;
            targets.push(RestoreTarget { source_region, source_operation, creation_task, region, start, end });
        }
        if at != bytes.len() {
            return Err(invalid("restore intent has trailing bytes"));
        }
        let intent = Self {
            root: RootDigest::from_bytes(bytes[8..40].try_into().unwrap()),
            operation: bytes[40..56].try_into().unwrap(),
            task: u(56),
            backup_task: u(64),
            backup_operation: bytes[72..88].try_into().unwrap(),
            keyspace: KeyspaceId(u32::from_be_bytes(bytes[88..92].try_into().unwrap())),
            tenant: TenantId(u(92)),
            sequence: u(100),
            requested_unix: u(108),
            name,
            targets,
        };
        if intent.root.as_bytes() == &[0; 32]
            || intent.task < FIRST_DYNAMIC_ID
            || intent.backup_task < FIRST_DYNAMIC_ID
            || intent.keyspace.0 == 0
            || intent.operation != restore_operation(intent.root, intent.backup_operation, &intent.name, intent.sequence)
            || intent.targets.iter().any(|t| {
                t.creation_task < FIRST_DYNAMIC_ID
                    || t.source_operation != super::backup::range_operation(intent.backup_operation, t.source_region)
            })
            || (1..intent.targets.len()).any(|i| intent.targets[..i].iter().any(|p| p.region == intent.targets[i].region || p.source_region == intent.targets[i].source_region))
        {
            return Err(invalid("invalid restore intent identities"));
        }
        Ok(intent)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreEvidence {
    root: RootDigest,
    operation: [u8; 16],
    task: u64,
    source_region: RegionId,
    target_task: u64,
    rows: u64,
    rows_digest: RootDigest,
}

impl RestoreEvidence {
    pub fn root(&self) -> RootDigest {
        self.root
    }
    pub fn operation(&self) -> [u8; 16] {
        self.operation
    }
    pub fn task(&self) -> u64 {
        self.task
    }
    pub fn source_region(&self) -> RegionId {
        self.source_region
    }
    pub fn target_task(&self) -> u64 {
        self.target_task
    }
    pub fn rows(&self) -> u64 {
        self.rows
    }
    pub fn rows_digest(&self) -> RootDigest {
        self.rows_digest
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(EVIDENCE_BYTES);
        bytes.extend_from_slice(RSE_MAGIC);
        bytes.extend_from_slice(self.root.as_bytes());
        bytes.extend_from_slice(&self.operation);
        bytes.extend_from_slice(&self.task.to_be_bytes());
        bytes.extend_from_slice(&self.source_region.0.to_be_bytes());
        bytes.extend_from_slice(&self.target_task.to_be_bytes());
        bytes.extend_from_slice(&self.rows.to_be_bytes());
        bytes.extend_from_slice(self.rows_digest.as_bytes());
        bytes
    }
    fn decode_inner(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != EVIDENCE_BYTES || &bytes[..8] != RSE_MAGIC {
            return Err(invalid("invalid restore evidence encoding"));
        }
        let u = |at: usize| u64::from_be_bytes(bytes[at..at + 8].try_into().unwrap());
        let evidence = Self {
            root: RootDigest::from_bytes(bytes[8..40].try_into().unwrap()),
            operation: bytes[40..56].try_into().unwrap(),
            task: u(56),
            source_region: RegionId(u(64)),
            target_task: u(72),
            rows: u(80),
            rows_digest: RootDigest::from_bytes(bytes[88..120].try_into().unwrap()),
        };
        if evidence.root.as_bytes() == &[0; 32] || evidence.operation == [0; 16] || evidence.target_task < FIRST_DYNAMIC_ID {
            return Err(invalid("invalid restore evidence identities"));
        }
        Ok(evidence)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let e = Self::decode_inner(bytes)?;
        if e.task < FIRST_DYNAMIC_ID {
            return Err(invalid("invalid restore evidence task"));
        }
        Ok(e)
    }
    pub fn decode_receipt(bytes: &[u8]) -> Result<Self> {
        let e = Self::decode_inner(bytes)?;
        if e.task != 0 {
            return Err(invalid("a restore evidence receipt cannot supply its own task"));
        }
        Ok(e)
    }
    /// Canonical receipt bytes a target leader emits after its copy and digest.
    pub fn receipt(intent: &RestoreIntent, target: &RestoreTarget, rows: u64, rows_digest: RootDigest) -> Result<Vec<u8>> {
        let receipt = Self {
            root: intent.root,
            operation: intent.operation,
            task: 0,
            source_region: target.source_region,
            target_task: target.creation_task,
            rows,
            rows_digest,
        };
        let bytes = receipt.encode();
        Self::decode_receipt(&bytes)?;
        Ok(bytes)
    }
}

pub fn is_restore_receipt(bytes: &[u8]) -> bool {
    bytes.len() == EVIDENCE_BYTES && &bytes[..8] == RSE_MAGIC
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

pub(crate) fn from_restore_row(row: &Row, root: RootDigest) -> Result<Option<RestoreIntent>> {
    typed_row(row, root, RESTORE_INTENT, "restore intent", RestoreIntent::decode, |i| (i.root, i.task))
}
pub(crate) fn from_restore_evidence_row(row: &Row, root: RootDigest) -> Result<Option<RestoreEvidence>> {
    typed_row(row, root, RESTORE_EVIDENCE, "restore evidence", RestoreEvidence::decode, |e| (e.root, e.task))
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreStage {
    Intended,
    Populating,
    Populated,
    Published,
    Aborted,
}

impl RestoreStage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Intended => "intended",
            Self::Populating => "populating",
            Self::Populated => "populated",
            Self::Published => "published",
            Self::Aborted => "aborted",
        }
    }
    pub fn live(self) -> bool {
        matches!(self, Self::Intended | Self::Populating | Self::Populated)
    }
}

#[derive(Debug, Clone, Default)]
pub struct RestoreRows {
    pub intents: Vec<RestoreIntent>,
    pub evidence: Vec<RestoreEvidence>,
    /// Restore operations with an effective kind-117 abort.
    pub aborted: std::collections::BTreeSet<[u8; 16]>,
    /// Reserved keyspace ids whose keyspace row exists: the restore published.
    pub published: std::collections::BTreeSet<u32>,
}

impl RestoreRows {
    pub fn evidence_of(&self, intent: &RestoreIntent, target: &RestoreTarget) -> Option<&RestoreEvidence> {
        self.evidence
            .iter()
            .find(|e| e.operation == intent.operation && e.target_task == target.creation_task)
    }
    pub fn stage(&self, intent: &RestoreIntent) -> RestoreStage {
        if self.published.contains(&intent.keyspace.0) {
            return RestoreStage::Published;
        }
        if self.aborted.contains(&intent.operation) {
            return RestoreStage::Aborted;
        }
        let evidenced = intent.targets.iter().filter(|t| self.evidence_of(intent, t).is_some()).count();
        if evidenced == 0 {
            RestoreStage::Intended
        } else if evidenced < intent.targets.len() {
            RestoreStage::Populating
        } else {
            RestoreStage::Populated
        }
    }
    pub fn intent_by_operation(&self, operation: [u8; 16]) -> Option<&RestoreIntent> {
        self.intents.iter().find(|i| i.operation == operation)
    }
    /// Live restores of `backup_task`: a backup with one does not retire.
    pub fn live_of_backup(&self, backup_task: u64) -> Vec<&RestoreIntent> {
        self.intents
            .iter()
            .filter(|i| i.backup_task == backup_task && self.stage(i).live())
            .collect()
    }
}

pub fn committed_restores_in_txn<E: Engine>(txn: &MetaTxn<'_, E>) -> Result<RestoreRows> {
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("restore readback requires a certified metadata root"))?
        .digest();
    let mut rows = RestoreRows {
        aborted: super::range_abort::effective_aborts_in_txn(txn, root)?,
        ..RestoreRows::default()
    };
    for row in task_rows(txn) {
        let row = row?;
        if let Some(intent) = from_restore_row(&row, root)? {
            if rows.intents.iter().any(|p| p.task == intent.task || p.operation == intent.operation) {
                return Err(invalid("duplicate restore identity"));
            }
            rows.intents.push(intent);
        } else if let Some(evidence) = from_restore_evidence_row(&row, root)? {
            rows.evidence.push(evidence);
        }
    }
    for e in &rows.evidence {
        let intent = rows
            .intent_by_operation(e.operation)
            .ok_or_else(|| invalid("restore evidence requires its committed intent"))?;
        if !intent.targets.iter().any(|t| t.creation_task == e.target_task && t.source_region == e.source_region) {
            return Err(invalid("restore evidence names a target its intent does not"));
        }
    }
    for intent in &rows.intents {
        if crate::tables::Tables::<E>::keyspace_in(txn, intent.keyspace)?.is_some() {
            rows.published.insert(intent.keyspace.0);
        }
    }
    Ok(rows)
}

pub fn committed_restores<E: Engine>(store: &MetaStore<E>) -> Result<RestoreRows> {
    committed_restores_in_txn(&store.begin()?)
}

/// Is `name` the name of an existing keyspace (any id)?
fn keyspace_name_taken<E: Engine>(txn: &MetaTxn<'_, E>, name: &str) -> Result<bool> {
    use crate::schema::KEYSPACES_DESC;
    for row in txn.scan(&KEYSPACES_DESC, usize::MAX)? {
        if row.value.get(ColumnId(2)) == Some(&ColumnValue::Text(name.to_owned())) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Stage one restore of the COMPLETE backup `backup_task` as the new keyspace
/// `name` on `voters` (3, 5 or 7 stores for every target), in the serialized,
/// term-fenced catalog transaction. The targets are created and activated here;
/// the keyspace id is reserved here and published later. A live restore of the
/// same backup under the same name confirms.
pub fn plan_restore<E: Engine>(
    txn: &mut MetaTxn<'_, E>,
    backup_task: u64,
    name: &str,
    tenant: TenantId,
    voters: &[NodeId],
    now_unix: u64,
) -> Result<(RestoreIntent, bool)> {
    if name.is_empty() || name.len() > MAX_NAME_BYTES {
        return Err(invalid("restore keyspace name must contain 1..1024 bytes"));
    }
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("restore requires a certified metadata root"))?
        .digest();
    let backups = super::backup::committed_backups_in_txn(txn)?;
    let backup = backups
        .intents
        .iter()
        .find(|b| b.task() == backup_task)
        .ok_or_else(|| invalid("restore names a task that is not a backup"))?;
    match backups.stage(backup) {
        super::backup::BackupStage::Complete => {}
        other => return Err(invalid(&format!("the backup is {}; only a complete backup restores", other.label()))),
    }
    let restores = committed_restores_in_txn(txn)?;
    let mut sequence = 0u64;
    for previous in restores.intents.iter().filter(|r| r.backup_task == backup_task && r.name == name) {
        if restores.stage(previous).live() {
            return Ok((previous.clone(), false));
        }
        sequence = sequence.max(previous.sequence + 1);
    }
    if keyspace_name_taken(txn, name)? || restores.intents.iter().any(|r| r.name == name && restores.stage(r).live()) {
        return Err(invalid(&format!("a keyspace named {name:?} exists or is being restored; restore under another name")));
    }
    let bindings = super::ranges::read_in(txn)?;
    let operation = restore_operation(root, backup.operation(), name, sequence);
    let keyspace_id = txn.allocate_id(SequenceKind::Keyspace)?;
    if keyspace_id == 0 || keyspace_id > u64::from(KeyspaceId::MAX) {
        return Err(invalid("keyspace sequence exhausted"));
    }
    let mut targets = Vec::with_capacity(backup.ranges().len());
    for range in backup.ranges() {
        let anchor = backups
            .anchor_of_range(range)
            .ok_or_else(|| invalid("a complete backup lost an anchor"))?;
        if !anchor.restorable() {
            return Err(invalid(&format!(
                "range {} of backup {} is not restorable: its image exceeded the image budget (restore needs the streamed path of increment 4)",
                range.region().0,
                backup_task
            )));
        }
        let binding = bindings
            .iter()
            .find(|b| b.bind_task() == range.bind_task())
            .ok_or_else(|| invalid("a backed-up range's binding row is missing"))?;
        let creation = super::plan_empty_group(txn, target_operation(operation, range.region()), voters)?;
        super::activation::plan_activation(txn, &creation)?;
        targets.push(RestoreTarget {
            source_region: range.region(),
            source_operation: range.operation(),
            creation_task: creation.task(),
            region: creation.region(),
            start: binding.range().start.clone(),
            end: binding.range().end.clone(),
        });
    }
    targets.sort_by(|a, b| a.start.cmp(&b.start));
    let intent = RestoreIntent {
        root,
        operation,
        task: txn.allocate_id(SequenceKind::Task)?,
        backup_task,
        sequence,
        backup_operation: backup.operation(),
        keyspace: KeyspaceId(keyspace_id as u32),
        tenant,
        requested_unix: now_unix,
        name: name.to_owned(),
        targets,
    };
    RestoreIntent::decode(&intent.encode())?;
    insert_row(txn, intent.task, RESTORE_INTENT, intent.encode())?;
    Ok((intent, true))
}

/// Commit one target's evidence from its leader's receipt: only for a live
/// restore, only when rows and digest equal the source range's anchor (B5).
pub fn plan_restore_evidence<E: Engine>(txn: &mut MetaTxn<'_, E>, receipt: &[u8]) -> Result<(RestoreEvidence, bool)> {
    let receipt = RestoreEvidence::decode_receipt(receipt)?;
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("restore evidence requires a certified metadata root"))?
        .digest();
    if receipt.root != root {
        return Err(invalid("restore evidence root differs from the certified root"));
    }
    let rows = committed_restores_in_txn(txn)?;
    let intent = rows
        .intent_by_operation(receipt.operation)
        .ok_or_else(|| invalid("restore evidence requires the committed intent"))?;
    if !rows.stage(intent).live() {
        return Err(invalid(&format!("the restore is {}; its evidence is refused", rows.stage(intent).label())));
    }
    let target = intent
        .targets
        .iter()
        .find(|t| t.creation_task == receipt.target_task && t.source_region == receipt.source_region)
        .ok_or_else(|| invalid("restore evidence names a target its intent does not"))?;
    let backups = super::backup::committed_backups_in_txn(txn)?;
    let anchor = backups
        .anchors
        .iter()
        .find(|a| a.range_operation() == target.source_operation)
        .ok_or_else(|| invalid("restore evidence needs the source range's anchor"))?;
    if anchor.rows() != receipt.rows || anchor.rows_digest() != receipt.rows_digest {
        return Err(invalid(&format!(
            "restored rows differ from the anchor ({} rows vs {}); the target is not published",
            receipt.rows,
            anchor.rows()
        )));
    }
    if let Some(previous) = rows.evidence_of(intent, target) {
        return Ok((previous.clone(), false));
    }
    let evidence = RestoreEvidence { task: txn.allocate_id(SequenceKind::Task)?, ..receipt };
    RestoreEvidence::decode(&evidence.encode())?;
    insert_row(txn, evidence.task, RESTORE_EVIDENCE, evidence.encode())?;
    Ok((evidence, true))
}

/// Publish the restored keyspace (B4): one transaction inserts the keyspace row
/// under the reserved id, one REGIONS row and one binding per target. Requires
/// every target's evidence. Idempotent: a published restore confirms.
pub fn plan_restore_publication<E: Engine>(txn: &mut MetaTxn<'_, E>, operation: [u8; 16]) -> Result<(RestoreIntent, bool)> {
    use crate::schema::{KEYSPACES_DESC, REGIONS_DESC};
    use crate::tables::Keyspace;
    use crate::data_groups::ranges::DATA_KEYSPACE_CONFIG;
    use kv9_common::ApiType;
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("restore publication requires a certified metadata root"))?
        .digest();
    let rows = committed_restores_in_txn(txn)?;
    let intent = rows
        .intent_by_operation(operation)
        .ok_or_else(|| invalid("restore publication requires the committed intent"))?
        .clone();
    match rows.stage(&intent) {
        RestoreStage::Published => return Ok((intent, false)),
        RestoreStage::Aborted => return Err(invalid("the restore is aborted; nothing is published")),
        RestoreStage::Populated => {}
        other => return Err(invalid(&format!("the restore is {}; publication needs every target's evidence", other.label()))),
    }
    if keyspace_name_taken(txn, &intent.name)? {
        return Err(invalid(&format!("a keyspace named {:?} appeared; the restore cannot publish", intent.name)));
    }
    let ks = Keyspace {
        id: intent.keyspace,
        name: intent.name.clone(),
        tenant_id: intent.tenant,
        api_type: ApiType::Raw,
        start_key: Vec::new(),
        end_key: Vec::new(),
        state: 0,
        config: DATA_KEYSPACE_CONFIG.to_vec(),
    };
    txn.insert(&KEYSPACES_DESC, &ks.pk(), ks.to_row_value())?;
    for target in &intent.targets {
        let creation = txn
            .get(&TASKS_DESC, &[memcmp_uint(target.creation_task)])?
            .as_ref()
            .map(|r| from_row(r, root))
            .transpose()?
            .flatten()
            .ok_or_else(|| invalid("restore target creation is missing"))?;
        let range = kv9_common::data_range::DataRange {
            root,
            creation: creation.digest(),
            region: creation.region(),
            keyspace: intent.keyspace,
            tenant: intent.tenant,
            conf_ver: 1,
            version: 1,
            start: target.start.clone(),
            end: target.end.clone(),
            sealed: false,
        };
        range.validate()?;
        let mut region = RowValue::new();
        for (c, v) in [
            (1, ColumnValue::Uint(range.region.0)),
            (2, ColumnValue::Uint(u64::from(range.keyspace.0))),
            (3, ColumnValue::Bytes(range.start.clone())),
            (4, ColumnValue::Bytes(range.end.clone())),
            (5, ColumnValue::Uint(1)),
            (6, ColumnValue::Uint(1)),
            (7, ColumnValue::Uint(0)),
        ] {
            region.set(ColumnId(c), v);
        }
        txn.insert(&REGIONS_DESC, &[memcmp_uint(range.region.0)], region)?;
        let bind_task = txn.allocate_id(SequenceKind::Task)?;
        let mut payload = target.creation_task.to_be_bytes().to_vec();
        payload.extend_from_slice(&range.encode());
        let mut row = RowValue::new();
        for (c, v) in [
            (1, ColumnValue::Uint(bind_task)),
            (2, ColumnValue::Uint(102)),
            (3, ColumnValue::Bytes(payload)),
            (4, ColumnValue::Uint(0)),
            (5, ColumnValue::Uint(0)),
        ] {
            row.set(ColumnId(c), v);
        }
        txn.insert(&TASKS_DESC, &[memcmp_uint(bind_task)], row)?;
    }
    // The staged shape must read back as a legal partition.
    super::ranges::read_in(txn)?;
    Ok((intent, true))
}

/// The descriptor a target carries before publication: the reserved keyspace
/// over its boundaries (the restore counterpart of a split child's half).
pub fn target_descriptor(intent: &RestoreIntent, target: &RestoreTarget, creation: &CreationIntent) -> kv9_common::data_range::DataRange {
    kv9_common::data_range::DataRange {
        root: intent.root,
        creation: creation.digest(),
        region: creation.region(),
        keyspace: intent.keyspace,
        tenant: intent.tenant,
        conf_ver: 1,
        version: 1,
        start: target.start.clone(),
        end: target.end.clone(),
        sealed: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kv9_common::AppliedPosition;

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

    /// A complete backup of the fixture's keyspace with one restorable anchor.
    fn complete_backup(store: &MetaStore<kv9_engine::MemEngine>, keyspace: KeyspaceId, rows: u64, digest: RootDigest) -> u64 {
        let mut txn = store.begin().unwrap();
        let (intent, _) = super::super::backup::plan_backup(&mut txn, keyspace, "nightly", 1).unwrap();
        let receipt = super::super::backup::BackupAnchor::receipt(
            &intent,
            &intent.ranges()[0],
            AppliedPosition { term: 2, index: 40 },
            None,
            RootDigest::sha256(b"image"),
            RootDigest::sha256(b"manifest"),
            1,
            4096,
            Some((rows, digest)),
            [9; 16],
        )
        .unwrap();
        super::super::backup::plan_backup_anchor(&mut txn, &receipt).unwrap();
        super::super::backup::plan_backup_completion(&mut txn, intent.task(), "backups/c/t/bundle", RootDigest::sha256(b"b"), 100, 2).unwrap();
        txn.commit().unwrap();
        intent.task()
    }

    #[test]
    fn a_restore_creates_targets_verifies_rows_publishes_once_and_never_touches_the_source() {
        let store = super::super::tests::fixture();
        let (keyspace, _region, _creation) = bound_keyspace(&store);
        let digest = RootDigest::sha256(b"rows");
        let backup = complete_backup(&store, keyspace, 150, digest);
        let voters = [NodeId(1), NodeId(2), NodeId(3)];
        let mut txn = store.begin().unwrap();
        assert!(plan_restore(&mut txn, backup, "orders", TenantId(1), &voters, 3).is_err(), "the source name is taken");
        assert!(plan_restore(&mut txn, 999, "x", TenantId(1), &voters, 3).is_err(), "not a backup");
        let (intent, changed) = plan_restore(&mut txn, backup, "orders-restored", TenantId(1), &voters, 3).unwrap();
        assert!(changed && intent.targets().len() == 1 && intent.keyspace() != keyspace);
        let target = intent.targets()[0].clone();
        assert_eq!(target.start(), b"");
        assert_eq!(target.end(), b"");
        txn.commit().unwrap();
        // The live restore confirms; a second name is refused while live? No: another
        // name is another restore; the same name confirms.
        let mut txn = store.begin().unwrap();
        let (again, changed) = plan_restore(&mut txn, backup, "orders-restored", TenantId(1), &voters, 4).unwrap();
        assert!(!changed && again == intent);
        // The backup cannot retire while a restore of it is live.
        assert!(super::super::backup::plan_backup_retirement(&mut txn, backup, 5).is_err());
        // Publication and wrong evidence are refused; the source keyspace is untouched.
        assert!(plan_restore_publication(&mut txn, intent.operation()).is_err(), "no evidence yet");
        let wrong = RestoreEvidence::receipt(&intent, &target, 149, digest).unwrap();
        assert!(plan_restore_evidence(&mut txn, &wrong).is_err(), "rows differ from the anchor");
        let wrong = RestoreEvidence::receipt(&intent, &target, 150, RootDigest::sha256(b"other")).unwrap();
        assert!(plan_restore_evidence(&mut txn, &wrong).is_err(), "digest differs from the anchor");
        let receipt = RestoreEvidence::receipt(&intent, &target, 150, digest).unwrap();
        assert!(is_restore_receipt(&receipt));
        let (evidence, changed) = plan_restore_evidence(&mut txn, &receipt).unwrap();
        assert!(changed && evidence.rows() == 150);
        let (_, changed) = plan_restore_evidence(&mut txn, &receipt).unwrap();
        assert!(!changed);
        txn.commit().unwrap();
        let rows = committed_restores(&store).unwrap();
        assert_eq!(rows.stage(&intent), RestoreStage::Populated);
        let mut txn = store.begin().unwrap();
        let (_, changed) = plan_restore_publication(&mut txn, intent.operation()).unwrap();
        assert!(changed);
        let (_, again) = plan_restore_publication(&mut txn, intent.operation()).unwrap();
        assert!(!again, "a published restore confirms");
        txn.commit().unwrap();
        let rows = committed_restores(&store).unwrap();
        assert_eq!(rows.stage(&intent), RestoreStage::Published);
        let txn = store.begin().unwrap();
        let bindings = super::super::ranges::read_in(&txn).unwrap();
        let restored: Vec<_> = bindings.iter().filter(|b| b.range().keyspace == intent.keyspace()).collect();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].creation().intent().task(), target.creation_task());
        assert_eq!(bindings.iter().filter(|b| b.range().keyspace == keyspace).count(), 1, "the source keyspace keeps its one binding");
        let ks = crate::tables::Tables::<kv9_engine::MemEngine>::keyspace_in(&txn, intent.keyspace()).unwrap().unwrap();
        assert_eq!(ks.name, "orders-restored");
        drop(txn);
        // Once published the name is taken for good; the backup may retire now.
        let mut txn = store.begin().unwrap();
        assert!(plan_restore(&mut txn, backup, "orders-restored", TenantId(1), &voters, 6).is_err());
        assert!(super::super::backup::plan_backup_retirement(&mut txn, backup, 7).is_ok());
        assert!(super::super::range_abort::plan_range_abort(&mut txn, intent.task()).is_err(), "a published restore does not abort");
        // A tampered intent is refused by decode.
        let mut bytes = intent.encode();
        bytes[40] ^= 1;
        assert!(RestoreIntent::decode(&bytes).is_err());
    }

    #[test]
    fn an_aborted_restore_is_history_and_frees_the_name() {
        let store = super::super::tests::fixture();
        let (keyspace, _, _) = bound_keyspace(&store);
        let backup = complete_backup(&store, keyspace, 1, RootDigest::sha256(b"r"));
        let voters = [NodeId(1), NodeId(2), NodeId(3)];
        let mut txn = store.begin().unwrap();
        let (intent, _) = plan_restore(&mut txn, backup, "copy", TenantId(1), &voters, 3).unwrap();
        let (abort, changed) = super::super::range_abort::plan_range_abort(&mut txn, intent.task()).unwrap();
        assert!(changed && abort.target() == super::super::range_abort::AbortTarget::Restore);
        txn.commit().unwrap();
        let rows = committed_restores(&store).unwrap();
        assert_eq!(rows.stage(&intent), RestoreStage::Aborted);
        let mut txn = store.begin().unwrap();
        assert!(plan_restore_evidence(&mut txn, &RestoreEvidence::receipt(&intent, &intent.targets()[0], 1, RootDigest::sha256(b"r")).unwrap()).is_err());
        let (second, changed) = plan_restore(&mut txn, backup, "copy", TenantId(1), &voters, 4).unwrap();
        assert!(changed && second.sequence() == 1 && second.operation() != intent.operation());
    }
}
