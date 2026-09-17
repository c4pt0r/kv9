//! Committed split intents: replicated authority to split one bound range
//! at one exact key into two ALREADY-CREATED empty child groups.
//!
//! An intent never seals a parent, populates a child, republishes the range
//! directory or reroutes a single key. It binds the parent's exact current
//! binding row, a nonempty split key, and two distinct committed child
//! creations on the SAME replica set as the parent — a split moves no
//! replica; movement stays D03's job. One live intent per parent region;
//! an exact resubmission is a confirmation; any divergence refuses.

use super::*;

const SPLIT_RANGE: u64 = 107;
const SPL_MAGIC: &[u8; 8] = b"KV9SPL01";
pub const MAX_SPLIT_KEY_BYTES: usize = 1024;
pub const MAX_SPLIT_BYTES: usize = 96 + 2 + MAX_SPLIT_KEY_BYTES;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitIntent {
    root: RootDigest,
    operation: [u8; 16],
    task: u64,
    parent_bind_task: u64,
    parent_region: RegionId,
    child_low: u64,
    child_high: u64,
    split_key: Vec<u8>,
}

impl SplitIntent {
    pub fn operation(&self) -> [u8; 16] {
        self.operation
    }
    pub fn root(&self) -> RootDigest {
        self.root
    }
    pub fn task(&self) -> u64 {
        self.task
    }
    pub fn parent_bind_task(&self) -> u64 {
        self.parent_bind_task
    }
    pub fn parent_region(&self) -> RegionId {
        self.parent_region
    }
    /// The creation task of the child owning `[start, split_key)`.
    pub fn child_low(&self) -> u64 {
        self.child_low
    }
    /// The creation task of the child owning `[split_key, end)`.
    pub fn child_high(&self) -> u64 {
        self.child_high
    }
    pub fn split_key(&self) -> &[u8] {
        &self.split_key
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(MAX_SPLIT_BYTES);
        bytes.extend_from_slice(SPL_MAGIC);
        bytes.extend_from_slice(self.root.as_bytes());
        bytes.extend_from_slice(&self.operation);
        bytes.extend_from_slice(&self.task.to_be_bytes());
        bytes.extend_from_slice(&self.parent_bind_task.to_be_bytes());
        bytes.extend_from_slice(&self.parent_region.0.to_be_bytes());
        bytes.extend_from_slice(&self.child_low.to_be_bytes());
        bytes.extend_from_slice(&self.child_high.to_be_bytes());
        bytes.extend_from_slice(&(self.split_key.len() as u16).to_be_bytes());
        bytes.extend_from_slice(&self.split_key);
        bytes
    }

    /// Decode data only. This does not mint committed split authority.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 99 || bytes.len() > MAX_SPLIT_BYTES || &bytes[..8] != SPL_MAGIC {
            return Err(invalid("invalid split intent encoding"));
        }
        let key_len = u16::from_be_bytes(bytes[96..98].try_into().unwrap()) as usize;
        if bytes.len() != 98 + key_len || key_len == 0 || key_len > MAX_SPLIT_KEY_BYTES {
            return Err(invalid("invalid split intent encoding"));
        }
        let intent = Self {
            root: RootDigest::from_bytes(bytes[8..40].try_into().unwrap()),
            operation: bytes[40..56].try_into().unwrap(),
            task: u64::from_be_bytes(bytes[56..64].try_into().unwrap()),
            parent_bind_task: u64::from_be_bytes(bytes[64..72].try_into().unwrap()),
            parent_region: RegionId(u64::from_be_bytes(bytes[72..80].try_into().unwrap())),
            child_low: u64::from_be_bytes(bytes[80..88].try_into().unwrap()),
            child_high: u64::from_be_bytes(bytes[88..96].try_into().unwrap()),
            split_key: bytes[98..].to_vec(),
        };
        if intent.root.as_bytes() == &[0; 32]
            || intent.operation == [0; 16]
            || intent.task < FIRST_DYNAMIC_ID
            || intent.parent_bind_task < FIRST_DYNAMIC_ID
            || intent.parent_region.0 < FIRST_DYNAMIC_ID
            || intent.child_low < FIRST_DYNAMIC_ID
            || intent.child_high < FIRST_DYNAMIC_ID
            || intent.child_low == intent.child_high
            || [intent.parent_bind_task, intent.child_low, intent.child_high].contains(&intent.task)
        {
            return Err(invalid("invalid split intent identities"));
        }
        Ok(intent)
    }
}

