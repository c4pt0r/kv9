//! Committed healthy-group log-compaction decisions: the replicated floor
//! below which every replica of one data group may discard its OWN local
//! raft-log prefix. No migration, settlement or retention involvement —
//! this is the bounded-log-growth authority for ordinary groups.
//!
//! A decision never compacts anything by itself. Execution is per-replica
//! and locally gated: a node compacts only once ITS applied position has
//! reached the floor (committed entries are permanent, so a voter whose
//! log once contained the floor never re-fetches below it), after its own
//! engine sync barrier, through the durable REC_COMPACTION seam. Floors
//! per region are strictly increasing; an exact resubmission confirms.

use super::*;
use kv9_common::AppliedPosition;

const GROUP_COMPACTION: u64 = 110;
const CMP_MAGIC: &[u8; 8] = b"KV9CMP01";
pub const MAX_COMPACTION_BYTES: usize = 8 + 32 + 8 + 8 + 8 + 8 + 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupCompaction {
    root: RootDigest,
    task: u64,
    creation_task: u64,
    region: RegionId,
    floor: AppliedPosition,
}

impl GroupCompaction {
    pub fn root(&self) -> RootDigest {
        self.root
    }
    pub fn task(&self) -> u64 {
        self.task
    }
    pub fn creation_task(&self) -> u64 {
        self.creation_task
    }
    pub fn region(&self) -> RegionId {
        self.region
    }
    pub fn floor(&self) -> AppliedPosition {
        self.floor
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(MAX_COMPACTION_BYTES);
        bytes.extend_from_slice(CMP_MAGIC);
        bytes.extend_from_slice(self.root.as_bytes());
        bytes.extend_from_slice(&self.task.to_be_bytes());
        bytes.extend_from_slice(&self.creation_task.to_be_bytes());
        bytes.extend_from_slice(&self.region.0.to_be_bytes());
        bytes.extend_from_slice(&self.floor.term.to_be_bytes());
        bytes.extend_from_slice(&self.floor.index.to_be_bytes());
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != MAX_COMPACTION_BYTES || &bytes[..8] != CMP_MAGIC {
            return Err(invalid("invalid group compaction encoding"));
        }
        let decision = Self {
            root: RootDigest::from_bytes(bytes[8..40].try_into().unwrap()),
            task: u64::from_be_bytes(bytes[40..48].try_into().unwrap()),
            creation_task: u64::from_be_bytes(bytes[48..56].try_into().unwrap()),
            region: RegionId(u64::from_be_bytes(bytes[56..64].try_into().unwrap())),
            floor: AppliedPosition {
                term: u64::from_be_bytes(bytes[64..72].try_into().unwrap()),
                index: u64::from_be_bytes(bytes[72..80].try_into().unwrap()),
            },
        };
        if decision.root.as_bytes() == &[0; 32]
            || decision.task < FIRST_DYNAMIC_ID
            || decision.creation_task < FIRST_DYNAMIC_ID
            || decision.task == decision.creation_task
            || decision.region.0 < FIRST_DYNAMIC_ID
            || decision.floor.term == 0
            || decision.floor.index == 0
        {
            return Err(invalid("invalid group compaction identities"));
        }
        Ok(decision)
    }
}

fn from_compaction_row(row: &Row, root: RootDigest) -> Result<Option<GroupCompaction>> {
    if row.value.get(ColumnId(2)) != Some(&ColumnValue::Uint(GROUP_COMPACTION)) {
        return Ok(None);
    }
    let Some(ColumnValue::Bytes(bytes)) = row.value.get(ColumnId(3)) else {
        return Err(invalid("group compaction payload is missing"));
    };
    let decision = GroupCompaction::decode(bytes)?;
    if decision.root != root
        || row.pk != [memcmp_uint(decision.task)]
        || row.value.get(ColumnId(1)) != Some(&ColumnValue::Uint(decision.task))
        || row.value.get(ColumnId(4)) != Some(&ColumnValue::Uint(0))
        || row.value.get(ColumnId(5)) != Some(&ColumnValue::Uint(0))
    {
        return Err(invalid("group compaction row binding differs"));
    }
    Ok(Some(decision))
}

/// Stage one idempotent compaction decision in the serialized, term-fenced
/// catalog transaction. `false` confirms the exact committed floor. Floors
/// per region strictly increase (a lower or equal new floor with a
/// different position refuses; the exact floor confirms). Nothing here
/// compacts a log, syncs an engine, or touches retention.
pub fn plan_compaction<E: Engine>(
    txn: &mut MetaTxn<'_, E>,
    region: RegionId,
    floor: AppliedPosition,
) -> Result<(GroupCompaction, bool)> {
    if floor.term == 0 || floor.index == 0 {
        return Err(invalid("compaction requires an exact floor"));
    }
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("compaction requires a certified metadata root"))?;
    let rows = txn.scan(&TASKS_DESC, MAX_CREATION_TASKS + 1)?;
    if rows.len() > MAX_CREATION_TASKS {
        return Err(invalid("bounded compaction task scan exceeded"));
    }
    let mut creation_task = None;
    for row in &rows {
        if let Some(intent) = super::from_row(row, root.digest())? {
            if intent.region() == region {
                creation_task = Some(intent.task());
            }
        }
    }
    let creation_task = creation_task
        .ok_or_else(|| invalid("compaction requires the region's committed creation"))?;
    let decision = GroupCompaction {
        root: root.digest(),
        task: 0,
        creation_task,
        region,
        floor,
    };
    let mut highest: Option<GroupCompaction> = None;
    for row in &rows {
        if let Some(previous) = from_compaction_row(row, root.digest())? {
            if previous.region != region {
                continue;
            }
            if previous.floor == floor {
                let expected = GroupCompaction {
                    task: previous.task,
                    ..decision.clone()
                };
                if expected != previous {
                    return Err(invalid("compaction floor binding conflicts"));
                }
                return Ok((previous, false));
            }
            if highest
                .as_ref()
                .is_none_or(|h| previous.floor.index > h.floor.index)
            {
                highest = Some(previous);
            }
        }
    }
    if let Some(highest) = highest {
        if floor.index <= highest.floor.index || floor.term < highest.floor.term {
            return Err(invalid(
                "compaction floors must strictly increase per region",
            ));
        }
    }
    let decision = GroupCompaction {
        task: txn.allocate_id(SequenceKind::Task)?,
        ..decision
    };
    GroupCompaction::decode(&decision.encode())?;
    if rows.len() == MAX_CREATION_TASKS {
        return Err(invalid("compaction task capacity reached"));
    }
    let mut row = RowValue::new();
    for (column, value) in [
        (ColumnId(1), ColumnValue::Uint(decision.task)),
        (ColumnId(2), ColumnValue::Uint(GROUP_COMPACTION)),
        (ColumnId(3), ColumnValue::Bytes(decision.encode())),
        (ColumnId(4), ColumnValue::Uint(0)),
        (ColumnId(5), ColumnValue::Uint(0)),
    ] {
        row.set(column, value);
    }
    txn.insert(&TASKS_DESC, &[memcmp_uint(decision.task)], row)?;
    Ok((decision, true))
}

