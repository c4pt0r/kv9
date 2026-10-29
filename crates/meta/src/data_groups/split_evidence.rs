//! Committed split population evidence (docs/RANGE-SPLIT.md §4): a child's
//! leader copied its half of the SEALED parent into the child and verified, on
//! the same node, that the child's rows equal the parent half row for row. One
//! immutable row per (split operation, child); an identical resubmission
//! confirms; a different digest for the same child refuses — a child is never
//! re-evidenced with other contents. Publication (§5) requires both rows.

use super::*;

const SPLIT_EVIDENCE: u64 = 114;
const SPE_MAGIC: &[u8; 8] = b"KV9SPE01";
pub const MAX_SPLIT_EVIDENCE_BYTES: usize = 8 + 32 + 16 + 8 + 8 + 8 + 32 + 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitEvidence {
    root: RootDigest,
    operation: [u8; 16],
    task: u64,
    intent_task: u64,
    child_task: u64,
    digest: RootDigest,
    rows: u64,
}

impl SplitEvidence {
    pub fn root(&self) -> RootDigest {
        self.root
    }
    pub fn operation(&self) -> [u8; 16] {
        self.operation
    }
    pub fn task(&self) -> u64 {
        self.task
    }
    pub fn intent_task(&self) -> u64 {
        self.intent_task
    }
    pub fn child_task(&self) -> u64 {
        self.child_task
    }
    /// Digest of the child's rows, equal to the sealed parent half's.
    pub fn digest(&self) -> RootDigest {
        self.digest
    }
    pub fn rows(&self) -> u64 {
        self.rows
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(MAX_SPLIT_EVIDENCE_BYTES);
        bytes.extend_from_slice(SPE_MAGIC);
        bytes.extend_from_slice(self.root.as_bytes());
        bytes.extend_from_slice(&self.operation);
        bytes.extend_from_slice(&self.task.to_be_bytes());
        bytes.extend_from_slice(&self.intent_task.to_be_bytes());
        bytes.extend_from_slice(&self.child_task.to_be_bytes());
        bytes.extend_from_slice(self.digest.as_bytes());
        bytes.extend_from_slice(&self.rows.to_be_bytes());
        bytes
    }

    fn decode_inner(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != MAX_SPLIT_EVIDENCE_BYTES || &bytes[..8] != SPE_MAGIC {
            return Err(invalid("invalid split evidence encoding"));
        }
        let evidence = Self {
            root: RootDigest::from_bytes(bytes[8..40].try_into().unwrap()),
            operation: bytes[40..56].try_into().unwrap(),
            task: u64::from_be_bytes(bytes[56..64].try_into().unwrap()),
            intent_task: u64::from_be_bytes(bytes[64..72].try_into().unwrap()),
            child_task: u64::from_be_bytes(bytes[72..80].try_into().unwrap()),
            digest: RootDigest::from_bytes(bytes[80..112].try_into().unwrap()),
            rows: u64::from_be_bytes(bytes[112..120].try_into().unwrap()),
        };
        if evidence.root.as_bytes() == &[0; 32]
            || evidence.operation == [0; 16]
            || evidence.intent_task < FIRST_DYNAMIC_ID
            || evidence.child_task < FIRST_DYNAMIC_ID
            || evidence.intent_task == evidence.child_task
            || evidence.digest.as_bytes() == &[0; 32]
        {
            return Err(invalid("invalid split evidence identities"));
        }
        Ok(evidence)
    }

    /// Decode a committed row (task allocated).
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let evidence = Self::decode_inner(bytes)?;
        if evidence.task < FIRST_DYNAMIC_ID || evidence.task == evidence.intent_task || evidence.task == evidence.child_task {
            return Err(invalid("invalid split evidence task"));
        }
        Ok(evidence)
    }

    /// Decode a child-emitted receipt: the same layout with the task still zero.
    pub fn decode_receipt(bytes: &[u8]) -> Result<Self> {
        let receipt = Self::decode_inner(bytes)?;
        if receipt.task != 0 {
            return Err(invalid("a split evidence receipt cannot supply its own task"));
        }
        Ok(receipt)
    }

    /// Canonical receipt bytes a child leader emits after its local verification.
    pub fn receipt(intent: &super::split::SplitIntent, child_task: u64, digest: RootDigest, rows: u64) -> Result<Vec<u8>> {
        let receipt = Self {
            root: intent.root(),
            operation: intent.operation(),
            task: 0,
            intent_task: intent.task(),
            child_task,
            digest,
            rows,
        };
        let bytes = receipt.encode();
        Self::decode_receipt(&bytes)?;
        Ok(bytes)
    }
}

