//! New Raw keyspaces bound to already reserved empty data groups. Existing
//! keyspaces cannot be rebound here; movement requires a different protocol.
use super::*;
use crate::{
    schema::{KEYSPACES_DESC, REGIONS_DESC},
    tables::{Keyspace, Tables},
};
use kv9_common::{data_range::DataRange, ApiType, KeyspaceId, TenantId};

const BIND_RANGE: u64 = 102;
pub const DATA_KEYSPACE_CONFIG: &[u8] = b"KV9DATA01";

#[derive(Debug, Clone)]
pub struct CommittedRange {
    creation: CommittedCreation,
    range: DataRange,
    bind_task: u64,
}
impl CommittedRange {
    pub fn creation(&self) -> &CommittedCreation {
        &self.creation
    }
    pub fn range(&self) -> &DataRange {
        &self.range
    }
    /// The kind-102 row that committed this binding.
    pub fn bind_task(&self) -> u64 {
        self.bind_task
    }
}

fn read_in<E: Engine>(txn: &MetaTxn<'_, E>) -> Result<Vec<CommittedRange>> {
    let root = crate::root::certified_root(txn)?
        .ok_or_else(|| invalid("range binding requires a certified root"))?;
    let rows = txn.scan(&TASKS_DESC, MAX_CREATION_TASKS + 1)?;
    if rows.len() > MAX_CREATION_TASKS {
        return Err(invalid("bounded range task scan exceeded"));
    }
    let mut bindings: Vec<CommittedRange> = Vec::new();
    for row in rows {
        if row.value.get(ColumnId(2)) != Some(&ColumnValue::Uint(BIND_RANGE)) {
            continue;
        }
        let Some(ColumnValue::Uint(task)) = row.value.get(ColumnId(1)) else {
            return Err(invalid("range task ID missing"));
        };
        let Some(ColumnValue::Bytes(payload)) = row.value.get(ColumnId(3)) else {
            return Err(invalid("range task payload missing"));
        };
        if payload.len() < 8
            || row.pk != [memcmp_uint(*task)]
            || *task < FIRST_DYNAMIC_ID
            || row.value.get(ColumnId(4)) != Some(&ColumnValue::Uint(0))
            || row.value.get(ColumnId(5)) != Some(&ColumnValue::Uint(0))
        {
            return Err(invalid("invalid range task binding"));
        }
        let creation_task = u64::from_be_bytes(payload[..8].try_into().unwrap());
        let creation = txn
            .get(&TASKS_DESC, &[memcmp_uint(creation_task)])?
            .as_ref()
            .map(|r| from_row(r, root.digest()))
            .transpose()?
            .flatten()
            .ok_or_else(|| invalid("range creation missing"))?;
        let range = DataRange::decode(&payload[8..])?;
        range.validate()?;
        let keyspace = Tables::<E>::keyspace_in(txn, range.keyspace)?
            .ok_or_else(|| invalid("range keyspace missing"))?;
        // Per-row binding rules. Bounds are no longer forced empty: a split
        // publishes children with real bounds. The PARTITION rules below are
        // what keep a keyspace's unsealed coverage exact.
        if range.root != root.digest()
            || range.creation != creation.digest()
            || range.region != creation.region()
            || range.conf_ver < 1
            || range.version < 1
            || keyspace.api_type != ApiType::Raw
            || keyspace.tenant_id != range.tenant
            || keyspace.config != DATA_KEYSPACE_CONFIG
            || bindings.iter().any(|b| b.range.region == range.region)
        {
            return Err(invalid(
                "range binding conflicts with immutable creation or namespace",
            ));
        }
        bindings.push(CommittedRange {
            creation: CommittedCreation(creation),
            range,
            bind_task: *task,
        });
    }
    // Directory-wide partition rules: for every keyspace, the UNSEALED
    // bindings must cover the whole key space exactly once — sorted by
    // start, first start empty, last end empty, each boundary shared with
    // its neighbor, never overlapping and never gapped. Sealed bindings are
    // routing history: they keep their row rules but cover nothing.
    let mut keyspaces: Vec<_> = bindings.iter().map(|b| b.range.keyspace).collect();
    keyspaces.sort_unstable();
    keyspaces.dedup();
    for keyspace in keyspaces {
        let mut live: Vec<&CommittedRange> = bindings
            .iter()
            .filter(|b| b.range.keyspace == keyspace && !b.range.sealed)
            .collect();
        if live.is_empty() {
            return Err(invalid("keyspace has no unsealed range coverage"));
        }
        live.sort_by(|a, b| a.range.start.cmp(&b.range.start));
        if !live[0].range.start.is_empty() || !live[live.len() - 1].range.end.is_empty() {
            return Err(invalid("unsealed ranges must cover the whole keyspace"));
        }
        for pair in live.windows(2) {
            if pair[0].range.end.is_empty() || pair[0].range.end != pair[1].range.start {
                return Err(invalid(
                    "unsealed ranges must partition the keyspace without overlap or gap",
                ));
            }
        }
    }
    Ok(bindings)
}

