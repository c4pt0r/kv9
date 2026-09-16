//! Durable activation desire, separate from immutable preparation authority.
//!
//! Creation and activation rows share the existing 255-row task budget. An
//! online group uses two rows; preparation-only callers retain their semantics.
//! No routing row is published and no request receipt claims quorum readiness.

use super::*;

const ACTIVATE_EMPTY_GROUP: u64 = 101;

/// A fresh, applied-engine readback of both exact immutable rows. Decoding an
/// RPC or a planner overlay cannot mint this capability.
#[derive(Debug, Clone)]
pub struct CommittedActivation(CommittedCreation);

impl CommittedActivation {
    pub fn creation(&self) -> &CommittedCreation {
        &self.0
    }
}

fn rows<E: Engine>(txn: &MetaTxn<'_, E>) -> Result<Vec<Row>> {
    let rows = txn.scan(&TASKS_DESC, MAX_CREATION_TASKS + 1)?;
    if rows.len() > MAX_CREATION_TASKS {
        return Err(invalid("bounded activation task scan exceeded"));
    }
    Ok(rows)
}

fn decode<E: Engine>(
    txn: &MetaTxn<'_, E>,
    rows: &[Row],
    root: RootDigest,
) -> Result<Vec<CreationIntent>> {
    let mut intents = Vec::new();
    for row in rows {
        if row.value.get(ColumnId(2)) != Some(&ColumnValue::Uint(ACTIVATE_EMPTY_GROUP)) {
            continue;
        }
        let Some(ColumnValue::Uint(task)) = row.value.get(ColumnId(1)) else {
            return Err(invalid("activation task identity is missing"));
        };
        let Some(ColumnValue::Bytes(bytes)) = row.value.get(ColumnId(3)) else {
            return Err(invalid("activation task payload is missing"));
        };
        let intent = CreationIntent::decode(bytes)?;
        if *task < FIRST_DYNAMIC_ID
            || *task == intent.task()
            || row.pk != [memcmp_uint(*task)]
            || row.value.get(ColumnId(4)) != Some(&ColumnValue::Uint(0))
            || row.value.get(ColumnId(5)) != Some(&ColumnValue::Uint(0))
        {
            return Err(invalid("activation task row binding differs"));
        }
        let creation = txn
            .get(&TASKS_DESC, &[memcmp_uint(intent.task())])?
            .ok_or_else(|| invalid("activation creation is missing"))?;
        if from_row(&creation, root)?.as_ref() != Some(&intent) {
            return Err(invalid("activation differs from exact creation"));
        }
        if intents.iter().any(|prior: &CreationIntent| {
            prior.task() == intent.task()
                || prior.region() == intent.region()
                || prior.operation == intent.operation
        }) {
            return Err(invalid("duplicate activation identity"));
        }
        intents.push(intent);
    }
    Ok(intents)
}

/// Stage one idempotent desire in the same serialized, term-fenced catalog
/// transaction as creation. On failure the caller must discard the whole txn.
/// `false` means an existing desire; its new commit is a confirmation receipt.
pub fn plan_activation<E: Engine>(
    txn: &mut MetaTxn<'_, E>,
    intent: &CreationIntent,
) -> Result<bool> {
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("activation requires a certified metadata root"))?;
    let creation = txn
        .get(&TASKS_DESC, &[memcmp_uint(intent.task())])?
        .ok_or_else(|| invalid("activation creation is missing"))?;
    if from_row(&creation, root.digest())?.as_ref() != Some(intent) {
        return Err(invalid("activation differs from exact creation"));
    }
    let rows = rows(txn)?;
    if decode(txn, &rows, root.digest())?.contains(intent) {
        return Ok(false);
    }
    if rows.len() == MAX_CREATION_TASKS {
        return Err(invalid("activation task capacity reached"));
    }
    let task = txn.allocate_id(SequenceKind::Task)?;
    let mut row = RowValue::new();
    for (column, value) in [
        (ColumnId(1), ColumnValue::Uint(task)),
        (ColumnId(2), ColumnValue::Uint(ACTIVATE_EMPTY_GROUP)),
        (ColumnId(3), ColumnValue::Bytes(intent.encode())),
        (ColumnId(4), ColumnValue::Uint(0)),
        (ColumnId(5), ColumnValue::Uint(0)),
    ] {
        row.set(column, value);
    }
    txn.insert(&TASKS_DESC, &[memcmp_uint(task)], row)?;
    Ok(true)
}

