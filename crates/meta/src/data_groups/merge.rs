//! Committed range merge (docs/RANGE-SPLIT.md §6): the split pipeline backwards.
//! One intent (kind 115, `KV9MRG01`) names two ADJACENT, unsealed bindings of
//! one keyspace on the same committed members — `low = [s, m)`, `high = [m, e)`
//! — and the one child `[s, e)` the same transaction creates and activates on
//! those members. The child's leader copies both sealed parents into the child
//! and commits evidence (kind 116, `KV9MRE01`, the split evidence's layout);
//! publication seals both parent bindings in place and binds the child.

use super::*;
use kv9_common::data_range::DataRange;

const MERGE_RANGE: u64 = 115;
const MERGE_EVIDENCE: u64 = 116;
const MRG_MAGIC: &[u8; 8] = b"KV9MRG01";
const MRE_MAGIC: &[u8; 8] = b"KV9MRE01";
pub const MAX_MERGE_BYTES: usize = 8 + 32 + 16 + 8 + 8 + 8 + 8 + 8 + 8;
pub const MAX_MERGE_EVIDENCE_BYTES: usize = 8 + 32 + 16 + 8 + 8 + 8 + 32 + 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeIntent {
    root: RootDigest,
    operation: [u8; 16],
    task: u64,
    low_bind_task: u64,
    low_region: RegionId,
    high_bind_task: u64,
    high_region: RegionId,
    child: u64,
}

impl MergeIntent {
    pub fn root(&self) -> RootDigest {
        self.root
    }
    pub fn operation(&self) -> [u8; 16] {
        self.operation
    }
    pub fn task(&self) -> u64 {
        self.task
    }
    pub fn low_region(&self) -> RegionId {
        self.low_region
    }
    pub fn high_region(&self) -> RegionId {
        self.high_region
    }
    pub fn low_bind_task(&self) -> u64 {
        self.low_bind_task
    }
    pub fn high_bind_task(&self) -> u64 {
        self.high_bind_task
    }
    /// The creation task of the merged child `[low.start, high.end)`.
    pub fn child(&self) -> u64 {
        self.child
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(MAX_MERGE_BYTES);
        bytes.extend_from_slice(MRG_MAGIC);
        bytes.extend_from_slice(self.root.as_bytes());
        bytes.extend_from_slice(&self.operation);
        for n in [
            self.task,
            self.low_bind_task,
            self.low_region.0,
            self.high_bind_task,
            self.high_region.0,
            self.child,
        ] {
            bytes.extend_from_slice(&n.to_be_bytes());
        }
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != MAX_MERGE_BYTES || &bytes[..8] != MRG_MAGIC {
            return Err(invalid("invalid merge intent encoding"));
        }
        let n = |i: usize| u64::from_be_bytes(bytes[56 + 8 * i..64 + 8 * i].try_into().unwrap());
        let intent = Self {
            root: RootDigest::from_bytes(bytes[8..40].try_into().unwrap()),
            operation: bytes[40..56].try_into().unwrap(),
            task: n(0),
            low_bind_task: n(1),
            low_region: RegionId(n(2)),
            high_bind_task: n(3),
            high_region: RegionId(n(4)),
            child: n(5),
        };
        if intent.root.as_bytes() == &[0; 32]
            || intent.operation == [0; 16]
            || [intent.task, intent.low_bind_task, intent.high_bind_task, intent.low_region.0, intent.high_region.0, intent.child]
                .iter()
                .any(|v| *v < FIRST_DYNAMIC_ID)
            || intent.low_region == intent.high_region
            || intent.low_bind_task == intent.high_bind_task
            || [intent.low_bind_task, intent.high_bind_task, intent.child].contains(&intent.task)
        {
            return Err(invalid("invalid merge intent identities"));
        }
        Ok(intent)
    }
}