/// Read-only routing observation from the caller's single snapshot. Unlike
/// CommittedRange, this projection is not an activation or publication capability.
pub fn route_in<E: Engine>(
    txn: &MetaTxn<'_, E>,
    keyspace: KeyspaceId,
    key: &[u8],
) -> Result<Option<(DataRange, Vec<super::InitialReplica>)>> {
    Ok(read_in(txn)?
        .into_iter()
        .find(|b| !b.range.sealed && b.range.keyspace == keyspace && b.range.contains(key))
        .map(|b| (b.range, b.creation.intent().replicas().to_vec())))
}

pub fn committed_ranges<E: Engine>(store: &MetaStore<E>) -> Result<Vec<CommittedRange>> {
    read_in(&store.begin()?)
}

/// Sibling planners (the split module) reuse the exact committed reader.
pub(crate) fn committed_ranges_in_txn<E: Engine>(
    txn: &MetaTxn<'_, E>,
) -> Result<Vec<CommittedRange>> {
    read_in(txn)
}

/// Caller holds the metadata planner lock through same-term committed apply.
/// A failure drops ALL staged sequence, creation/desire and namespace changes.
pub fn plan_keyspace<E: Engine>(
    txn: &mut MetaTxn<'_, E>,
    root: RootDigest,
    creation_task: u64,
    name: &str,
    tenant: TenantId,
) -> Result<(DataRange, bool)> {
    if name.is_empty() || name.len() > 1024 {
        return Err(invalid("data keyspace name must contain 1..1024 bytes"));
    }
    if crate::root::certified_root(txn)?.map(|r| r.digest()) != Some(root) {
        return Err(invalid("range request root differs"));
    }
    for binding in read_in(txn)? {
        if binding.creation.intent().task() == creation_task {
            let ks = Tables::<E>::keyspace_in(txn, binding.range.keyspace)?.unwrap();
            if ks.name != name || ks.tenant_id != tenant {
                return Err(invalid("data group namespace binding conflicts"));
            }
            return Ok((binding.range, false));
        }
    }
    let creation = txn
        .get(&TASKS_DESC, &[memcmp_uint(creation_task)])?
        .as_ref()
        .map(|r| from_row(r, root))
        .transpose()?
        .flatten()
        .ok_or_else(|| invalid("data group creation missing"))?;
    activation::plan_activation(txn, &creation)?;
    if txn.scan(&TASKS_DESC, MAX_CREATION_TASKS + 1)?.len() >= MAX_CREATION_TASKS {
        return Err(invalid("range task capacity reached"));
    }
    let id = txn.allocate_id(SequenceKind::Keyspace)?;
    if id > u64::from(KeyspaceId::MAX) {
        return Err(invalid("keyspace sequence exhausted"));
    }
    let range = DataRange {
        root,
        creation: creation.digest(),
        region: creation.region(),
        keyspace: KeyspaceId(id as u32),
        tenant,
        conf_ver: 1,
        version: 1,
        start: Vec::new(),
        end: Vec::new(),
        sealed: false,
    };
    range.validate()?;
    let ks = Keyspace {
        id: range.keyspace,
        name: name.to_string(),
        tenant_id: tenant,
        api_type: ApiType::Raw,
        start_key: Vec::new(),
        end_key: Vec::new(),
        state: 0,
        config: DATA_KEYSPACE_CONFIG.to_vec(),
    };
    txn.insert(&KEYSPACES_DESC, &ks.pk(), ks.to_row_value())?;
    let mut region = RowValue::new();
    for (c, v) in [
        (1, ColumnValue::Uint(range.region.0)),
        (2, ColumnValue::Uint(id)),
        (3, ColumnValue::Bytes(Vec::new())),
        (4, ColumnValue::Bytes(Vec::new())),
        (5, ColumnValue::Uint(1)),
        (6, ColumnValue::Uint(1)),
        (7, ColumnValue::Uint(0)),
    ] {
        region.set(ColumnId(c), v);
    }
    txn.insert(&REGIONS_DESC, &[memcmp_uint(range.region.0)], region)?;
    let task = txn.allocate_id(SequenceKind::Task)?;
    let mut payload = creation_task.to_be_bytes().to_vec();
    payload.extend_from_slice(&range.encode());
    let mut row = RowValue::new();
    for (c, v) in [
        (1, ColumnValue::Uint(task)),
        (2, ColumnValue::Uint(BIND_RANGE)),
        (3, ColumnValue::Bytes(payload)),
        (4, ColumnValue::Uint(0)),
        (5, ColumnValue::Uint(0)),
    ] {
        row.set(ColumnId(c), v);
    }
    txn.insert(&TASKS_DESC, &[memcmp_uint(task)], row)?;
    Ok((range, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_bind_row(
        store: &MetaStore<kv9_engine::MemEngine>,
        creation_task: u64,
        range: &DataRange,
    ) -> u64 {
        let mut txn = store.begin().unwrap();
        let task = txn.allocate_id(SequenceKind::Task).unwrap();
        let mut payload = creation_task.to_be_bytes().to_vec();
        payload.extend_from_slice(&range.encode());
        let mut row = RowValue::new();
        for (c, v) in [
            (1, ColumnValue::Uint(task)),
            (2, ColumnValue::Uint(BIND_RANGE)),
            (3, ColumnValue::Bytes(payload)),
            (4, ColumnValue::Uint(0)),
            (5, ColumnValue::Uint(0)),
        ] {
            row.set(ColumnId(c), v);
        }
        txn.insert(&TASKS_DESC, &[memcmp_uint(task)], row).unwrap();
        txn.commit().unwrap();
        task
    }

    #[test]
    fn a_sealed_parent_with_a_child_partition_routes_only_through_children() {
        let store = super::super::tests::fixture();
        let mut txn = store.begin().unwrap();
        let mut tenant = RowValue::new();
        tenant.set(ColumnId(2), ColumnValue::Text("tenant".into()));
        tenant.set(ColumnId(3), ColumnValue::Uint(0));
        tenant.set(ColumnId(4), ColumnValue::Uint(0));
        txn.insert(&crate::schema::TENANTS_DESC, &[memcmp_uint(1)], tenant)
            .unwrap();
        let parent =
            plan_empty_group(&mut txn, [41; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        let (bound, _) = plan_keyspace(
            &mut txn,
            parent.root(),
            parent.task(),
            "split-me",
            TenantId(1),
        )
        .unwrap();
        let low = plan_empty_group(&mut txn, [42; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        activation::plan_activation(&mut txn, &low).unwrap();
        let high =
            plan_empty_group(&mut txn, [43; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        activation::plan_activation(&mut txn, &high).unwrap();
        txn.commit().unwrap();

        // Seal the parent binding in place (version bump, one-way).
        let sealed = DataRange {
            sealed: true,
            version: bound.version + 1,
            ..bound.clone()
        };
        let parent_bind = committed_ranges(&store).unwrap()[0].bind_task();
        let mut txn = store.begin().unwrap();
        let mut payload = parent.task().to_be_bytes().to_vec();
        payload.extend_from_slice(&sealed.encode());
        txn.update(
            &TASKS_DESC,
            &[memcmp_uint(parent_bind)],
            vec![(ColumnId(3), ColumnValue::Bytes(payload))],
        )
        .unwrap();
        txn.commit().unwrap();
        assert!(
            committed_ranges(&store).is_err(),
            "a sealed parent with no children leaves the keyspace uncovered"
        );

        let child = |creation: &CreationIntent, start: &[u8], end: &[u8]| DataRange {
            root: bound.root,
            creation: creation.digest(),
            region: creation.region(),
            keyspace: bound.keyspace,
            tenant: bound.tenant,
            conf_ver: 1,
            version: 1,
            start: start.to_vec(),
            end: end.to_vec(),
            sealed: false,
        };
        // A gapped child set must refuse; the exact partition is accepted.
        let high_only = raw_bind_row(&store, high.task(), &child(&high, b"m", b""));
        assert!(
            committed_ranges(&store).is_err(),
            "a gapped child set must refuse"
        );
        raw_bind_row(&store, low.task(), &child(&low, b"", b"m"));
        let bindings = committed_ranges(&store).unwrap();
        assert_eq!(bindings.len(), 3);

        // Routing skips the sealed parent and picks the covering child.
        let txn = store.begin().unwrap();
        let (hit, _) = route_in(&txn, bound.keyspace, b"a").unwrap().unwrap();
        assert_eq!(hit.region, low.region());
        let (hit, _) = route_in(&txn, bound.keyspace, b"z").unwrap().unwrap();
        assert_eq!(hit.region, high.region());
        let (hit, _) = route_in(&txn, bound.keyspace, b"m").unwrap().unwrap();
        assert_eq!(
            hit.region,
            high.region(),
            "the boundary belongs to the high child"
        );
        drop(txn);

        // An overlapping extra child must refuse.
        let mut txn = store.begin().unwrap();
        let extra =
            plan_empty_group(&mut txn, [44; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        activation::plan_activation(&mut txn, &extra).unwrap();
        txn.commit().unwrap();
        raw_bind_row(&store, extra.task(), &child(&extra, b"f", b"t"));
        assert!(
            committed_ranges(&store).is_err(),
            "an overlapping child must refuse"
        );
        let _ = high_only;
    }
    #[test]
    fn range_binding_is_atomic_idempotent_and_cannot_reuse_a_namespace() {
        let store = super::super::tests::fixture();
        let mut txn = store.begin().unwrap();
        let mut tenant = RowValue::new();
        tenant.set(ColumnId(2), ColumnValue::Text("tenant".into()));
        tenant.set(ColumnId(3), ColumnValue::Uint(0));
        tenant.set(ColumnId(4), ColumnValue::Uint(0));
        txn.insert(&crate::schema::TENANTS_DESC, &[memcmp_uint(1)], tenant)
            .unwrap();
        let intent =
            plan_empty_group(&mut txn, [32; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        txn.commit().unwrap();
        let mut txn = store.begin().unwrap();
        let (range, changed) =
            plan_keyspace(&mut txn, intent.root(), intent.task(), "first", TenantId(1)).unwrap();
        assert!(changed);
        assert!(committed_ranges(&store).unwrap().is_empty());
        assert!(Tables::<kv9_engine::MemEngine>::keyspace_in(
            &store.begin().unwrap(),
            range.keyspace
        )
        .unwrap()
        .is_none());
        txn.commit().unwrap();
        assert_eq!(committed_ranges(&store).unwrap()[0].range(), &range);
        assert_eq!(activation::committed_activations(&store).unwrap().len(), 1);
        let mut txn = store.begin().unwrap();
        assert_eq!(
            plan_keyspace(&mut txn, intent.root(), intent.task(), "first", TenantId(1)).unwrap(),
            (range.clone(), false)
        );
        assert!(plan_keyspace(
            &mut txn,
            intent.root(),
            intent.task(),
            "renamed",
            TenantId(1)
        )
        .is_err());
        drop(txn);
        let mut txn = store.begin().unwrap();
        let other =
            plan_empty_group(&mut txn, [33; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
        assert!(
            plan_keyspace(&mut txn, intent.root(), other.task(), "first", TenantId(1)).is_err()
        );
        drop(txn);
        assert!(committed_creation(&store, other.task()).unwrap().is_none());
        assert_eq!(committed_ranges(&store).unwrap().len(), 1);
    }
}