/// Read both rows from one fresh applied snapshot. Immutable desires do not
/// expire or cancel; followers can reconcile without a control-plane leader.
pub fn committed_activations<E: Engine>(store: &MetaStore<E>) -> Result<Vec<CommittedActivation>> {
    let txn = store.begin()?;
    let root = crate::root::certified_root(&txn)?
        .ok_or_else(|| invalid("activation readback requires a certified metadata root"))?;
    Ok(decode(&txn, &rows(&txn)?, root.digest())?
        .into_iter()
        .map(|intent| CommittedActivation(CommittedCreation(intent)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_readback_refuses_row_and_creation_rebinding() {
        for defect in [
            "task",
            "state",
            "created",
            "truncated",
            "root",
            "operation",
            "duplicate",
        ] {
            let store = super::super::tests::fixture();
            let mut txn = store.begin().unwrap();
            let intent =
                plan_empty_group(&mut txn, [91; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
            plan_activation(&mut txn, &intent).unwrap();
            txn.commit().unwrap();
            let mut txn = store.begin().unwrap();
            let request = rows(&txn)
                .unwrap()
                .into_iter()
                .find(|r| {
                    r.value.get(ColumnId(2)) == Some(&ColumnValue::Uint(ACTIVATE_EMPTY_GROUP))
                })
                .unwrap();
            let change = match defect {
                "task" => {
                    let mut malformed = request.clone();
                    malformed.value.set(ColumnId(1), ColumnValue::Uint(999));
                    assert!(decode(&txn, &[malformed], intent.root()).is_err());
                    assert!(
                        txn.update(
                            &TASKS_DESC,
                            &request.pk,
                            vec![(ColumnId(1), ColumnValue::Uint(999))]
                        )
                        .is_err(),
                        "the lower catalog layer must also reject a rebound primary key"
                    );
                    continue;
                }
                "state" => (ColumnId(4), ColumnValue::Uint(1)),
                "created" => (ColumnId(5), ColumnValue::Uint(1)),
                "truncated" => (
                    ColumnId(3),
                    ColumnValue::Bytes(intent.encode()[..100].to_vec()),
                ),
                "root" | "operation" => {
                    let mut bytes = intent.encode();
                    bytes[if defect == "root" { 8 } else { 40 }] ^= 1;
                    (ColumnId(3), ColumnValue::Bytes(bytes))
                }
                "duplicate" => {
                    let task = txn.allocate_id(SequenceKind::Task).unwrap();
                    let mut row = request.value.clone();
                    row.set(ColumnId(1), ColumnValue::Uint(task));
                    txn.insert(&TASKS_DESC, &[memcmp_uint(task)], row).unwrap();
                    txn.commit().unwrap();
                    assert!(committed_activations(&store).is_err(), "accepted {defect}");
                    continue;
                }
                _ => unreachable!(),
            };
            txn.update(&TASKS_DESC, &request.pk, vec![change]).unwrap();
            txn.commit().unwrap();
            assert!(committed_activations(&store).is_err(), "accepted {defect}");
        }
    }

    #[test]
    fn activation_requires_committed_exact_pair_and_keeps_preparation_immutable() {
        let store = super::super::tests::fixture();
        let mut txn = store.begin().unwrap();
        let intent =
            plan_empty_group(&mut txn, [91; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        txn.commit().unwrap();
        assert!(committed_activations(&store).unwrap().is_empty());
        let mut txn = store.begin().unwrap();
        assert!(plan_activation(&mut txn, &intent).unwrap());
        assert!(committed_activations(&store).unwrap().is_empty());
        txn.commit().unwrap();
        assert_eq!(
            committed_activations(&store).unwrap()[0]
                .creation()
                .intent(),
            &intent
        );
        assert_eq!(
            committed_creation(&store, intent.task())
                .unwrap()
                .unwrap()
                .intent(),
            &intent
        );
        let mut txn = store.begin().unwrap();
        assert!(!plan_activation(&mut txn, &intent).unwrap());
        txn.commit().unwrap();
        assert_eq!(rows(&store.begin().unwrap()).unwrap().len(), 2);
        // Binding a valid desire to a changed creation must fail closed.
        let mut txn = store.begin().unwrap();
        txn.update(
            &TASKS_DESC,
            &[memcmp_uint(intent.task())],
            vec![(ColumnId(4), ColumnValue::Uint(1))],
        )
        .unwrap();
        txn.commit().unwrap();
        assert!(committed_activations(&store).is_err());
    }

    #[test]
    fn activation_retries_at_capacity_and_discards_partial_creation() {
        let store = super::super::tests::fixture();
        let mut txn = store.begin().unwrap();
        let first =
            plan_empty_group(&mut txn, [1; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        plan_activation(&mut txn, &first).unwrap();
        // 254 occupied rows leave room for creation but not its desire.
        for id in 2..=253 {
            plan_empty_group(&mut txn, [id; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        }
        txn.commit().unwrap();
        let mut txn = store.begin().unwrap();
        let uncommitted =
            plan_empty_group(&mut txn, [254; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        assert!(plan_activation(&mut txn, &uncommitted).is_err());
        drop(txn);
        assert!(committed_creation(&store, uncommitted.task())
            .unwrap()
            .is_none());
        let mut txn = store.begin().unwrap();
        plan_empty_group(&mut txn, [254; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        txn.commit().unwrap();
        let mut txn = store.begin().unwrap();
        assert_eq!(
            plan_empty_group(&mut txn, [1; 16], &[NodeId(3), NodeId(2), NodeId(1)]).unwrap(),
            first
        );
        assert!(!plan_activation(&mut txn, &first).unwrap());
        txn.commit().unwrap();
        assert_eq!(committed_activations(&store).unwrap().len(), 1);
    }
}