pub(crate) fn from_split_evidence_row(row: &Row, root: RootDigest) -> Result<Option<SplitEvidence>> {
    if row.value.get(ColumnId(2)) != Some(&ColumnValue::Uint(SPLIT_EVIDENCE)) {
        return Ok(None);
    }
    let Some(ColumnValue::Bytes(bytes)) = row.value.get(ColumnId(3)) else {
        return Err(invalid("split evidence payload is missing"));
    };
    let evidence = SplitEvidence::decode(bytes)?;
    if evidence.root != root
        || row.pk != [memcmp_uint(evidence.task)]
        || row.value.get(ColumnId(1)) != Some(&ColumnValue::Uint(evidence.task))
        || row.value.get(ColumnId(4)) != Some(&ColumnValue::Uint(0))
        || row.value.get(ColumnId(5)) != Some(&ColumnValue::Uint(0))
    {
        return Err(invalid("split evidence row binding differs"));
    }
    Ok(Some(evidence))
}

fn matches_intent(evidence: &SplitEvidence, intent: &super::split::SplitIntent) -> Result<()> {
    if intent.task() != evidence.intent_task
        || intent.operation() != evidence.operation
        || intent.root() != evidence.root
        || ![intent.child_low(), intent.child_high()].contains(&evidence.child_task)
    {
        return Err(invalid("split evidence differs from the committed intent"));
    }
    Ok(())
}

/// Stage one idempotent evidence row in the serialized, term-fenced catalog
/// transaction. `false` means the exact row already exists. A row for the same
/// child with another digest refuses permanently.
pub fn plan_split_evidence<E: Engine>(txn: &mut MetaTxn<'_, E>, receipt: &[u8]) -> Result<(SplitEvidence, bool)> {
    let receipt = SplitEvidence::decode_receipt(receipt)?;
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("split evidence requires a certified metadata root"))?;
    if receipt.root != root.digest() {
        return Err(invalid("split evidence root differs from the certified root"));
    }
    let intent = super::split::committed_splits_in_txn(txn)?
        .into_iter()
        .find(|s| s.operation() == receipt.operation)
        .ok_or_else(|| invalid("split evidence requires the committed intent"))?;
    if super::range_abort::effective_aborts_in_txn(txn, root.digest())?.contains(&receipt.operation) {
        return Err(invalid("the split intent is aborted; its evidence is refused"));
    }
    matches_intent(&receipt, &intent)?;
    for row in task_rows(txn) {
        if let Some(previous) = from_split_evidence_row(&row?, root.digest())? {
            if previous.operation == receipt.operation && previous.child_task == receipt.child_task {
                let expected = SplitEvidence {
                    task: previous.task,
                    ..receipt.clone()
                };
                if expected != previous {
                    return Err(invalid("split evidence for this child conflicts"));
                }
                return Ok((previous, false));
            }
        }
    }
    let evidence = SplitEvidence {
        task: txn.allocate_id(SequenceKind::Task)?,
        ..receipt
    };
    SplitEvidence::decode(&evidence.encode())?;
    let mut row = RowValue::new();
    for (column, value) in [
        (ColumnId(1), ColumnValue::Uint(evidence.task)),
        (ColumnId(2), ColumnValue::Uint(SPLIT_EVIDENCE)),
        (ColumnId(3), ColumnValue::Bytes(evidence.encode())),
        (ColumnId(4), ColumnValue::Uint(0)),
        (ColumnId(5), ColumnValue::Uint(0)),
    ] {
        row.set(column, value);
    }
    txn.insert(&TASKS_DESC, &[memcmp_uint(evidence.task)], row)?;
    Ok((evidence, true))
}

/// Every committed evidence row, re-validated against its intent.
pub fn committed_split_evidence<E: Engine>(store: &MetaStore<E>) -> Result<Vec<SplitEvidence>> {
    let txn = store.begin()?;
    committed_split_evidence_in_txn(&txn)
}

pub fn committed_split_evidence_in_txn<E: Engine>(txn: &MetaTxn<'_, E>) -> Result<Vec<SplitEvidence>> {
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("split evidence readback requires a certified metadata root"))?;
    let intents = super::split::committed_splits_in_txn(txn)?;
    let mut out: Vec<SplitEvidence> = Vec::new();
    for row in task_rows(txn) {
        if let Some(evidence) = from_split_evidence_row(&row?, root.digest())? {
            let intent = intents
                .iter()
                .find(|s| s.operation() == evidence.operation)
                .ok_or_else(|| invalid("split evidence requires the committed intent"))?;
            matches_intent(&evidence, intent)?;
            if out.iter().any(|prior| {
                prior.task == evidence.task
                    || (prior.operation == evidence.operation && prior.child_task == evidence.child_task)
            }) {
                return Err(invalid("duplicate split evidence identity"));
            }
            out.push(evidence);
        }
    }
    Ok(out)
}
