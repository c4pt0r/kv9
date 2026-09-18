//! Committed source-truncation decisions: replicated authority for the
//! migration source group to compact its log prefix below one exact floor.
//!
//! A decision never compacts anything by itself, never touches the ledger,
//! and never moves data. Its floor is bounded by the committed evidence
//! cut: the destination's installed image already carries every entry at or
//! below that cut, so no entry a migration learner could still need is ever
//! authorized away. One decision per operation; an exact resubmission is a
//! confirmation; any divergence refuses; the row is immutable.

use super::*;
use kv9_common::AppliedPosition;

const SOURCE_TRUNCATION: u64 = 105;
const TRC_MAGIC: &[u8; 8] = b"KV9TRC01";
pub const MAX_TRUNCATION_BYTES: usize = 8 + 32 + 16 + 8 + 8 + 8 + 8 + 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TruncationDecision {
    root: RootDigest,
    operation: [u8; 16],
    task: u64,
    evidence_task: u64,
    region: RegionId,
    floor: AppliedPosition,
}

impl TruncationDecision {
    pub fn operation(&self) -> [u8; 16] {
        self.operation
    }
    pub fn root(&self) -> RootDigest {
        self.root
    }
    pub fn task(&self) -> u64 {
        self.task
    }
    pub fn evidence_task(&self) -> u64 {
        self.evidence_task
    }
    pub fn region(&self) -> RegionId {
        self.region
    }
    pub fn floor(&self) -> AppliedPosition {
        self.floor
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(MAX_TRUNCATION_BYTES);
        bytes.extend_from_slice(TRC_MAGIC);
        bytes.extend_from_slice(self.root.as_bytes());
        bytes.extend_from_slice(&self.operation);
        bytes.extend_from_slice(&self.task.to_be_bytes());
        bytes.extend_from_slice(&self.evidence_task.to_be_bytes());
        bytes.extend_from_slice(&self.region.0.to_be_bytes());
        bytes.extend_from_slice(&self.floor.term.to_be_bytes());
        bytes.extend_from_slice(&self.floor.index.to_be_bytes());
        bytes
    }

    /// Decode data only. This does not mint committed truncation authority.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != MAX_TRUNCATION_BYTES || &bytes[..8] != TRC_MAGIC {
            return Err(invalid("invalid truncation decision encoding"));
        }
        let decision = Self {
            root: RootDigest::from_bytes(bytes[8..40].try_into().unwrap()),
            operation: bytes[40..56].try_into().unwrap(),
            task: u64::from_be_bytes(bytes[56..64].try_into().unwrap()),
            evidence_task: u64::from_be_bytes(bytes[64..72].try_into().unwrap()),
            region: RegionId(u64::from_be_bytes(bytes[72..80].try_into().unwrap())),
            floor: AppliedPosition {
                term: u64::from_be_bytes(bytes[80..88].try_into().unwrap()),
                index: u64::from_be_bytes(bytes[88..96].try_into().unwrap()),
            },
        };
        if decision.root.as_bytes() == &[0; 32]
            || decision.operation == [0; 16]
            || decision.task < FIRST_DYNAMIC_ID
            || decision.evidence_task < FIRST_DYNAMIC_ID
            || decision.task == decision.evidence_task
            || decision.region.0 < FIRST_DYNAMIC_ID
            || decision.floor.term == 0
            || decision.floor.index == 0
        {
            return Err(invalid("invalid truncation decision identities"));
        }
        Ok(decision)
    }
}

fn from_truncation_row(row: &Row, root: RootDigest) -> Result<Option<TruncationDecision>> {
    if row.value.get(ColumnId(2)) != Some(&ColumnValue::Uint(SOURCE_TRUNCATION)) {
        return Ok(None);
    }
    let Some(ColumnValue::Bytes(bytes)) = row.value.get(ColumnId(3)) else {
        return Err(invalid("truncation decision payload is missing"));
    };
    let decision = TruncationDecision::decode(bytes)?;
    if decision.root != root
        || row.pk != [memcmp_uint(decision.task)]
        || row.value.get(ColumnId(1)) != Some(&ColumnValue::Uint(decision.task))
        || row.value.get(ColumnId(4)) != Some(&ColumnValue::Uint(0))
        || row.value.get(ColumnId(5)) != Some(&ColumnValue::Uint(0))
    {
        return Err(invalid("truncation decision row binding differs"));
    }
    Ok(Some(decision))
}