pub fn child_operation(operation: [u8; 16]) -> [u8; 16] {
    let mut bytes = b"kv9-merge-child/v1".to_vec();
    bytes.extend_from_slice(&operation);
    let digest = RootDigest::sha256(&bytes);
    let mut out = [0u8; 16];
    out.copy_from_slice(&digest.as_bytes()[..16]);
    out
}

pub(crate) fn from_merge_row(row: &Row, root: RootDigest) -> Result<Option<MergeIntent>> {
    if row.value.get(ColumnId(2)) != Some(&ColumnValue::Uint(MERGE_RANGE)) {
        return Ok(None);
    }
    let Some(ColumnValue::Bytes(bytes)) = row.value.get(ColumnId(3)) else {
        return Err(invalid("merge intent payload is missing"));
    };
    let intent = MergeIntent::decode(bytes)?;
    if intent.root != root
        || row.pk != [memcmp_uint(intent.task)]
        || row.value.get(ColumnId(1)) != Some(&ColumnValue::Uint(intent.task))
        || row.value.get(ColumnId(4)) != Some(&ColumnValue::Uint(0))
        || row.value.get(ColumnId(5)) != Some(&ColumnValue::Uint(0))
    {
        return Err(invalid("merge intent row binding differs"));
    }
    Ok(Some(intent))
}

fn creation_of<E: Engine>(txn: &MetaTxn<'_, E>, root: RootDigest, task: u64) -> Result<CreationIntent> {
    let row = txn
        .get(&TASKS_DESC, &[memcmp_uint(task)])?
        .ok_or_else(|| invalid("merge child creation is missing"))?;
    from_row(&row, root)?.ok_or_else(|| invalid("merge child creation row differs"))
}

fn sorted_members(members: &[InitialReplica]) -> Vec<InitialReplica> {
    let mut out = members.to_vec();
    out.sort_by_key(|r| r.node.0);
    out
}

/// Planning and readback share this: both parents bound with these exact rows,
/// adjacent, one keyspace, same committed members; unsealed, the child is an
/// activated UNBOUND creation on those members; sealed (both), the child is
/// bound to exactly `[low.start, high.end)` — the intent's own published shape.
fn validate_against<E: Engine>(txn: &MetaTxn<'_, E>, root: RootDigest, intent: &MergeIntent) -> Result<()> {
    let bindings = super::ranges::read_in(txn)?;
    let find = |region: RegionId, task: u64| {
        bindings
            .iter()
            .find(|b| b.range().region == region)
            .filter(|b| b.bind_task() == task)
            .ok_or_else(|| invalid("merge requires its exact parent bindings"))
    };
    let low = find(intent.low_region, intent.low_bind_task)?;
    let high = find(intent.high_region, intent.high_bind_task)?;
    let (lr, hr) = (low.range(), high.range());
    if lr.keyspace != hr.keyspace || lr.end.is_empty() || lr.end != hr.start {
        return Err(invalid("merge parents must be adjacent ranges of one keyspace"));
    }
    let child = creation_of(txn, root, intent.child)?;
    if lr.sealed || hr.sealed {
        let published = lr.sealed
            && hr.sealed
            && bindings.iter().any(|b| {
                b.range().region == child.region() && b.range().start == lr.start && b.range().end == hr.end
            });
        if !published {
            return Err(invalid("a merge parent is sealed outside the merge's own publication"));
        }
        return Ok(());
    }
    let low_members = sorted_members(
        &super::committed_members(txn, root, low.creation().intent())?
            .into_iter()
            .map(|(m, _)| m)
            .collect::<Vec<_>>(),
    );
    let high_members = sorted_members(
        &super::committed_members(txn, root, high.creation().intent())?
            .into_iter()
            .map(|(m, _)| m)
            .collect::<Vec<_>>(),
    );
    if low_members != high_members {
        return Err(invalid("merge parents must live on the same committed members"));
    }
    if sorted_members(child.replicas()) != low_members {
        return Err(invalid("merge child must live on the parents' committed members"));
    }
    if !super::activation::decode(txn, root)?.contains(&child) {
        return Err(invalid("merge child requires committed activation"));
    }
    if bindings.iter().any(|b| b.range().region == child.region()) {
        return Err(invalid("merge child must be an unbound empty group"));
    }
    Ok(())
}

