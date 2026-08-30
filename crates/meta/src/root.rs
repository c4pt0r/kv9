//! META_REGION_0-owned L0 root certificate.
//!
//! The durable files authorize a process to start; this committed singleton
//! certifies that the Raft group accepted that exact authority. It is not a
//! tenant row and is never resolved through keyspace/range routing.

use kv9_common::{Error, Result, RootDescriptor};
use kv9_engine::Engine;

use crate::codec::{memcmp_uint, ColumnValue, RowValue};
use crate::schema::{ColumnId, ROOT_META_DESC};
use crate::store::MetaTxn;

const ROOT_PK: u64 = 0;
const ROOT_DESCRIPTOR: ColumnId = ColumnId(2);
const ROOT_DIGEST: ColumnId = ColumnId(3);
const ROOT_GENERATION: ColumnId = ColumnId(4);

pub fn initialize_root<E: Engine>(txn: &mut MetaTxn<'_, E>, root: &RootDescriptor) -> Result<()> {
    if txn.get(&ROOT_META_DESC, &[memcmp_uint(ROOT_PK)])?.is_some() {
        return Err(Error::Config(
            "root descriptor is already certified and immutable".into(),
        ));
    }
    let mut row = RowValue::new();
    row.set(ROOT_DESCRIPTOR, ColumnValue::Bytes(root.canonical_bytes()));
    row.set(
        ROOT_DIGEST,
        ColumnValue::Bytes(root.digest().as_bytes().to_vec()),
    );
    row.set(
        ROOT_GENERATION,
        ColumnValue::Bytes(root.bootstrap_generation.as_bytes().to_vec()),
    );
    txn.insert(&ROOT_META_DESC, &[memcmp_uint(ROOT_PK)], row)
}

pub fn certified_root<E: Engine>(txn: &MetaTxn<'_, E>) -> Result<Option<RootDescriptor>> {
    let Some(row) = txn.get(&ROOT_META_DESC, &[memcmp_uint(ROOT_PK)])? else {
        return Ok(None);
    };
    let Some(ColumnValue::Bytes(descriptor)) = row.value.get(ROOT_DESCRIPTOR) else {
        return Err(Error::Config(
            "root certificate is missing descriptor".into(),
        ));
    };
    let root = RootDescriptor::decode(descriptor)?;
    let Some(ColumnValue::Bytes(digest)) = row.value.get(ROOT_DIGEST) else {
        return Err(Error::Config("root certificate is missing digest".into()));
    };
    if digest.as_slice() != root.digest().as_bytes() {
        return Err(Error::Config("root certificate digest mismatch".into()));
    }
    let Some(ColumnValue::Bytes(generation)) = row.value.get(ROOT_GENERATION) else {
        return Err(Error::Config(
            "root certificate is missing bootstrap generation".into(),
        ));
    };
    if generation.as_slice() != root.bootstrap_generation.as_bytes() {
        return Err(Error::Config(
            "root certificate bootstrap generation mismatch".into(),
        ));
    }
    Ok(Some(root))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use kv9_common::{BootstrapGeneration, ClusterId, NodeId, RootVoter, StoreIncarnation};
    use kv9_engine::MemEngine;

    use super::*;
    use crate::store::MetaStore;

    fn descriptor() -> RootDescriptor {
        RootDescriptor::new(
            ClusterId::from_bytes([1; 16]),
            BootstrapGeneration::from_bytes([2; 16]),
            vec![RootVoter {
                node_id: NodeId(1),
                addr: "127.0.0.1:20160".parse().unwrap(),
                store_incarnation: StoreIncarnation::from_bytes([3; 16]),
            }],
            b"credential",
        )
        .unwrap()
    }

    #[test]
    fn committed_root_round_trips_and_refuses_replacement() {
        let store = MetaStore::new(Arc::new(MemEngine::new()));
        let root = descriptor();
        let mut txn = store.begin().unwrap();
        initialize_root(&mut txn, &root).unwrap();
        txn.commit().unwrap();

        let txn = store.begin().unwrap();
        assert_eq!(certified_root(&txn).unwrap(), Some(root.clone()));
        drop(txn);

        let mut txn = store.begin().unwrap();
        let other = RootDescriptor::new(
            ClusterId::from_bytes([9; 16]),
            BootstrapGeneration::from_bytes([8; 16]),
            root.voters.clone(),
            b"other",
        )
        .unwrap();
        assert!(initialize_root(&mut txn, &other).is_err());
        assert_eq!(certified_root(&txn).unwrap(), Some(root));
    }
}