fn matches_evidence(
    decision: &TruncationDecision,
    evidence: &super::evidence::InstallEvidence,
) -> Result<()> {
    if evidence.task() != decision.evidence_task
        || evidence.operation() != decision.operation
        || evidence.root() != decision.root
        || evidence.region() != decision.region
    {
        return Err(invalid("decision differs from the committed evidence"));
    }
    // The installed image carries everything at or below the evidence cut;
    // a floor above it could strand the very learner the evidence names.
    if decision.floor.index > evidence.cut().index {
        return Err(invalid("truncation floor exceeds the evidence cut"));
    }
    if decision.floor == evidence.cut() && decision.floor.term != evidence.cut().term {
        return Err(invalid(
            "truncation floor term differs from the evidence cut",
        ));
    }
    Ok(())
}

fn matches_abort(
    decision: &TruncationDecision,
    abort: &super::abort::MigrationAbort,
) -> Result<()> {
    if abort.task() != decision.evidence_task
        || abort.operation() != decision.operation
        || abort.root() != decision.root
        || abort.region() != decision.region
    {
        return Err(invalid("decision differs from the committed abort"));
    }
    // An aborted operation has no installed image and no evidence cut: the
    // detached learner no longer consumes the tail. The floor's guard is
    // the raft-level compaction gate (leader-only, every voter matched at
    // or beyond the floor, local prefix only) plus the released source pin.
    Ok(())
}

/// Stage one idempotent truncation decision in the serialized, term-fenced
/// catalog transaction. `false` means the exact decision already exists and
/// this commit is a confirmation receipt. The committed evidence row is the
/// authority the floor is bounded by; nothing here compacts a log.
pub fn plan_truncation<E: Engine>(
    txn: &mut MetaTxn<'_, E>,
    operation: [u8; 16],
    floor: AppliedPosition,
) -> Result<(TruncationDecision, bool)> {
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("truncation requires a certified metadata root"))?;
    let rows = txn.scan(&TASKS_DESC, MAX_CREATION_TASKS + 1)?;
    if rows.len() > MAX_CREATION_TASKS {
        return Err(invalid("bounded truncation task scan exceeded"));
    }
    // The committed SETTLEMENT is the authority: install evidence (the
    // transfer completed; the floor stays at or below its cut) or a
    // committed abort (the operation was abandoned and its learner
    // detached; the raft compaction gate guards the floor). Exactly one
    // exists per operation — evidence and abort are mutually exclusive.
    let mut evidence = None;
    let mut abort = None;
    for row in &rows {
        if let Some(committed) = super::evidence::from_row_for_siblings(row, root.digest())? {
            if committed.operation() == operation {
                evidence = Some(committed);
            }
        }
        if let Some(committed) = super::abort::from_abort_row(row, root.digest())? {
            if committed.operation() == operation {
                abort = Some(committed);
            }
        }
    }
    let (settlement_task, region) = match (&evidence, &abort) {
        (Some(evidence), None) => (evidence.task(), evidence.region()),
        (None, Some(abort)) => (abort.task(), abort.region()),
        (Some(_), Some(_)) => {
            return Err(invalid("an operation carries both evidence and an abort"))
        }
        (None, None) => {
            return Err(invalid(
                "truncation requires the committed install evidence",
            ))
        }
    };
    let decision = TruncationDecision {
        root: root.digest(),
        operation,
        task: 0,
        evidence_task: settlement_task,
        region,
        floor,
    };
    for row in &rows {
        if let Some(previous) = from_truncation_row(row, root.digest())? {
            if previous.operation == operation {
                let expected = TruncationDecision {
                    task: previous.task,
                    ..decision.clone()
                };
                if expected != previous {
                    return Err(invalid("truncation operation binding conflicts"));
                }
                return Ok((previous, false));
            }
            if previous.region == region {
                return Err(invalid("a committed decision already binds this group"));
            }
        }
    }
    let decision = TruncationDecision {
        task: txn.allocate_id(SequenceKind::Task)?,
        ..decision
    };
    match (&evidence, &abort) {
        (Some(evidence), None) => matches_evidence(&decision, evidence)?,
        (None, Some(abort)) => matches_abort(&decision, abort)?,
        _ => unreachable!("settlement resolved above"),
    }
    TruncationDecision::decode(&decision.encode())?;
    if rows.len() == MAX_CREATION_TASKS {
        return Err(invalid("truncation task capacity reached"));
    }
    let mut row = RowValue::new();
    for (column, value) in [
        (ColumnId(1), ColumnValue::Uint(decision.task)),
        (ColumnId(2), ColumnValue::Uint(SOURCE_TRUNCATION)),
        (ColumnId(3), ColumnValue::Bytes(decision.encode())),
        (ColumnId(4), ColumnValue::Uint(0)),
        (ColumnId(5), ColumnValue::Uint(0)),
    ] {
        row.set(column, value);
    }
    txn.insert(&TASKS_DESC, &[memcmp_uint(decision.task)], row)?;
    Ok((decision, true))
}

