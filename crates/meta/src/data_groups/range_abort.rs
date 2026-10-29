//! Aborting a long-running intent (docs/RANGE-SPLIT.md §12): one immutable row
//! (kind 117, `KV9ROA01`) says that a split intent, a merge intent or a node
//! drain is abandoned. Replica plans abort through the migration abort (kind
//! 108) they already have.
//!
//! A split or merge becomes **irrevocable at its first seal**: the parent's
//! leader commits seal evidence (kind 118, `KV9SLE01`) right after sealing, and
//! the abort planner refuses once any exists. The one race left — an abort
//! committed between the seal and its evidence — is decided by the catalog
//! alone: an abort is *effective* iff no seal evidence names its operation, and
//! every reader (drivers, readbacks, listings) applies that rule, so a late
//! abort is visibly ignored and the split completes.

use super::*;

const RANGE_ABORT: u64 = 117;
const SEAL_EVIDENCE: u64 = 118;
const ROA_MAGIC: &[u8; 8] = b"KV9ROA01";
const SLE_MAGIC: &[u8; 8] = b"KV9SLE01";
pub const MAX_RANGE_ABORT_BYTES: usize = 8 + 32 + 16 + 8 + 8 + 1;
pub const MAX_SEAL_EVIDENCE_BYTES: usize = 8 + 32 + 16 + 8 + 8 + 8 + 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbortTarget {
    Split = 1,
    Merge = 2,
    Drain = 3,
}