/// Stage one idempotent merge intent and its child in the serialized,
/// term-fenced catalog transaction. `false` confirms the exact intent.
pub fn plan_merge<E: Engine>(
    txn: &mut MetaTxn<'_, E>,
    operation: [u8; 16],
    low_region: RegionId,
    high_region: RegionId,
) -> Result<(MergeIntent, CreationIntent, bool)> {
    if operation == [0; 16] {
        return Err(invalid("merge needs a nonzero operation"));
    }
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("merge requires a certified metadata root"))?;
    let bindings = super::ranges::read_in(txn)?;
    let binding = |region: RegionId| {
        bindings
            .iter()
            .find(|b| b.range().region == region)
            .ok_or_else(|| invalid("merge requires the parents' committed range bindings"))
    };
    let (low, high) = (binding(low_region)?, binding(high_region)?);
    let aborted = super::range_abort::effective_aborts_in_txn(txn, root.digest())?;
    for row in task_rows(txn) {
        let row = row?;
        if let Some(previous) = from_merge_row(&row, root.digest())? {
            if aborted.contains(&previous.operation) {
                continue;
            }
            if previous.operation == operation {
                if previous.low_region != low_region || previous.high_region != high_region {
                    return Err(invalid("merge operation binding conflicts"));
                }
                let child = creation_of(txn, root.digest(), previous.child)?;
                return Ok((previous, child, false));
            }
            if [previous.low_region, previous.high_region].iter().any(|r| *r == low_region || *r == high_region)
                && !(bindings.iter().any(|b| b.range().region == previous.low_region && b.range().sealed))
            {
                return Err(invalid("a committed merge already binds one of these parents"));
            }
        }
        if let Some(split) = super::split::from_split_row(&row, root.digest())? {
            if (split.parent_region() == low_region || split.parent_region() == high_region)
                && !aborted.contains(&split.operation())
                && !bindings.iter().any(|b| b.range().region == split.parent_region() && b.range().sealed)
            {
                return Err(invalid("a parent has a live split intent"));
            }
        }
    }
    if low.range().sealed || high.range().sealed {
        return Err(invalid("a merge parent is already sealed"));
    }
    if super::live_plan_on_region(txn, root.digest(), low_region)? || super::live_plan_on_region(txn, root.digest(), high_region)? {
        return Err(invalid("a parent has an unsettled replica plan; merge after it settles"));
    }
    if low.range().keyspace != high.range().keyspace || low.range().end.is_empty() || low.range().end != high.range().start {
        return Err(invalid("merge parents must be adjacent ranges of one keyspace"));
    }
    let members: Vec<NodeId> = super::committed_members(txn, root.digest(), low.creation().intent())?
        .into_iter()
        .map(|(m, _)| m.node)
        .collect();
    let child = super::plan_empty_group(txn, child_operation(operation), &members)?;
    super::activation::plan_activation(txn, &child)?;
    let intent = MergeIntent {
        root: root.digest(),
        operation,
        task: txn.allocate_id(SequenceKind::Task)?,
        low_bind_task: low.bind_task(),
        low_region,
        high_bind_task: high.bind_task(),
        high_region,
        child: child.task(),
    };
    MergeIntent::decode(&intent.encode())?;
    validate_against(txn, root.digest(), &intent)?;
    let mut row = RowValue::new();
    for (column, value) in [
        (ColumnId(1), ColumnValue::Uint(intent.task)),
        (ColumnId(2), ColumnValue::Uint(MERGE_RANGE)),
        (ColumnId(3), ColumnValue::Bytes(intent.encode())),
        (ColumnId(4), ColumnValue::Uint(0)),
        (ColumnId(5), ColumnValue::Uint(0)),
    ] {
        row.set(column, value);
    }
    txn.insert(&TASKS_DESC, &[memcmp_uint(intent.task)], row)?;
    Ok((intent, child, true))
}

