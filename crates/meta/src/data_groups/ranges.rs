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
}
impl CommittedRange {
    pub fn creation(&self) -> &CommittedCreation {
        &self.creation
    }
    pub fn range(&self) -> &DataRange {
        &self.range
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
        let keyspace = Tables::<E>::keyspace_in(txn, range.keyspace)?
            .ok_or_else(|| invalid("range keyspace missing"))?;
        if range.root != root.digest()
            || range.creation != creation.digest()
            || range.region != creation.region()
            || range.sealed
            || range.conf_ver != 1
            || range.version != 1
            || !range.start.is_empty()
            || !range.end.is_empty()
            || keyspace.api_type != ApiType::Raw
            || keyspace.tenant_id != range.tenant
            || keyspace.config != DATA_KEYSPACE_CONFIG
            || bindings
                .iter()
                .any(|b| b.range.region == range.region || b.range.keyspace == range.keyspace)
        {
            return Err(invalid(
                "range binding conflicts with immutable creation or namespace",
            ));
        }
        bindings.push(CommittedRange {
            creation: CommittedCreation(creation),
            range,
        });
    }
    Ok(bindings)
}

pub fn committed_ranges<E: Engine>(store: &MetaStore<E>) -> Result<Vec<CommittedRange>> {
    read_in(&store.begin()?)
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