fn from_split_row(row: &Row, root: RootDigest) -> Result<Option<SplitIntent>> {
    if row.value.get(ColumnId(2)) != Some(&ColumnValue::Uint(SPLIT_RANGE)) {
        return Ok(None);
    }
    let Some(ColumnValue::Bytes(bytes)) = row.value.get(ColumnId(3)) else {
        return Err(invalid("split intent payload is missing"));
    };
    let intent = SplitIntent::decode(bytes)?;
    if intent.root != root
        || row.pk != [memcmp_uint(intent.task)]
        || row.value.get(ColumnId(1)) != Some(&ColumnValue::Uint(intent.task))
        || row.value.get(ColumnId(4)) != Some(&ColumnValue::Uint(0))
        || row.value.get(ColumnId(5)) != Some(&ColumnValue::Uint(0))
    {
        return Err(invalid("split intent row binding differs"));
    }
    Ok(Some(intent))
}

/// The full committed cross-validation shared by planning and readback: the
/// parent binding must exist unsealed with this exact bind task, the split
/// key must fall strictly inside the parent range, and both children must be
/// committed, activated, UNBOUND creations with exactly the parent's replica
/// set. Nothing here mints a publication or sealing capability.
fn validate_against<E: Engine>(
    txn: &MetaTxn<'_, E>,
    rows: &[Row],
    root: RootDigest,
    intent: &SplitIntent,
) -> Result<()> {
    let parent = super::ranges::committed_ranges_in_txn(txn)?
        .into_iter()
        .find(|b| b.range().region == intent.parent_region)
        .ok_or_else(|| invalid("split requires the parent's committed range binding"))?;
    if parent.bind_task() != intent.parent_bind_task {
        return Err(invalid("split names a different parent binding row"));
    }
    if parent.range().sealed {
        return Err(invalid("split parent range is already sealed"));
    }
    let range = parent.range();
    let inside = (range.start.is_empty() || intent.split_key.as_slice() > range.start.as_slice())
        && (range.end.is_empty() || intent.split_key.as_slice() < range.end.as_slice());
    if !inside {
        return Err(invalid("split key must fall strictly inside the parent"));
    }
    let activations = super::activation::decode(txn, rows, root)?;
    let bound: Vec<RegionId> = super::ranges::committed_ranges_in_txn(txn)?
        .iter()
        .map(|b| b.range().region)
        .collect();
    for child_task in [intent.child_low, intent.child_high] {
        let creation_row = txn
            .get(&TASKS_DESC, &[memcmp_uint(child_task)])?
            .ok_or_else(|| invalid("split child creation is missing"))?;
        let child = from_row(&creation_row, root)?
            .ok_or_else(|| invalid("split child creation row differs"))?;
        if child.replicas() != parent.creation().intent().replicas() {
            return Err(invalid("split children must keep the parent replica set"));
        }
        if !activations.iter().any(|a| a == &child) {
            return Err(invalid("split children require committed activation"));
        }
        if bound.contains(&child.region()) {
            return Err(invalid("split children must be unbound empty groups"));
        }
        if child.region() == intent.parent_region {
            return Err(invalid("a split child cannot be its own parent"));
        }
    }
    Ok(())
}