pub fn committed_merges<E: Engine>(store: &MetaStore<E>) -> Result<Vec<MergeIntent>> {
    committed_merges_in_txn(&store.begin()?)
}

pub fn committed_merges_in_txn<E: Engine>(txn: &MetaTxn<'_, E>) -> Result<Vec<MergeIntent>> {
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("merge readback requires a certified metadata root"))?;
    let aborted = super::range_abort::effective_aborts_in_txn(txn, root.digest())?;
    let mut out: Vec<MergeIntent> = Vec::new();
    for row in task_rows(txn) {
        let row = row?;
        if let Some(intent) = from_merge_row(&row, root.digest())? {
            if !aborted.contains(&intent.operation) {
                validate_against(txn, root.digest(), &intent)?;
            }
            if out.iter().any(|p| p.task == intent.task || p.operation == intent.operation || p.child == intent.child) {
                return Err(invalid("duplicate merge identity"));
            }
            out.push(intent);
        }
    }
    Ok(out)
}

// ------------------------------------------------------------------ evidence

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeEvidence {
    root: RootDigest,
    operation: [u8; 16],
    task: u64,
    intent_task: u64,
    child_task: u64,
    digest: RootDigest,
    rows: u64,
}

impl MergeEvidence {
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
    pub fn digest(&self) -> RootDigest {
        self.digest
    }
    pub fn rows(&self) -> u64 {
        self.rows
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(MAX_MERGE_EVIDENCE_BYTES);
        bytes.extend_from_slice(MRE_MAGIC);
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
        if bytes.len() != MAX_MERGE_EVIDENCE_BYTES || &bytes[..8] != MRE_MAGIC {
            return Err(invalid("invalid merge evidence encoding"));
        }
        let e = Self {
            root: RootDigest::from_bytes(bytes[8..40].try_into().unwrap()),
            operation: bytes[40..56].try_into().unwrap(),
            task: u64::from_be_bytes(bytes[56..64].try_into().unwrap()),
            intent_task: u64::from_be_bytes(bytes[64..72].try_into().unwrap()),
            child_task: u64::from_be_bytes(bytes[72..80].try_into().unwrap()),
            digest: RootDigest::from_bytes(bytes[80..112].try_into().unwrap()),
            rows: u64::from_be_bytes(bytes[112..120].try_into().unwrap()),
        };
        if e.root.as_bytes() == &[0; 32]
            || e.operation == [0; 16]
            || e.intent_task < FIRST_DYNAMIC_ID
            || e.child_task < FIRST_DYNAMIC_ID
            || e.intent_task == e.child_task
            || e.digest.as_bytes() == &[0; 32]
        {
            return Err(invalid("invalid merge evidence identities"));
        }
        Ok(e)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let e = Self::decode_inner(bytes)?;
        if e.task < FIRST_DYNAMIC_ID || e.task == e.intent_task || e.task == e.child_task {
            return Err(invalid("invalid merge evidence task"));
        }
        Ok(e)
    }
    pub fn decode_receipt(bytes: &[u8]) -> Result<Self> {
        let e = Self::decode_inner(bytes)?;
        if e.task != 0 {
            return Err(invalid("a merge evidence receipt cannot supply its own task"));
        }
        Ok(e)
    }
    pub fn receipt(intent: &MergeIntent, digest: RootDigest, rows: u64) -> Result<Vec<u8>> {
        let receipt = Self {
            root: intent.root,
            operation: intent.operation,
            task: 0,
            intent_task: intent.task,
            child_task: intent.child,
            digest,
            rows,
        };
        let bytes = receipt.encode();
        Self::decode_receipt(&bytes)?;
        Ok(bytes)
    }
}

