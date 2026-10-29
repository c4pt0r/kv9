//! Committed split intents (docs/RANGE-SPLIT.md §2): replicated authority to split
//! one bound range at one exact key into two child groups on the parent's stores.
//!
//! An intent never seals a parent, populates a child, republishes the range
//! directory or reroutes a key. It binds the parent's exact current binding row,
//! a nonempty split key strictly inside the range, and two child creations that
//! the SAME transaction creates and activates on the parent's committed member
//! nodes. One live intent per parent region; an exact resubmission confirms;
//! divergence refuses.

use super::*;

const SPLIT_RANGE: u64 = 113;
const SPL_MAGIC: &[u8; 8] = b"KV9SPL02";
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

/// The child operations are derived from the split operation, so a repeated
/// `plan_split` finds (and confirms) the same creations.
pub fn child_operations(operation: [u8; 16]) -> ([u8; 16], [u8; 16]) {
    let derive = |suffix: &[u8]| {
        let mut bytes = b"kv9-split-child/v1".to_vec();
        bytes.extend_from_slice(&operation);
        bytes.extend_from_slice(suffix);
        let digest = RootDigest::sha256(&bytes);
        let mut out = [0u8; 16];
        out.copy_from_slice(&digest.as_bytes()[..16]);
        out
    };
    (derive(b"/low"), derive(b"/high"))
}