/// Read the HIGHEST committed compaction floor per region, with the
/// creation cross-validated. Decisions are immutable and never expire.
pub fn committed_compaction_floors<E: Engine>(
    store: &MetaStore<E>,
) -> Result<Vec<GroupCompaction>> {
    let txn = store.begin()?;
    let root = crate::root::certified_root(&txn)?
        .ok_or_else(|| invalid("compaction readback requires a certified metadata root"))?;
    let rows = txn.scan(&TASKS_DESC, MAX_CREATION_TASKS + 1)?;
    if rows.len() > MAX_CREATION_TASKS {
        return Err(invalid("bounded compaction task scan exceeded"));
    }
    let mut highest: std::collections::BTreeMap<u64, GroupCompaction> =
        std::collections::BTreeMap::new();
    for row in &rows {
        if let Some(decision) = from_compaction_row(row, root.digest())? {
            let creation = rows
                .iter()
                .filter_map(|other| super::from_row(other, root.digest()).ok().flatten())
                .find(|intent| intent.region() == decision.region)
                .ok_or_else(|| invalid("compaction requires the region's committed creation"))?;
            if creation.task() != decision.creation_task {
                return Err(invalid("compaction names a different creation"));
            }
            match highest.get(&decision.region.0) {
                Some(prior) if prior.floor.index >= decision.floor.index => {}
                _ => {
                    highest.insert(decision.region.0, decision);
                }
            }
        }
    }
    Ok(highest.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn committed_group(store: &MetaStore<kv9_engine::MemEngine>) -> RegionId {
        let mut txn = store.begin().unwrap();
        let creation =
            plan_empty_group(&mut txn, [1; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        super::super::activation::plan_activation(&mut txn, &creation).unwrap();
        txn.commit().unwrap();
        creation.region()
    }

    fn at(term: u64, index: u64) -> AppliedPosition {
        AppliedPosition { term, index }
    }

    #[test]
    fn floors_bind_the_creation_strictly_increase_and_confirm_idempotently() {
        let store = super::super::tests::fixture();
        let region = committed_group(&store);
        let mut txn = store.begin().unwrap();
        assert!(
            plan_compaction(&mut txn, RegionId(999), at(2, 10)).is_err(),
            "an unknown region must refuse"
        );
        assert!(plan_compaction(&mut txn, region, at(0, 10)).is_err());
        let (first, changed) = plan_compaction(&mut txn, region, at(2, 10)).unwrap();
        assert!(changed);
        assert_eq!(first.region(), region);
        assert_eq!(first.floor(), at(2, 10));
        txn.commit().unwrap();
        assert_eq!(
            committed_compaction_floors(&store).unwrap(),
            vec![first.clone()]
        );
        let mut txn = store.begin().unwrap();
        // Exact confirm; equal or lower floors refuse; higher commits and
        // the readback returns only the HIGHEST floor per region.
        let (retry, changed) = plan_compaction(&mut txn, region, at(2, 10)).unwrap();
        assert!(!changed);
        assert_eq!(retry, first);
        assert!(plan_compaction(&mut txn, region, at(2, 9)).is_err());
        assert!(plan_compaction(&mut txn, region, at(3, 10)).is_err());
        let (second, changed) = plan_compaction(&mut txn, region, at(2, 25)).unwrap();
        assert!(changed);
        txn.commit().unwrap();
        assert_eq!(
            committed_compaction_floors(&store).unwrap(),
            vec![second.clone()]
        );
        // Term regression at a higher index refuses.
        let mut txn = store.begin().unwrap();
        assert!(plan_compaction(&mut txn, region, at(1, 30)).is_err());
    }

    #[test]
    fn compaction_readback_refuses_row_rebinding() {
        for defect in ["task", "state", "created", "truncated", "creation"] {
            let store = super::super::tests::fixture();
            let region = committed_group(&store);
            let mut txn = store.begin().unwrap();
            let (planned, _) = plan_compaction(&mut txn, region, at(2, 10)).unwrap();
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
                    ColumnValue::Bytes(planned.encode()[..40].to_vec()),
                ),
                "creation" => {
                    let mut bytes = planned.encode();
                    bytes[55] ^= 1; // creation task
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
            assert!(
                committed_compaction_floors(&store).is_err(),
                "accepted {defect}"
            );
        }
    }
}