/// Is this receipt a merge evidence receipt (by magic)? The evidence RPC is shared.
pub fn is_merge_receipt(bytes: &[u8]) -> bool {
    bytes.len() >= 8 && &bytes[..8] == MRE_MAGIC
}

pub(crate) fn from_merge_evidence_row(row: &Row, root: RootDigest) -> Result<Option<MergeEvidence>> {
    if row.value.get(ColumnId(2)) != Some(&ColumnValue::Uint(MERGE_EVIDENCE)) {
        return Ok(None);
    }
    let Some(ColumnValue::Bytes(bytes)) = row.value.get(ColumnId(3)) else {
        return Err(invalid("merge evidence payload is missing"));
    };
    let e = MergeEvidence::decode(bytes)?;
    if e.root != root
        || row.pk != [memcmp_uint(e.task)]
        || row.value.get(ColumnId(1)) != Some(&ColumnValue::Uint(e.task))
        || row.value.get(ColumnId(4)) != Some(&ColumnValue::Uint(0))
        || row.value.get(ColumnId(5)) != Some(&ColumnValue::Uint(0))
    {
        return Err(invalid("merge evidence row binding differs"));
    }
    Ok(Some(e))
}

pub fn plan_merge_evidence<E: Engine>(txn: &mut MetaTxn<'_, E>, receipt: &[u8]) -> Result<(MergeEvidence, bool)> {
    let receipt = MergeEvidence::decode_receipt(receipt)?;
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("merge evidence requires a certified metadata root"))?;
    if receipt.root != root.digest() {
        return Err(invalid("merge evidence root differs from the certified root"));
    }
    let intent = committed_merges_in_txn(txn)?
        .into_iter()
        .find(|m| m.operation == receipt.operation)
        .ok_or_else(|| invalid("merge evidence requires the committed intent"))?;
    if super::range_abort::effective_aborts_in_txn(txn, root.digest())?.contains(&receipt.operation) {
        return Err(invalid("the merge intent is aborted; its evidence is refused"));
    }
    if intent.task != receipt.intent_task || intent.child != receipt.child_task {
        return Err(invalid("merge evidence differs from the committed intent"));
    }
    for row in task_rows(txn) {
        if let Some(previous) = from_merge_evidence_row(&row?, root.digest())? {
            if previous.operation == receipt.operation {
                let expected = MergeEvidence { task: previous.task, ..receipt.clone() };
                if expected != previous {
                    return Err(invalid("merge evidence conflicts"));
                }
                return Ok((previous, false));
            }
        }
    }
    let e = MergeEvidence {
        task: txn.allocate_id(SequenceKind::Task)?,
        ..receipt
    };
    MergeEvidence::decode(&e.encode())?;
    let mut row = RowValue::new();
    for (column, value) in [
        (ColumnId(1), ColumnValue::Uint(e.task)),
        (ColumnId(2), ColumnValue::Uint(MERGE_EVIDENCE)),
        (ColumnId(3), ColumnValue::Bytes(e.encode())),
        (ColumnId(4), ColumnValue::Uint(0)),
        (ColumnId(5), ColumnValue::Uint(0)),
    ] {
        row.set(column, value);
    }
    txn.insert(&TASKS_DESC, &[memcmp_uint(e.task)], row)?;
    Ok((e, true))
}

pub fn committed_merge_evidence<E: Engine>(store: &MetaStore<E>) -> Result<Vec<MergeEvidence>> {
    committed_merge_evidence_in_txn(&store.begin()?)
}

pub fn committed_merge_evidence_in_txn<E: Engine>(txn: &MetaTxn<'_, E>) -> Result<Vec<MergeEvidence>> {
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("merge evidence readback requires a certified metadata root"))?;
    let intents = committed_merges_in_txn(txn)?;
    let mut out: Vec<MergeEvidence> = Vec::new();
    for row in task_rows(txn) {
        if let Some(e) = from_merge_evidence_row(&row?, root.digest())? {
            let intent = intents
                .iter()
                .find(|m| m.operation == e.operation)
                .ok_or_else(|| invalid("merge evidence requires the committed intent"))?;
            if intent.task != e.intent_task || intent.child != e.child_task {
                return Err(invalid("merge evidence differs from the committed intent"));
            }
            if out.iter().any(|p| p.task == e.task || p.operation == e.operation) {
                return Err(invalid("duplicate merge evidence identity"));
            }
            out.push(e);
        }
    }
    Ok(out)
}