/// Read every committed truncation decision from one fresh applied snapshot,
/// with the exact evidence rows re-validated. Decisions never expire.
pub fn committed_truncations<E: Engine>(store: &MetaStore<E>) -> Result<Vec<TruncationDecision>> {
    let txn = store.begin()?;
    let root = crate::root::certified_root(&txn)?
        .ok_or_else(|| invalid("truncation readback requires a certified metadata root"))?;
    let rows = txn.scan(&TASKS_DESC, MAX_CREATION_TASKS + 1)?;
    if rows.len() > MAX_CREATION_TASKS {
        return Err(invalid("bounded truncation task scan exceeded"));
    }
    let mut out: Vec<TruncationDecision> = Vec::new();
    for row in &rows {
        if let Some(decision) = from_truncation_row(row, root.digest())? {
            let mut evidence = None;
            let mut abort = None;
            for other in &rows {
                if let Some(committed) =
                    super::evidence::from_row_for_siblings(other, root.digest())?
                {
                    if committed.operation() == decision.operation {
                        evidence = Some(committed);
                    }
                }
                if let Some(committed) = super::abort::from_abort_row(other, root.digest())? {
                    if committed.operation() == decision.operation {
                        abort = Some(committed);
                    }
                }
            }
            match (&evidence, &abort) {
                (Some(evidence), None) => matches_evidence(&decision, evidence)?,
                (None, Some(abort)) => matches_abort(&decision, abort)?,
                (Some(_), Some(_)) => {
                    return Err(invalid("an operation carries both evidence and an abort"))
                }
                (None, None) => {
                    return Err(invalid(
                        "truncation requires the committed install evidence",
                    ))
                }
            }
            if out.iter().any(|prior| {
                prior.task == decision.task
                    || prior.operation == decision.operation
                    || prior.region == decision.region
            }) {
                return Err(invalid("duplicate truncation identity"));
            }
            out.push(decision);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::NODES_DESC;

    fn register_node(store: &MetaStore<kv9_engine::MemEngine>, node: u64, incarnation: [u8; 16]) {
        let mut txn = store.begin().unwrap();
        let mut row = RowValue::new();
        row.set(
            ColumnId(2),
            ColumnValue::Text(format!("127.0.0.1:{}", 18000 + node)),
        );
        row.set(ColumnId(3), ColumnValue::Uint(2));
        row.set(ColumnId(4), ColumnValue::Uint(0));
        row.set(ColumnId(5), ColumnValue::Bytes(incarnation.to_vec()));
        txn.insert(&NODES_DESC, &[memcmp_uint(node)], row).unwrap();
        txn.commit().unwrap();
    }

    fn committed_evidence_row(store: &MetaStore<kv9_engine::MemEngine>) -> ([u8; 16], u64) {
        register_node(store, 4, [40; 16]);
        let mut txn = store.begin().unwrap();
        let creation =
            plan_empty_group(&mut txn, [1; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        super::super::activation::plan_activation(&mut txn, &creation).unwrap();
        let (migration, _) =
            super::super::migration::plan_migration(&mut txn, [2; 16], creation.task(), NodeId(4))
                .unwrap();
        let receipt = super::super::evidence::InstallEvidence::receipt(
            &migration,
            StoreIncarnation::from_bytes([41; 16]),
            RootDigest::sha256(b"image"),
            [42; 32],
            AppliedPosition { term: 3, index: 12 },
        )
        .unwrap();
        let (evidence, _) =
            super::super::evidence::plan_install_evidence(&mut txn, &receipt).unwrap();
        txn.commit().unwrap();
        ([2; 16], evidence.cut().index)
    }

    #[test]
    fn a_decision_requires_committed_evidence_and_stays_below_its_cut() {
        let store = super::super::tests::fixture();
        {
            // No evidence at all: refuse.
            let mut txn = store.begin().unwrap();
            assert!(
                plan_truncation(&mut txn, [2; 16], AppliedPosition { term: 3, index: 5 }).is_err()
            );
        }
        let (operation, cut) = committed_evidence_row(&store);
        let mut txn = store.begin().unwrap();
        assert!(
            plan_truncation(
                &mut txn,
                operation,
                AppliedPosition {
                    term: 3,
                    index: cut + 1
                }
            )
            .is_err(),
            "a floor above the evidence cut can strand the learner"
        );
        assert!(
            plan_truncation(&mut txn, [9; 16], AppliedPosition { term: 3, index: 5 }).is_err(),
            "a different operation has no evidence"
        );
        let floor = AppliedPosition {
            term: 3,
            index: cut,
        };
        let (planned, changed) = plan_truncation(&mut txn, operation, floor).unwrap();
        assert!(changed);
        assert_eq!(planned.floor(), floor);
        assert!(committed_truncations(&store).unwrap().is_empty());
        txn.commit().unwrap();
        assert_eq!(
            committed_truncations(&store).unwrap(),
            vec![planned.clone()]
        );
        // Idempotent confirm; a different floor conflicts.
        let mut txn = store.begin().unwrap();
        let (retry, changed) = plan_truncation(&mut txn, operation, floor).unwrap();
        assert!(!changed);
        assert_eq!(retry, planned);
        assert!(
            plan_truncation(&mut txn, operation, AppliedPosition { term: 3, index: 1 }).is_err(),
            "one operation never authorizes a second floor"
        );
    }

    #[test]
    fn truncation_readback_refuses_row_and_evidence_rebinding() {
        for defect in [
            "task",
            "state",
            "created",
            "truncated",
            "operation",
            "floor",
        ] {
            let store = super::super::tests::fixture();
            let (operation, cut) = committed_evidence_row(&store);
            let mut txn = store.begin().unwrap();
            let (planned, _) = plan_truncation(
                &mut txn,
                operation,
                AppliedPosition {
                    term: 3,
                    index: cut,
                },
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
                    ColumnValue::Bytes(planned.encode()[..80].to_vec()),
                ),
                "operation" | "floor" => {
                    let mut bytes = planned.encode();
                    bytes[if defect == "operation" { 40 } else { 95 }] ^= 1;
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
            assert!(committed_truncations(&store).is_err(), "accepted {defect}");
        }
    }

    #[test]
    fn an_abort_settles_truncation_without_an_evidence_cut() {
        let store = super::super::tests::fixture();
        register_node(&store, 4, [40; 16]);
        let mut txn = store.begin().unwrap();
        let creation =
            plan_empty_group(&mut txn, [1; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        super::super::activation::plan_activation(&mut txn, &creation).unwrap();
        let (migration, _) =
            super::super::migration::plan_migration(&mut txn, [2; 16], creation.task(), NodeId(4))
                .unwrap();
        let floor = AppliedPosition { term: 5, index: 40 };
        assert!(
            plan_truncation(&mut txn, migration.operation(), floor).is_err(),
            "an unsettled operation must refuse"
        );
        super::super::abort::plan_abort(&mut txn, migration.operation(), [42; 32]).unwrap();
        txn.commit().unwrap();
        let mut txn = store.begin().unwrap();
        let (planned, changed) = plan_truncation(&mut txn, migration.operation(), floor).unwrap();
        assert!(changed);
        assert_eq!(planned.region(), migration.region());
        assert_eq!(planned.floor(), floor);
        txn.commit().unwrap();
        assert_eq!(
            committed_truncations(&store).unwrap(),
            vec![planned.clone()]
        );
        let mut txn = store.begin().unwrap();
        let (retry, changed) = plan_truncation(&mut txn, migration.operation(), floor).unwrap();
        assert!(!changed);
        assert_eq!(retry, planned);
        assert!(plan_truncation(
            &mut txn,
            migration.operation(),
            AppliedPosition { term: 5, index: 41 }
        )
        .is_err());
    }
}