/// Stage one idempotent split intent in the serialized, term-fenced catalog
/// transaction. `false` means the exact intent already exists and this
/// commit is a confirmation receipt. Nothing here seals, populates,
/// republishes or reroutes.
pub fn plan_split<E: Engine>(
    txn: &mut MetaTxn<'_, E>,
    operation: [u8; 16],
    parent_region: RegionId,
    split_key: &[u8],
    child_low: u64,
    child_high: u64,
) -> Result<(SplitIntent, bool)> {
    if operation == [0; 16] {
        return Err(invalid("split needs a nonzero operation"));
    }
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("split requires a certified metadata root"))?;
    let rows = txn.scan(&TASKS_DESC, MAX_CREATION_TASKS + 1)?;
    if rows.len() > MAX_CREATION_TASKS {
        return Err(invalid("bounded split task scan exceeded"));
    }
    let parent = super::ranges::committed_ranges_in_txn(txn)?
        .into_iter()
        .find(|b| b.range().region == parent_region)
        .ok_or_else(|| invalid("split requires the parent's committed range binding"))?;
    let intent = SplitIntent {
        root: root.digest(),
        operation,
        task: 0,
        parent_bind_task: parent.bind_task(),
        parent_region,
        child_low,
        child_high,
        split_key: split_key.to_vec(),
    };
    for row in &rows {
        if let Some(previous) = from_split_row(row, root.digest())? {
            if previous.operation == operation {
                let expected = SplitIntent {
                    task: previous.task,
                    ..intent.clone()
                };
                if expected != previous {
                    return Err(invalid("split operation binding conflicts"));
                }
                return Ok((previous, false));
            }
            if previous.parent_region == parent_region {
                return Err(invalid("a committed split already binds this parent"));
            }
        }
    }
    let intent = SplitIntent {
        task: txn.allocate_id(SequenceKind::Task)?,
        ..intent
    };
    SplitIntent::decode(&intent.encode())?;
    validate_against(txn, &rows, root.digest(), &intent)?;
    if rows.len() == MAX_CREATION_TASKS {
        return Err(invalid("split task capacity reached"));
    }
    let mut row = RowValue::new();
    for (column, value) in [
        (ColumnId(1), ColumnValue::Uint(intent.task)),
        (ColumnId(2), ColumnValue::Uint(SPLIT_RANGE)),
        (ColumnId(3), ColumnValue::Bytes(intent.encode())),
        (ColumnId(4), ColumnValue::Uint(0)),
        (ColumnId(5), ColumnValue::Uint(0)),
    ] {
        row.set(column, value);
    }
    txn.insert(&TASKS_DESC, &[memcmp_uint(intent.task)], row)?;
    Ok((intent, true))
}