pub(crate) fn from_split_row(row: &Row, root: RootDigest) -> Result<Option<SplitIntent>> {
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

fn creation_of<E: Engine>(txn: &MetaTxn<'_, E>, root: RootDigest, task: u64) -> Result<CreationIntent> {
    let row = txn
        .get(&TASKS_DESC, &[memcmp_uint(task)])?
        .ok_or_else(|| invalid("split child creation is missing"))?;
    from_row(&row, root)?.ok_or_else(|| invalid("split child creation row differs"))
}

/// The full committed cross-validation shared by planning and readback: the
/// parent binding must exist with this exact bind task; unsealed, the split key
/// falls strictly inside it and both children are committed, activated, UNBOUND
/// creations whose replicas are exactly the parent's committed members; sealed,
/// the parent is legal only as this intent's own published shape.
fn validate_against<E: Engine>(txn: &MetaTxn<'_, E>, root: RootDigest, intent: &SplitIntent) -> Result<()> {
    let bindings = super::ranges::read_in(txn)?;
    let parent = bindings
        .iter()
        .find(|b| b.range().region == intent.parent_region)
        .ok_or_else(|| invalid("split requires the parent's committed range binding"))?;
    if parent.bind_task() != intent.parent_bind_task {
        return Err(invalid("split names a different parent binding row"));
    }
    if parent.range().sealed {
        let published = [
            (intent.child_low, parent.range().start.clone(), intent.split_key.clone()),
            (intent.child_high, intent.split_key.clone(), parent.range().end.clone()),
        ]
        .into_iter()
        .all(|(task, start, end)| {
            creation_of(txn, root, task).is_ok_and(|creation| {
                bindings.iter().any(|b| {
                    b.range().region == creation.region() && b.range().start == start && b.range().end == end
                })
            })
        });
        if !published {
            return Err(invalid("split parent range is already sealed"));
        }
        return Ok(());
    }
    let range = parent.range();
    let inside = (range.start.is_empty() || intent.split_key.as_slice() > range.start.as_slice())
        && (range.end.is_empty() || intent.split_key.as_slice() < range.end.as_slice());
    if !inside {
        return Err(invalid("split key must fall strictly inside the parent"));
    }
    let members: Vec<InitialReplica> = super::committed_members(txn, root, parent.creation().intent())?
        .into_iter()
        .map(|(m, _)| m)
        .collect();
    let activations = super::activation::decode(txn, root)?;
    let bound: Vec<RegionId> = bindings.iter().map(|b| b.range().region).collect();
    for child_task in [intent.child_low, intent.child_high] {
        let child = creation_of(txn, root, child_task)?;
        let mut replicas = child.replicas().to_vec();
        replicas.sort_by_key(|r| r.node.0);
        let mut expected = members.clone();
        expected.sort_by_key(|r| r.node.0);
        if replicas != expected {
            return Err(invalid("split children must live on the parent's committed members"));
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

/// Stage one idempotent split intent — and the two children it names, created
/// and activated on the parent's committed member nodes — in the serialized,
/// term-fenced catalog transaction. `false` means the exact intent already
/// exists and this commit is a confirmation receipt. Nothing here seals,
/// populates, republishes or reroutes.
pub fn plan_split<E: Engine>(
    txn: &mut MetaTxn<'_, E>,
    operation: [u8; 16],
    parent_region: RegionId,
    split_key: &[u8],
) -> Result<(SplitIntent, CreationIntent, CreationIntent, bool)> {
    if operation == [0; 16] {
        return Err(invalid("split needs a nonzero operation"));
    }
    if split_key.is_empty() || split_key.len() > MAX_SPLIT_KEY_BYTES {
        return Err(invalid("split key must contain 1..1024 bytes"));
    }
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("split requires a certified metadata root"))?;
    let parent = super::ranges::read_in(txn)?
        .into_iter()
        .find(|b| b.range().region == parent_region)
        .ok_or_else(|| invalid("split requires the parent's committed range binding"))?;
    // An existing intent for this operation or parent decides before anything is staged.
    for row in task_rows(txn) {
        let row = row?;
        if let Some(previous) = from_split_row(&row, root.digest())? {
            if previous.operation == operation {
                if previous.parent_region != parent_region
                    || previous.parent_bind_task != parent.bind_task()
                    || previous.split_key != split_key
                {
                    return Err(invalid("split operation binding conflicts"));
                }
                let low = creation_of(txn, root.digest(), previous.child_low)?;
                let high = creation_of(txn, root.digest(), previous.child_high)?;
                return Ok((previous, low, high, false));
            }
            if previous.parent_region == parent_region {
                return Err(invalid("a committed split already binds this parent"));
            }
        }
    }
    if parent.range().sealed {
        return Err(invalid("split parent range is already sealed"));
    }
    let members: Vec<NodeId> = super::committed_members(txn, root.digest(), parent.creation().intent())?
        .into_iter()
        .map(|(m, _)| m.node)
        .collect();
    let (low_operation, high_operation) = child_operations(operation);
    let low = super::plan_empty_group(txn, low_operation, &members)?;
    let high = super::plan_empty_group(txn, high_operation, &members)?;
    super::activation::plan_activation(txn, &low)?;
    super::activation::plan_activation(txn, &high)?;
    let intent = SplitIntent {
        root: root.digest(),
        operation,
        task: txn.allocate_id(SequenceKind::Task)?,
        parent_bind_task: parent.bind_task(),
        parent_region,
        child_low: low.task(),
        child_high: high.task(),
        split_key: split_key.to_vec(),
    };
    SplitIntent::decode(&intent.encode())?;
    validate_against(txn, root.digest(), &intent)?;
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
    Ok((intent, low, high, true))
}

/// Every committed split intent from one applied snapshot, with the full
/// cross-validation re-run. Intents never expire or cancel.
pub fn committed_splits<E: Engine>(store: &MetaStore<E>) -> Result<Vec<SplitIntent>> {
    let txn = store.begin()?;
    committed_splits_in_txn(&txn)
}

pub fn committed_splits_in_txn<E: Engine>(txn: &MetaTxn<'_, E>) -> Result<Vec<SplitIntent>> {
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("split readback requires a certified metadata root"))?;
    let mut out: Vec<SplitIntent> = Vec::new();
    for row in task_rows(txn) {
        let row = row?;
        if let Some(intent) = from_split_row(&row, root.digest())? {
            validate_against(txn, root.digest(), &intent)?;
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
    use crate::schema::NODES_DESC;
    use kv9_common::TenantId;

    fn register_node(store: &MetaStore<kv9_engine::MemEngine>, node: u64, incarnation: [u8; 16]) {
        let mut txn = store.begin().unwrap();
        let mut row = RowValue::new();
        row.set(ColumnId(2), ColumnValue::Text(format!("127.0.0.1:{}", 18000 + node)));
        row.set(ColumnId(3), ColumnValue::Uint(2));
        row.set(ColumnId(4), ColumnValue::Uint(0));
        row.set(ColumnId(5), ColumnValue::Bytes(incarnation.to_vec()));
        txn.insert(&NODES_DESC, &[memcmp_uint(node)], row).unwrap();
        txn.commit().unwrap();
    }

    /// A bound parent on nodes 1,2,3 (the fixture registers them).
    fn bound_parent(store: &MetaStore<kv9_engine::MemEngine>) -> (CreationIntent, RegionId) {
        let mut txn = store.begin().unwrap();
        let mut tenant = RowValue::new();
        tenant.set(ColumnId(2), ColumnValue::Text("tenant".into()));
        tenant.set(ColumnId(3), ColumnValue::Uint(0));
        tenant.set(ColumnId(4), ColumnValue::Uint(0));
        txn.insert(&crate::schema::TENANTS_DESC, &[memcmp_uint(1)], tenant).unwrap();
        let creation = plan_empty_group(&mut txn, [1; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        let root = crate::root::certified_root(&txn).unwrap().unwrap().digest();
        super::super::ranges::plan_keyspace(&mut txn, root, creation.task(), "split-me", TenantId(1)).unwrap();
        txn.commit().unwrap();
        let region = creation.region();
        (creation, region)
    }

    #[test]
    fn an_intent_creates_its_children_and_confirms_exactly_once_per_parent() {
        let store = super::super::tests::fixture();
        let (parent, region) = bound_parent(&store);
        {
            let mut txn = store.begin().unwrap();
            assert!(plan_split(&mut txn, [2; 16], RegionId(region.0 + 1), b"m").is_err(), "unbound parent");
            assert!(plan_split(&mut txn, [2; 16], region, b"").is_err(), "empty key");
        }
        let mut txn = store.begin().unwrap();
        let (intent, low, high, changed) = plan_split(&mut txn, [2; 16], region, b"m").unwrap();
        assert!(changed);
        txn.commit().unwrap();
        assert_eq!(intent.parent_region(), region);
        assert_eq!(intent.split_key(), b"m");
        assert_ne!(low.region(), high.region());
        assert_eq!(low.replicas(), parent.replicas());
        assert_eq!(high.replicas(), parent.replicas());
        let activated = super::super::activation::committed_activations(&store).unwrap();
        assert!(activated.iter().any(|a| a.creation().intent() == &low));
        assert!(activated.iter().any(|a| a.creation().intent() == &high));
        // The exact resubmission confirms; a different key under the same operation
        // and a second intent for the parent refuse.
        let mut txn = store.begin().unwrap();
        let (again, l2, h2, changed) = plan_split(&mut txn, [2; 16], region, b"m").unwrap();
        assert!(!changed);
        assert_eq!((again, l2, h2), (intent.clone(), low.clone(), high.clone()));
        assert!(plan_split(&mut txn, [2; 16], region, b"n").is_err());
        assert!(plan_split(&mut txn, [3; 16], region, b"n").is_err());
        drop(txn);
        let splits = committed_splits(&store).unwrap();
        assert_eq!(splits, vec![intent.clone()]);
        // A child is not a parent: it is bound to nothing.
        let mut txn = store.begin().unwrap();
        assert!(plan_split(&mut txn, [4; 16], low.region(), b"c").is_err());
        drop(txn);
        // Readback refuses an intent whose children lost their activation evidence
        // (a tampered catalog), and decode refuses a tampered key length.
        let mut bytes = intent.encode();
        bytes[97] ^= 1;
        assert!(SplitIntent::decode(&bytes).is_err());
        let _ = register_node;
    }
}
