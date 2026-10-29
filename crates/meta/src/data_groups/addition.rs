//! Committed replica plans: the one row that makes the cluster drive a replica
//! addition or move by itself (docs/REPLICA-ADDITION.md §1).
//!
//! A plan names the migration operation it drives (the kind-103 intent is committed
//! in the same transaction), the group, the mode — `Add` grows the group by the
//! destination; `Move` additionally retires one exact initial replica — and, for a
//! move, that source replica. It grants no capability of its own: every step the
//! driver takes is one of the existing verbs with its own committed prerequisite.
//! One plan per operation; one live plan per group; an exact resubmission confirms;
//! any divergence refuses; the row is immutable.

use super::*;

const REPLICA_PLAN: u64 = 111;
const ADD_MAGIC: &[u8; 8] = b"KV9ADD01";
pub const MAX_PLAN_BYTES: usize = 8 + 32 + 16 + 8 + 8 + 8 + 1 + 8 + 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanMode {
    Add = 1,
    Move = 2,
}

impl PlanMode {
    fn from_code(code: u8) -> Result<Self> {
        match code {
            1 => Ok(Self::Add),
            2 => Ok(Self::Move),
            _ => Err(invalid("unknown replica plan mode")),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Move => "move",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaPlan {
    root: RootDigest,
    operation: [u8; 16],
    task: u64,
    migration_task: u64,
    region: RegionId,
    mode: PlanMode,
    /// The replica a move retires; all-zero for an addition.
    source: InitialReplica,
}

impl ReplicaPlan {
    pub fn operation(&self) -> [u8; 16] {
        self.operation
    }
    pub fn root(&self) -> RootDigest {
        self.root
    }
    pub fn task(&self) -> u64 {
        self.task
    }
    pub fn migration_task(&self) -> u64 {
        self.migration_task
    }
    pub fn region(&self) -> RegionId {
        self.region
    }
    pub fn mode(&self) -> PlanMode {
        self.mode
    }
    /// `Some` only for a move.
    pub fn source(&self) -> Option<InitialReplica> {
        (self.mode == PlanMode::Move).then_some(self.source)
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(MAX_PLAN_BYTES);
        bytes.extend_from_slice(ADD_MAGIC);
        bytes.extend_from_slice(self.root.as_bytes());
        bytes.extend_from_slice(&self.operation);
        bytes.extend_from_slice(&self.task.to_be_bytes());
        bytes.extend_from_slice(&self.migration_task.to_be_bytes());
        bytes.extend_from_slice(&self.region.0.to_be_bytes());
        bytes.push(self.mode as u8);
        bytes.extend_from_slice(&self.source.node.0.to_be_bytes());
        bytes.extend_from_slice(self.source.incarnation.as_bytes());
        bytes
    }

    /// Decode data only. This does not mint committed authority.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != MAX_PLAN_BYTES || &bytes[..8] != ADD_MAGIC {
            return Err(invalid("invalid replica plan encoding"));
        }
        let plan = Self {
            root: RootDigest::from_bytes(bytes[8..40].try_into().unwrap()),
            operation: bytes[40..56].try_into().unwrap(),
            task: u64::from_be_bytes(bytes[56..64].try_into().unwrap()),
            migration_task: u64::from_be_bytes(bytes[64..72].try_into().unwrap()),
            region: RegionId(u64::from_be_bytes(bytes[72..80].try_into().unwrap())),
            mode: PlanMode::from_code(bytes[80])?,
            source: InitialReplica {
                node: NodeId(u64::from_be_bytes(bytes[81..89].try_into().unwrap())),
                incarnation: StoreIncarnation::from_bytes(bytes[89..105].try_into().unwrap()),
            },
        };
        let zero_source = plan.source.node.0 == 0 && plan.source.incarnation.as_bytes() == &[0; 16];
        if plan.root.as_bytes() == &[0; 32]
            || plan.operation == [0; 16]
            || plan.task < FIRST_DYNAMIC_ID
            || plan.migration_task < FIRST_DYNAMIC_ID
            || plan.task == plan.migration_task
            || plan.region.0 < FIRST_DYNAMIC_ID
            || (plan.mode == PlanMode::Add) != zero_source
            || (plan.mode == PlanMode::Move
                && (plan.source.node.0 == 0 || plan.source.incarnation.as_bytes() == &[0; 16]))
        {
            return Err(invalid("invalid replica plan identities"));
        }
        Ok(plan)
    }
}

pub(crate) fn from_plan_row(row: &Row, root: RootDigest) -> Result<Option<ReplicaPlan>> {
    if row.value.get(ColumnId(2)) != Some(&ColumnValue::Uint(REPLICA_PLAN)) {
        return Ok(None);
    }
    let Some(ColumnValue::Bytes(bytes)) = row.value.get(ColumnId(3)) else {
        return Err(invalid("replica plan payload is missing"));
    };
    let plan = ReplicaPlan::decode(bytes)?;
    if plan.root != root
        || row.pk != [memcmp_uint(plan.task)]
        || row.value.get(ColumnId(1)) != Some(&ColumnValue::Uint(plan.task))
        || row.value.get(ColumnId(4)) != Some(&ColumnValue::Uint(0))
        || row.value.get(ColumnId(5)) != Some(&ColumnValue::Uint(0))
    {
        return Err(invalid("replica plan row binding differs"));
    }
    Ok(Some(plan))
}

fn matches_authority(
    plan: &ReplicaPlan,
    migration: &super::migration::MigrationIntent,
    members: &[(InitialReplica, u64)],
) -> Result<()> {
    if migration.task() != plan.migration_task
        || migration.operation() != plan.operation
        || migration.root() != plan.root
        || migration.region() != plan.region
    {
        return Err(invalid("plan differs from the committed migration"));
    }
    if let Some(source) = plan.source() {
        // The source is a committed member when the plan is written: an initial
        // replica or an evidenced destination not yet retired.
        if !members.iter().any(|(member, _)| *member == source) {
            return Err(invalid("move names a replica that is not a committed member"));
        }
        if source.node == migration.destination().node {
            return Err(invalid("move cannot retire the destination"));
        }
    }
    Ok(())
}

/// A plan is terminal, as far as the catalog can tell, when the row that ends its
/// chain exists: evidence for an addition, the removal decision for a move. The live
/// voter facts complete the picture at the group (docs/REPLICA-ADDITION.md §3).
fn catalog_terminal<E: Engine>(txn: &MetaTxn<'_, E>, root: RootDigest, plan: &ReplicaPlan) -> Result<bool> {
    for row in task_rows(txn) {
        let row = row?;
        match plan.mode {
            PlanMode::Add => {
                if super::evidence::from_row_for_siblings(&row, root)?
                    .is_some_and(|e| e.operation() == plan.operation)
                {
                    return Ok(true);
                }
            }
            PlanMode::Move => {
                if super::removal::from_removal_row(&row, root)?
                    .is_some_and(|d| d.operation() == plan.operation)
                {
                    return Ok(true);
                }
            }
        }
        if super::abort::from_abort_row(&row, root)?.is_some_and(|a| a.operation() == plan.operation) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Stage one idempotent replica plan in the serialized, term-fenced catalog
/// transaction, after `plan_migration` staged (or confirmed) the intent for the
/// same operation in the SAME transaction. `false` means the exact plan already
/// exists and this commit is a confirmation receipt.
pub fn plan_replica<E: Engine>(
    txn: &mut MetaTxn<'_, E>,
    operation: [u8; 16],
    mode: PlanMode,
    source: Option<InitialReplica>,
) -> Result<(ReplicaPlan, bool)> {
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("replica plan requires a certified metadata root"))?;
    let migration = super::migration::committed_migrations_in_txn(txn, root.digest())?
        .into_iter()
        .find(|m| m.operation() == operation)
        .ok_or_else(|| invalid("replica plan requires the committed migration intent"))?;
    let creation_row = txn
        .get(&TASKS_DESC, &[memcmp_uint(migration.creation_task())])?
        .ok_or_else(|| invalid("replica plan creation is missing"))?;
    let creation = from_row(&creation_row, root.digest())?
        .ok_or_else(|| invalid("replica plan creation row differs"))?;
    let source = match (mode, source) {
        (PlanMode::Add, None) => InitialReplica {
            node: NodeId(0),
            incarnation: StoreIncarnation::from_bytes([0; 16]),
        },
        (PlanMode::Move, Some(source)) => source,
        _ => return Err(invalid("a move names its source replica; an addition names none")),
    };
    let plan = ReplicaPlan {
        root: root.digest(),
        operation,
        task: 0,
        migration_task: migration.task(),
        region: migration.region(),
        mode,
        source,
    };
    for row in task_rows(txn) {
        let row = row?;
        if let Some(previous) = from_plan_row(&row, root.digest())? {
            if previous.operation == operation {
                let expected = ReplicaPlan {
                    task: previous.task,
                    ..plan.clone()
                };
                if expected != previous {
                    return Err(invalid("replica plan operation binding conflicts"));
                }
                return Ok((previous, false));
            }
            if previous.region == plan.region && !catalog_terminal(txn, root.digest(), &previous)? {
                return Err(invalid("a live replica plan already drives this group"));
            }
        }
    }
    let plan = ReplicaPlan {
        task: txn.allocate_id(SequenceKind::Task)?,
        ..plan
    };
    let members = super::committed_members(txn, root.digest(), &creation)?;
    matches_authority(&plan, &migration, &members)?;
    ReplicaPlan::decode(&plan.encode())?;
    let mut row = RowValue::new();
    for (column, value) in [
        (ColumnId(1), ColumnValue::Uint(plan.task)),
        (ColumnId(2), ColumnValue::Uint(REPLICA_PLAN)),
        (ColumnId(3), ColumnValue::Bytes(plan.encode())),
        (ColumnId(4), ColumnValue::Uint(0)),
        (ColumnId(5), ColumnValue::Uint(0)),
    ] {
        row.set(column, value);
    }
    txn.insert(&TASKS_DESC, &[memcmp_uint(plan.task)], row)?;
    Ok((plan, true))
}

/// Read every committed replica plan from one fresh applied snapshot, with the
/// migration and creation rows re-validated. Plans never expire.
pub fn committed_plans<E: Engine>(store: &MetaStore<E>) -> Result<Vec<ReplicaPlan>> {
    let txn = store.begin()?;
    let root = crate::root::certified_root(&txn)?
        .ok_or_else(|| invalid("replica plan readback requires a certified metadata root"))?;
    let migrations = super::migration::committed_migrations_in_txn(&txn, root.digest())?;
    let mut out: Vec<ReplicaPlan> = Vec::new();
    for row in task_rows(&txn) {
        let row = row?;
        if let Some(plan) = from_plan_row(&row, root.digest())? {
            let migration = migrations
                .iter()
                .find(|m| m.operation() == plan.operation)
                .ok_or_else(|| invalid("replica plan requires the committed migration"))?;
            let creation_row = txn
                .get(&TASKS_DESC, &[memcmp_uint(migration.creation_task())])?
                .ok_or_else(|| invalid("replica plan creation is missing"))?;
            let creation = from_row(&creation_row, root.digest())?
                .ok_or_else(|| invalid("replica plan creation row differs"))?;
            // Readback validates against the membership at plan time or later: a
            // member retired by THIS plan's own decision was a member when planned.
            let mut members = super::committed_members(&txn, root.digest(), &creation)?;
            if let Some(source) = plan.source() {
                members.push((source, u64::MAX));
            }
            matches_authority(&plan, migration, &members)?;
            if out
                .iter()
                .any(|prior| prior.task == plan.task || prior.operation == plan.operation)
            {
                return Err(invalid("duplicate replica plan identity"));
            }
            out.push(plan);
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

    /// A committed creation with destination 4 registered; the caller plans the
    /// migration intent and the plan in one transaction, as `AddReplica` does.
    fn group(store: &MetaStore<kv9_engine::MemEngine>) -> CreationIntent {
        register_node(store, 4, [40; 16]);
        let mut txn = store.begin().unwrap();
        let creation =
            plan_empty_group(&mut txn, [1; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        super::super::activation::plan_activation(&mut txn, &creation).unwrap();
        txn.commit().unwrap();
        creation
    }

    #[test]
    fn a_plan_needs_the_intent_in_the_same_transaction_and_confirms_exactly() {
        let store = super::super::tests::fixture();
        let creation = group(&store);
        {
            let mut txn = store.begin().unwrap();
            assert!(
                plan_replica(&mut txn, [2; 16], PlanMode::Add, None).is_err(),
                "no committed or staged intent: refuse"
            );
        }
        let mut txn = store.begin().unwrap();
        let (migration, _) =
            super::super::migration::plan_migration(&mut txn, [2; 16], creation.task(), NodeId(4))
                .unwrap();
        let (plan, changed) = plan_replica(&mut txn, [2; 16], PlanMode::Add, None).unwrap();
        assert!(changed);
        assert_eq!(plan.migration_task(), migration.task());
        assert_eq!(plan.region(), creation.region());
        assert_eq!(plan.mode(), PlanMode::Add);
        assert_eq!(plan.source(), None);
        txn.commit().unwrap();

        // Identical resubmission confirms; a changed mode diverges and refuses.
        let mut txn = store.begin().unwrap();
        let (again, changed) = plan_replica(&mut txn, [2; 16], PlanMode::Add, None).unwrap();
        assert!(!changed);
        assert_eq!(again, plan);
        let source = creation.replicas()[0];
        assert!(plan_replica(&mut txn, [2; 16], PlanMode::Move, Some(source)).is_err());
        drop(txn);

        let plans = committed_plans(&store).unwrap();
        assert_eq!(plans, vec![plan.clone()]);
        assert_eq!(ReplicaPlan::decode(&plan.encode()).unwrap(), plan);
    }

    #[test]
    fn a_move_names_an_initial_replica_that_is_not_the_destination_and_one_plan_drives_a_group() {
        let store = super::super::tests::fixture();
        let creation = group(&store);
        register_node(&store, 5, [50; 16]);
        let mut txn = store.begin().unwrap();
        super::super::migration::plan_migration(&mut txn, [2; 16], creation.task(), NodeId(4))
            .unwrap();
        let outsider = InitialReplica {
            node: NodeId(7),
            incarnation: StoreIncarnation::from_bytes([70; 16]),
        };
        assert!(plan_replica(&mut txn, [2; 16], PlanMode::Move, Some(outsider)).is_err());
        let wrong_incarnation = InitialReplica {
            node: creation.replicas()[0].node,
            incarnation: StoreIncarnation::from_bytes([99; 16]),
        };
        assert!(plan_replica(&mut txn, [2; 16], PlanMode::Move, Some(wrong_incarnation)).is_err());
        assert!(
            plan_replica(&mut txn, [2; 16], PlanMode::Move, None).is_err(),
            "a move names its source"
        );
        assert!(
            plan_replica(&mut txn, [2; 16], PlanMode::Add, Some(creation.replicas()[0])).is_err(),
            "an addition names none"
        );
        let (plan, changed) =
            plan_replica(&mut txn, [2; 16], PlanMode::Move, Some(creation.replicas()[0])).unwrap();
        assert!(changed);
        assert_eq!(plan.source(), Some(creation.replicas()[0]));
        txn.commit().unwrap();

        // A second intent for the same group refuses while the first is live.
        let mut txn = store.begin().unwrap();
        assert!(
            super::super::migration::plan_migration(&mut txn, [3; 16], creation.task(), NodeId(5))
                .is_err(),
            "one operation at a time binds a group"
        );
        drop(txn);

        // Once the first plan is terminal in the catalog (its removal decision
        // exists), the group accepts a new plan.
        let mut txn = store.begin().unwrap();
        let migration = super::super::migration::committed_migrations_in_txn(&txn, plan.root())
            .unwrap()
            .into_iter()
            .find(|m| m.operation() == [2; 16])
            .unwrap();
        let receipt = super::super::evidence::InstallEvidence::receipt(
            &migration,
            StoreIncarnation::from_bytes([41; 16]),
            RootDigest::sha256(b"image"),
            [42; 32],
            kv9_common::AppliedPosition { term: 3, index: 12 },
        )
        .unwrap();
        super::super::evidence::plan_install_evidence(&mut txn, &receipt).unwrap();
        super::super::removal::plan_removal(&mut txn, [2; 16], creation.replicas()[0]).unwrap();
        txn.commit().unwrap();
        let mut txn = store.begin().unwrap();
        super::super::migration::plan_migration(&mut txn, [3; 16], creation.task(), NodeId(5))
            .unwrap();
        let (second, changed) = plan_replica(&mut txn, [3; 16], PlanMode::Add, None).unwrap();
        assert!(changed && second.task() != plan.task());
        txn.commit().unwrap();
        assert_eq!(committed_plans(&store).unwrap().len(), 2);
    }
}