/// Read every committed split intent from one fresh applied snapshot with
/// the full cross-validation re-run. Intents never expire or cancel.
pub fn committed_splits<E: Engine>(store: &MetaStore<E>) -> Result<Vec<SplitIntent>> {
    let txn = store.begin()?;
    let root = crate::root::certified_root(&txn)?
        .ok_or_else(|| invalid("split readback requires a certified metadata root"))?;
    let rows = txn.scan(&TASKS_DESC, MAX_CREATION_TASKS + 1)?;
    if rows.len() > MAX_CREATION_TASKS {
        return Err(invalid("bounded split task scan exceeded"));
    }
    let mut out: Vec<SplitIntent> = Vec::new();
    for row in &rows {
        if let Some(intent) = from_split_row(row, root.digest())? {
            validate_against(&txn, &rows, root.digest(), &intent)?;
            if out.iter().any(|prior| {
                prior.task == intent.task
                    || prior.operation == intent.operation
                    || prior.parent_region == intent.parent_region
                    || prior.child_low == intent.child_low
                    || prior.child_high == intent.child_high
            }) {
                return Err(invalid("duplicate split identity"));
            }
            out.push(intent);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kv9_common::TenantId;

    fn bound_parent(
        store: &MetaStore<kv9_engine::MemEngine>,
    ) -> (RegionId, CreationIntent, CreationIntent) {
        let mut txn = store.begin().unwrap();
        let mut tenant = RowValue::new();
        tenant.set(ColumnId(2), ColumnValue::Text("tenant".into()));
        tenant.set(ColumnId(3), ColumnValue::Uint(0));
        tenant.set(ColumnId(4), ColumnValue::Uint(0));
        txn.insert(&crate::schema::TENANTS_DESC, &[memcmp_uint(1)], tenant)
            .unwrap();
        let parent =
            plan_empty_group(&mut txn, [1; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        super::super::ranges::plan_keyspace(
            &mut txn,
            parent.root(),
            parent.task(),
            "p",
            TenantId(1),
        )
        .unwrap();
        let low = plan_empty_group(&mut txn, [2; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        super::super::activation::plan_activation(&mut txn, &low).unwrap();
        let high = plan_empty_group(&mut txn, [3; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        super::super::activation::plan_activation(&mut txn, &high).unwrap();
        txn.commit().unwrap();
        (parent.region(), low, high)
    }

    #[test]
    fn a_split_binds_one_unsealed_parent_and_two_unbound_activated_children() {
        let store = super::super::tests::fixture();
        let (parent_region, low, high) = bound_parent(&store);
        let mut txn = store.begin().unwrap();
        assert!(
            plan_split(
                &mut txn,
                [9; 16],
                RegionId(999),
                b"m",
                low.task(),
                high.task()
            )
            .is_err(),
            "an unbound parent region must refuse"
        );
        assert!(
            plan_split(
                &mut txn,
                [9; 16],
                parent_region,
                b"m",
                low.task(),
                low.task()
            )
            .is_err(),
            "identical children must refuse"
        );
        assert!(
            plan_split(&mut txn, [9; 16], parent_region, b"m", low.task(), 999).is_err(),
            "a missing child creation must refuse"
        );
        let (planned, changed) = plan_split(
            &mut txn,
            [9; 16],
            parent_region,
            b"m",
            low.task(),
            high.task(),
        )
        .unwrap();
        assert!(changed);
        assert_eq!(planned.parent_region(), parent_region);
        assert_eq!(planned.split_key(), b"m");
        assert!(committed_splits(&store).unwrap().is_empty());
        txn.commit().unwrap();
        assert_eq!(committed_splits(&store).unwrap(), vec![planned.clone()]);
        let mut txn = store.begin().unwrap();
        let (retry, changed) = plan_split(
            &mut txn,
            [9; 16],
            parent_region,
            b"m",
            low.task(),
            high.task(),
        )
        .unwrap();
        assert!(!changed);
        assert_eq!(retry, planned);
        assert!(
            plan_split(
                &mut txn,
                [9; 16],
                parent_region,
                b"n",
                low.task(),
                high.task()
            )
            .is_err(),
            "one operation never names a second split key"
        );
        assert!(
            plan_split(
                &mut txn,
                [10; 16],
                parent_region,
                b"n",
                low.task(),
                high.task()
            )
            .is_err(),
            "one live split per parent region"
        );
    }

    #[test]
    fn split_readback_refuses_row_and_authority_rebinding() {
        // Operation and split key are the row's own immutable identity;
        // cross-bound fields must refuse on any divergence.
        for defect in [
            "task",
            "state",
            "created",
            "truncated",
            "parent-bind",
            "child-low",
            "child-high",
        ] {
            let store = super::super::tests::fixture();
            let (parent_region, low, high) = bound_parent(&store);
            let mut txn = store.begin().unwrap();
            let (planned, _) = plan_split(
                &mut txn,
                [9; 16],
                parent_region,
                b"m",
                low.task(),
                high.task(),
            )
            .unwrap();
            txn.commit().unwrap();
            let mut txn = store.begin().unwrap();
            let row = txn
                .get(&TASKS_DESC, &[memcmp_uint(planned.task())])
                .unwrap()
                .unwrap();
            let change = match defect {
                "task" => (ColumnId(1), ColumnValue::Uint(999)),
                "state" => (ColumnId(4), ColumnValue::Uint(1)),
                "created" => (ColumnId(5), ColumnValue::Uint(1)),
                "truncated" => (
                    ColumnId(3),
                    ColumnValue::Bytes(planned.encode()[..90].to_vec()),
                ),
                "parent-bind" | "child-low" | "child-high" => {
                    let mut bytes = planned.encode();
                    let at = match defect {
                        "parent-bind" => 71,
                        "child-low" => 87,
                        _ => 95,
                    };
                    bytes[at] ^= 1;
                    (ColumnId(3), ColumnValue::Bytes(bytes))
                }
                _ => unreachable!(),
            };
            if defect == "task" {
                assert!(txn.update(&TASKS_DESC, &row.pk, vec![change]).is_err());
                continue;
            }
            txn.update(&TASKS_DESC, &row.pk, vec![change]).unwrap();
            txn.commit().unwrap();
            assert!(committed_splits(&store).is_err(), "accepted {defect}");
        }
    }
}