// ------------------------------------------------------------------ publication

/// Seal both parent bindings in place, bind the child to `[low.start, high.end)`
/// with its routable region row, remove both parents' region rows: one
/// transaction, only with the child's evidence. `false` confirms.
pub fn plan_merge_publication<E: Engine>(txn: &mut MetaTxn<'_, E>, operation: [u8; 16]) -> Result<(MergeIntent, bool)> {
    use crate::schema::REGIONS_DESC;
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("merge publication requires a certified metadata root"))?;
    let intent = committed_merges_in_txn(txn)?
        .into_iter()
        .find(|m| m.operation == operation)
        .ok_or_else(|| invalid("merge publication requires the committed intent"))?;
    if super::range_abort::effective_aborts_in_txn(txn, root.digest())?.contains(&operation) {
        return Err(invalid("the merge intent is aborted; nothing is published"));
    }
    let bindings = super::ranges::read_in(txn)?;
    let low = bindings
        .iter()
        .find(|b| b.range().region == intent.low_region)
        .ok_or_else(|| invalid("merge publication requires the low parent binding"))?;
    let high = bindings
        .iter()
        .find(|b| b.range().region == intent.high_region)
        .ok_or_else(|| invalid("merge publication requires the high parent binding"))?;
    if low.range().sealed && high.range().sealed {
        return Ok((intent, false));
    }
    if !committed_merge_evidence_in_txn(txn)?.iter().any(|e| e.operation == operation) {
        return Err(invalid("merge publication requires the child's population evidence"));
    }
    let (lr, hr) = (low.range().clone(), high.range().clone());
    for (binding, range) in [(low, &lr), (high, &hr)] {
        let mut sealed = range.clone();
        sealed.version = range.version.checked_add(1).ok_or_else(|| invalid("range version exhausted"))?;
        sealed.sealed = true;
        let mut payload = binding.creation().intent().task().to_be_bytes().to_vec();
        payload.extend_from_slice(&sealed.encode());
        txn.update(&TASKS_DESC, &[memcmp_uint(binding.bind_task())], vec![(ColumnId(3), ColumnValue::Bytes(payload))])?;
        if txn.get(&REGIONS_DESC, &[memcmp_uint(range.region.0)])?.is_some() {
            txn.delete(&REGIONS_DESC, &[memcmp_uint(range.region.0)])?;
        }
    }
    let child = creation_of(txn, root.digest(), intent.child)?;
    let range = DataRange {
        root: root.digest(),
        creation: child.digest(),
        region: child.region(),
        keyspace: lr.keyspace,
        tenant: lr.tenant,
        conf_ver: 1,
        version: 1,
        start: lr.start.clone(),
        end: hr.end.clone(),
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
    let mut payload = intent.child.to_be_bytes().to_vec();
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
    super::ranges::read_in(txn)?;
    validate_against(txn, root.digest(), &intent)?;
    Ok((intent, true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kv9_common::TenantId;

    #[test]
    fn a_merge_follows_a_published_split_and_publishes_the_union() {
        let store = super::super::tests::fixture();
        let mut txn = store.begin().unwrap();
        let mut tenant = RowValue::new();
        tenant.set(ColumnId(2), ColumnValue::Text("tenant".into()));
        tenant.set(ColumnId(3), ColumnValue::Uint(0));
        tenant.set(ColumnId(4), ColumnValue::Uint(0));
        txn.insert(&crate::schema::TENANTS_DESC, &[memcmp_uint(1)], tenant).unwrap();
        let parent = plan_empty_group(&mut txn, [1; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        let root = crate::root::certified_root(&txn).unwrap().unwrap().digest();
        let (bound, _) = super::super::ranges::plan_keyspace(&mut txn, root, parent.task(), "merge-me", TenantId(1)).unwrap();
        txn.commit().unwrap();
        // Split, evidence both children, publish.
        let mut txn = store.begin().unwrap();
        let (split, low, high, _) = super::super::split::plan_split(&mut txn, [2; 16], parent.region(), b"m").unwrap();
        txn.commit().unwrap();
        {
            let mut txn = store.begin().unwrap();
            assert!(plan_merge(&mut txn, [5; 16], low.region(), high.region()).is_err(), "unbound children cannot merge yet");
        }
        for child in [low.task(), high.task()] {
            let receipt = super::super::split_evidence::SplitEvidence::receipt(&split, child, RootDigest::sha256(&child.to_be_bytes()), 1).unwrap();
            let mut txn = store.begin().unwrap();
            super::super::split_evidence::plan_split_evidence(&mut txn, &receipt).unwrap();
            txn.commit().unwrap();
        }
        let mut txn = store.begin().unwrap();
        super::super::split::plan_split_publication(&mut txn, [2; 16]).unwrap();
        txn.commit().unwrap();
        // Merge the two children back.
        let mut txn = store.begin().unwrap();
        assert!(plan_merge(&mut txn, [5; 16], high.region(), low.region()).is_err(), "order matters: low then high");
        let (merge, child, changed) = plan_merge(&mut txn, [5; 16], low.region(), high.region()).unwrap();
        assert!(changed);
        txn.commit().unwrap();
        assert_eq!(merge.child(), child.task());
        assert_eq!(committed_merges(&store).unwrap(), vec![merge.clone()]);
        let mut txn = store.begin().unwrap();
        let (_, _, again) = plan_merge(&mut txn, [5; 16], low.region(), high.region()).unwrap();
        assert!(!again);
        assert!(plan_merge(&mut txn, [6; 16], low.region(), high.region()).is_err(), "one merge per parent");
        assert!(plan_merge_publication(&mut txn, [5; 16]).is_err(), "no evidence yet");
        drop(txn);
        let receipt = MergeEvidence::receipt(&merge, RootDigest::sha256(b"union"), 2).unwrap();
        assert!(is_merge_receipt(&receipt));
        let mut txn = store.begin().unwrap();
        let (_, ev_changed) = plan_merge_evidence(&mut txn, &receipt).unwrap();
        assert!(ev_changed);
        txn.commit().unwrap();
        let mut txn = store.begin().unwrap();
        let (_, published) = plan_merge_publication(&mut txn, [5; 16]).unwrap();
        assert!(published);
        txn.commit().unwrap();
        let bindings = super::super::ranges::committed_ranges(&store).unwrap();
        let live: Vec<_> = bindings.iter().filter(|b| !b.range().sealed).collect();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].range().region, child.region());
        assert!(live[0].range().start.is_empty() && live[0].range().end.is_empty());
        let txn = store.begin().unwrap();
        for key in [b"a".as_slice(), b"m", b"z"] {
            let (hit, _) = super::super::ranges::route_in(&txn, bound.keyspace, key).unwrap().unwrap();
            assert_eq!(hit.region, child.region());
        }
        let tables = crate::tables::Tables::new(&store);
        assert_eq!(tables.region_for_key(bound.keyspace, b"q").unwrap().unwrap().id, child.region());
        drop(txn);
        assert_eq!(committed_merges(&store).unwrap().len(), 1, "the intent reads back as its published shape");
        assert_eq!(super::super::split::committed_splits(&store).unwrap().len(), 1, "the split's history still validates");
        let mut txn = store.begin().unwrap();
        let (_, again) = plan_merge_publication(&mut txn, [5; 16]).unwrap();
        assert!(!again);
    }
}