impl AbortTarget {
    fn from_code(code: u8) -> Result<Self> {
        Ok(match code {
            1 => Self::Split,
            2 => Self::Merge,
            3 => Self::Drain,
            _ => return Err(invalid("invalid abort target")),
        })
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Split => "split",
            Self::Merge => "merge",
            Self::Drain => "drain",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeAbort {
    root: RootDigest,
    operation: [u8; 16],
    task: u64,
    target_task: u64,
    target: AbortTarget,
}

impl RangeAbort {
    pub fn operation(&self) -> [u8; 16] {
        self.operation
    }
    pub fn task(&self) -> u64 {
        self.task
    }
    pub fn target_task(&self) -> u64 {
        self.target_task
    }
    pub fn target(&self) -> AbortTarget {
        self.target
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(MAX_RANGE_ABORT_BYTES);
        bytes.extend_from_slice(ROA_MAGIC);
        bytes.extend_from_slice(self.root.as_bytes());
        bytes.extend_from_slice(&self.operation);
        bytes.extend_from_slice(&self.task.to_be_bytes());
        bytes.extend_from_slice(&self.target_task.to_be_bytes());
        bytes.push(self.target as u8);
        bytes
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != MAX_RANGE_ABORT_BYTES || &bytes[..8] != ROA_MAGIC {
            return Err(invalid("invalid range abort encoding"));
        }
        let a = Self {
            root: RootDigest::from_bytes(bytes[8..40].try_into().unwrap()),
            operation: bytes[40..56].try_into().unwrap(),
            task: u64::from_be_bytes(bytes[56..64].try_into().unwrap()),
            target_task: u64::from_be_bytes(bytes[64..72].try_into().unwrap()),
            target: AbortTarget::from_code(bytes[72])?,
        };
        if a.root.as_bytes() == &[0; 32]
            || a.operation == [0; 16]
            || a.task < FIRST_DYNAMIC_ID
            || a.target_task < FIRST_DYNAMIC_ID
            || a.task == a.target_task
        {
            return Err(invalid("invalid range abort identities"));
        }
        Ok(a)
    }
}

pub(crate) fn from_abort_row(row: &Row, root: RootDigest) -> Result<Option<RangeAbort>> {
    if row.value.get(ColumnId(2)) != Some(&ColumnValue::Uint(RANGE_ABORT)) {
        return Ok(None);
    }
    let Some(ColumnValue::Bytes(bytes)) = row.value.get(ColumnId(3)) else {
        return Err(invalid("range abort payload is missing"));
    };
    let a = RangeAbort::decode(bytes)?;
    if a.root != root
        || row.pk != [memcmp_uint(a.task)]
        || row.value.get(ColumnId(1)) != Some(&ColumnValue::Uint(a.task))
        || row.value.get(ColumnId(4)) != Some(&ColumnValue::Uint(0))
        || row.value.get(ColumnId(5)) != Some(&ColumnValue::Uint(0))
    {
        return Err(invalid("range abort row binding differs"));
    }
    Ok(Some(a))
}

// ------------------------------------------------------------------ seal evidence

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealEvidence {
    root: RootDigest,
    operation: [u8; 16],
    task: u64,
    region: RegionId,
    term: u64,
    index: u64,
}

impl SealEvidence {
    pub fn operation(&self) -> [u8; 16] {
        self.operation
    }
    pub fn task(&self) -> u64 {
        self.task
    }
    pub fn region(&self) -> RegionId {
        self.region
    }
    pub fn position(&self) -> kv9_common::AppliedPosition {
        kv9_common::AppliedPosition { term: self.term, index: self.index }
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(MAX_SEAL_EVIDENCE_BYTES);
        bytes.extend_from_slice(SLE_MAGIC);
        bytes.extend_from_slice(self.root.as_bytes());
        bytes.extend_from_slice(&self.operation);
        for n in [self.task, self.region.0, self.term, self.index] {
            bytes.extend_from_slice(&n.to_be_bytes());
        }
        bytes
    }
    fn decode_inner(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != MAX_SEAL_EVIDENCE_BYTES || &bytes[..8] != SLE_MAGIC {
            return Err(invalid("invalid seal evidence encoding"));
        }
        let n = |i: usize| u64::from_be_bytes(bytes[56 + 8 * i..64 + 8 * i].try_into().unwrap());
        let e = Self {
            root: RootDigest::from_bytes(bytes[8..40].try_into().unwrap()),
            operation: bytes[40..56].try_into().unwrap(),
            task: n(0),
            region: RegionId(n(1)),
            term: n(2),
            index: n(3),
        };
        if e.root.as_bytes() == &[0; 32] || e.operation == [0; 16] || e.region.0 < FIRST_DYNAMIC_ID || e.index == 0 {
            return Err(invalid("invalid seal evidence identities"));
        }
        Ok(e)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let e = Self::decode_inner(bytes)?;
        if e.task < FIRST_DYNAMIC_ID {
            return Err(invalid("invalid seal evidence task"));
        }
        Ok(e)
    }
    pub fn decode_receipt(bytes: &[u8]) -> Result<Self> {
        let e = Self::decode_inner(bytes)?;
        if e.task != 0 {
            return Err(invalid("a seal evidence receipt cannot supply its own task"));
        }
        Ok(e)
    }
    /// The parent leader's receipt right after `region` applied its seal.
    pub fn receipt(root: RootDigest, operation: [u8; 16], region: RegionId, position: kv9_common::AppliedPosition) -> Result<Vec<u8>> {
        let e = Self { root, operation, task: 0, region, term: position.term, index: position.index };
        let bytes = e.encode();
        Self::decode_receipt(&bytes)?;
        Ok(bytes)
    }
}

pub fn is_seal_receipt(bytes: &[u8]) -> bool {
    bytes.len() >= 8 && &bytes[..8] == SLE_MAGIC
}

pub(crate) fn from_seal_row(row: &Row, root: RootDigest) -> Result<Option<SealEvidence>> {
    if row.value.get(ColumnId(2)) != Some(&ColumnValue::Uint(SEAL_EVIDENCE)) {
        return Ok(None);
    }
    let Some(ColumnValue::Bytes(bytes)) = row.value.get(ColumnId(3)) else {
        return Err(invalid("seal evidence payload is missing"));
    };
    let e = SealEvidence::decode(bytes)?;
    if e.root != root
        || row.pk != [memcmp_uint(e.task)]
        || row.value.get(ColumnId(1)) != Some(&ColumnValue::Uint(e.task))
        || row.value.get(ColumnId(4)) != Some(&ColumnValue::Uint(0))
        || row.value.get(ColumnId(5)) != Some(&ColumnValue::Uint(0))
    {
        return Err(invalid("seal evidence row binding differs"));
    }
    Ok(Some(e))
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

/// Commit seal evidence from a parent leader's receipt: the operation must be a
/// committed split or merge intent and `region` one of its parents. Idempotent per
/// `(operation, region)`.
pub fn plan_seal_evidence<E: Engine>(txn: &mut MetaTxn<'_, E>, receipt: &[u8]) -> Result<(SealEvidence, bool)> {
    let receipt = SealEvidence::decode_receipt(receipt)?;
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("seal evidence requires a certified metadata root"))?;
    if receipt.root != root.digest() {
        return Err(invalid("seal evidence root differs from the certified root"));
    }
    let parents: Vec<RegionId> = if let Some(s) = split_intent_for(txn, root.digest(), receipt.operation)? {
        vec![s.parent_region()]
    } else if let Some(m) = merge_intent_for(txn, root.digest(), receipt.operation)? {
        vec![m.low_region(), m.high_region()]
    } else {
        return Err(invalid("seal evidence requires a committed split or merge intent"));
    };
    if !parents.contains(&receipt.region) {
        return Err(invalid("seal evidence names a region that is not a parent of the operation"));
    }
    for row in task_rows(txn) {
        if let Some(previous) = from_seal_row(&row?, root.digest())? {
            if previous.operation == receipt.operation && previous.region == receipt.region {
                return Ok((previous, false));
            }
        }
    }
    let e = SealEvidence { task: txn.allocate_id(SequenceKind::Task)?, ..receipt };
    SealEvidence::decode(&e.encode())?;
    insert_row(txn, e.task, SEAL_EVIDENCE, e.encode())?;
    Ok((e, true))
}

fn split_intent_for<E: Engine>(txn: &MetaTxn<'_, E>, root: RootDigest, operation: [u8; 16]) -> Result<Option<split::SplitIntent>> {
    for row in task_rows(txn) {
        if let Some(s) = split::from_split_row(&row?, root)? {
            if s.operation() == operation {
                return Ok(Some(s));
            }
        }
    }
    Ok(None)
}

fn merge_intent_for<E: Engine>(txn: &MetaTxn<'_, E>, root: RootDigest, operation: [u8; 16]) -> Result<Option<merge::MergeIntent>> {
    for row in task_rows(txn) {
        if let Some(m) = merge::from_merge_row(&row?, root)? {
            if m.operation() == operation {
                return Ok(Some(m));
            }
        }
    }
    Ok(None)
}

/// The operations whose abort is EFFECTIVE: an abort row exists and no seal evidence
/// names the operation. Drains have no seal; their abort is always effective.
pub fn effective_aborts_in_txn<E: Engine>(txn: &MetaTxn<'_, E>, root: RootDigest) -> Result<std::collections::BTreeSet<[u8; 16]>> {
    let mut aborted = std::collections::BTreeSet::new();
    let mut sealed = std::collections::BTreeSet::new();
    for row in task_rows(txn) {
        let row = row?;
        if let Some(a) = from_abort_row(&row, root)? {
            aborted.insert(a.operation);
        }
        if let Some(e) = from_seal_row(&row, root)? {
            sealed.insert(e.operation);
        }
    }
    Ok(aborted.difference(&sealed).copied().collect())
}

/// [`effective_aborts_in_txn`] from one fresh snapshot.
pub fn effective_aborts<E: Engine>(store: &MetaStore<E>) -> Result<std::collections::BTreeSet<[u8; 16]>> {
    let txn = store.begin()?;
    let root = crate::root::certified_root(&txn)?
        .ok_or_else(|| invalid("abort readback requires a certified metadata root"))?;
    effective_aborts_in_txn(&txn, root.digest())
}

pub fn committed_range_aborts<E: Engine>(store: &MetaStore<E>) -> Result<Vec<RangeAbort>> {
    let txn = store.begin()?;
    let root = crate::root::certified_root(&txn)?
        .ok_or_else(|| invalid("abort readback requires a certified metadata root"))?;
    let mut out = Vec::new();
    for row in task_rows(&txn) {
        if let Some(a) = from_abort_row(&row?, root.digest())? {
            out.push(a);
        }
    }
    Ok(out)
}

pub fn committed_seal_evidence<E: Engine>(store: &MetaStore<E>) -> Result<Vec<SealEvidence>> {
    let txn = store.begin()?;
    let root = crate::root::certified_root(&txn)?
        .ok_or_else(|| invalid("seal evidence readback requires a certified metadata root"))?;
    let mut out = Vec::new();
    for row in task_rows(&txn) {
        if let Some(e) = from_seal_row(&row?, root.digest())? {
            out.push(e);
        }
    }
    Ok(out)
}

/// Stage the abort of the intent `target_task` names. Refuses a split or merge that
/// is published or has any seal evidence (irrevocable), and a drain that is not
/// committed. Idempotent per target.
pub fn plan_range_abort<E: Engine>(txn: &mut MetaTxn<'_, E>, target_task: u64) -> Result<(RangeAbort, bool)> {
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("abort requires a certified metadata root"))?;
    let row = txn
        .get(&TASKS_DESC, &[memcmp_uint(target_task)])?
        .ok_or_else(|| invalid("abort names a task that is not committed"))?;
    let (operation, target) = if let Some(s) = split::from_split_row(&row, root.digest())? {
        let sealed = ranges::read_in(txn)?.iter().any(|b| b.range().region == s.parent_region() && b.range().sealed);
        if sealed {
            return Err(invalid("the split is published; nothing to abort"));
        }
        (s.operation(), AbortTarget::Split)
    } else if let Some(m) = merge::from_merge_row(&row, root.digest())? {
        let bindings = ranges::read_in(txn)?;
        if bindings.iter().any(|b| (b.range().region == m.low_region() || b.range().region == m.high_region()) && b.range().sealed) {
            return Err(invalid("the merge is published; nothing to abort"));
        }
        (m.operation(), AbortTarget::Merge)
    } else if let Some(d) = drain::from_drain_row(&row, root.digest())? {
        (d.operation(), AbortTarget::Drain)
    } else {
        return Err(invalid("abort takes a split intent, a merge intent or a node drain (a replica plan aborts through its migration abort)"));
    };
    for other in task_rows(txn) {
        let other = other?;
        if from_seal_row(&other, root.digest())?.is_some_and(|e| e.operation == operation) {
            return Err(invalid("a parent is already sealed; the operation is irrevocable and will complete"));
        }
        if let Some(previous) = from_abort_row(&other, root.digest())? {
            if previous.operation == operation {
                return Ok((previous, false));
            }
        }
    }
    let abort = RangeAbort {
        root: root.digest(),
        operation,
        task: txn.allocate_id(SequenceKind::Task)?,
        target_task,
        target,
    };
    RangeAbort::decode(&abort.encode())?;
    insert_row(txn, abort.task, RANGE_ABORT, abort.encode())?;
    Ok((abort, true))
}
