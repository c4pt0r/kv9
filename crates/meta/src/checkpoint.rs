//! Historical metadata of the initial whole-engine checkpoint.
//!
//! This is an observation from the supplied immutable view, not an install or
//! retention capability. Startup supplies its actual verified restored image;
//! upload supplies its actual frozen image. The initial engine includes every
//! logical routing range in one Raft group. A routing row is not a per-range
//! snapshot or permission to attach a new store.
use std::sync::Arc;

use kv9_common::{Error, KeyspaceId, Result, RootDescriptor, RootDigest, META_REGION_0};
use kv9_engine::checkpoint::{CheckpointManifest, FlushScope};
use kv9_engine::{MemEngine, ReadView};

use crate::codec::{memcmp_uint, ColumnValue};
use crate::schema::{ColumnId, SCHEMA_VERSION, SCHEMA_VERSION_DESC};
use crate::store::MetaStore;
use crate::tables::{Region, Tables};

/// The catalog facts at one supplied image. This type does not certify that
/// image's Raft cut or publication; the enclosing recovery protocol must do so.
#[derive(Debug)]
pub struct InitialWholeEngineBase {
    root_digest: RootDigest,
    scope: FlushScope,
    metadata_region: Region,
}

impl InitialWholeEngineBase {
    pub fn root_digest(&self) -> RootDigest {
        self.root_digest
    }

    /// Full-engine scope owned by the initial metadata Raft group. It must not
    /// be relabeled as one of the engine's logical data regions.
    pub fn flush_scope(&self) -> FlushScope {
        self.scope.clone()
    }

    pub fn metadata_region(&self) -> &Region {
        &self.metadata_region
    }

    pub fn check_manifest(&self, manifest: &CheckpointManifest) -> Result<()> {
        if manifest.scope != self.scope {
            return Err(invalid("manifest scope differs from its historical image"));
        }
        Ok(())
    }
}

/// Inspect one immutable view using the existing typed catalog decoders. This
/// does not consult a second/live engine snapshot. All fields, including the
/// root certificate and metadata region, therefore belong to the same cut.
pub fn inspect_initial_checkpoint_base(
    view: &dyn ReadView,
    expected_root: &RootDescriptor,
) -> Result<InitialWholeEngineBase> {
    expected_root.validate()?;
    // MetaTxn supplies the canonical schema/PK checks and typed decoders. Its
    // reads are exclusively from begin_at's borrowed view. The empty backing
    // engine is never read, written or committed; it supplies no authority.
    let schema = MetaStore::new(Arc::new(MemEngine::new()));
    let txn = schema.begin_at(Box::new(view));
    if crate::admission::cluster_id(&txn)? != Some(expected_root.cluster_id) {
        return Err(invalid(
            "image cluster identity differs from the durable root",
        ));
    }
    if crate::root::certified_root(&txn)?.as_ref() != Some(expected_root) {
        return Err(invalid("image lacks the exact certified root"));
    }
    let version = txn
        .get(&SCHEMA_VERSION_DESC, &[memcmp_uint(0)])?
        .ok_or_else(|| invalid("image schema version is missing"))?;
    if version.value.get(ColumnId(2)) != Some(&ColumnValue::Uint(SCHEMA_VERSION as u64)) {
        return Err(invalid("image schema version is unsupported"));
    }
    let region = Tables::<MemEngine>::region_by_id_in(&txn, META_REGION_0)?
        .ok_or_else(|| invalid("image metadata region is missing"))?;
    if region.keyspace_id != KeyspaceId::SYSTEM
        || !region.start_key.is_empty()
        || !region.end_key.is_empty()
        || region.epoch_conf == 0
        || region.epoch_ver == 0
    {
        return Err(invalid("image is not the initial whole-engine owner scope"));
    }
    Ok(InitialWholeEngineBase {
        root_digest: expected_root.digest(),
        scope: FlushScope {
            cluster: expected_root.cluster_id.to_string(),
            region: META_REGION_0.0,
            conf_ver: region.epoch_conf,
            version: region.epoch_ver,
        },
        metadata_region: region,
    })
}

fn invalid(reason: &str) -> Error {
    Error::Engine(format!("checkpoint base identity: {reason}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{encode_row_key, RowValue};
    use crate::schema::{TableDesc, CLUSTER_META_DESC, REGIONS_DESC, ROOT_META_DESC};
    use kv9_common::{
        AppliedPosition, BootstrapGeneration, ClusterId, NodeId, RootVoter, StoreIncarnation,
    };
    use kv9_engine::{ColumnFamily, Engine, ReplicatedEngine, WriteBatch};

    fn root(generation: u8) -> RootDescriptor {
        RootDescriptor::new(
            ClusterId::from_bytes([1; 16]),
            BootstrapGeneration::from_bytes([generation; 16]),
            vec![RootVoter {
                node_id: NodeId(1),
                addr: "127.0.0.1:20160".parse().unwrap(),
                store_incarnation: StoreIncarnation::from_bytes([3; 16]),
            }],
            b"checkpoint-test-credential",
        )
        .unwrap()
    }

    fn key(table: &TableDesc, id: u64) -> Vec<u8> {
        encode_row_key(table.id, &[memcmp_uint(id)]).unwrap()
    }

    fn region(epoch: u64) -> RowValue {
        let mut row = RowValue::new();
        for (id, value) in [
            (1, META_REGION_0.0),
            (2, KeyspaceId::SYSTEM.0 as u64),
            (5, epoch),
            (6, epoch),
            (7, 1),
        ] {
            row.set(ColumnId(id), ColumnValue::Uint(value));
        }
        row.set(ColumnId(3), ColumnValue::Bytes(Vec::new()));
        row.set(ColumnId(4), ColumnValue::Bytes(Vec::new()));
        row
    }

    fn region_batch(row: RowValue) -> WriteBatch {
        let mut batch = WriteBatch::new();
        batch.put(
            ColumnFamily::Default,
            key(&REGIONS_DESC, META_REGION_0.0),
            row.encode(),
        );
        batch
    }

    // A component fixture for the four identity records. Root and cluster use
    // their production encoders; raw region/schema writes allow corrupt-image
    // controls without pretending to initialize a complete serving catalog.
    fn identity_batch(root: &RootDescriptor, epoch: u64) -> WriteBatch {
        let store = MetaStore::new(Arc::new(MemEngine::new()));
        let mut txn = store.begin().unwrap();
        crate::root::initialize_root(&mut txn, root).unwrap();
        crate::admission::initialize_cluster(&mut txn, root.cluster_id, 1).unwrap();
        let mut version = RowValue::new();
        version.set(ColumnId(1), ColumnValue::Uint(0));
        version.set(ColumnId(2), ColumnValue::Uint(SCHEMA_VERSION as u64));
        txn.insert(&SCHEMA_VERSION_DESC, &[memcmp_uint(0)], version)
            .unwrap();
        let mut batch = txn.into_batch();
        batch.put(
            ColumnFamily::Default,
            key(&REGIONS_DESC, META_REGION_0.0),
            region(epoch).encode(),
        );
        batch
    }

    fn fixture() -> (MemEngine, RootDescriptor) {
        let root = root(2);
        let engine = MemEngine::new();
        engine.write(identity_batch(&root, 1)).unwrap();
        (engine, root)
    }

    #[test]
    fn historical_view_keeps_its_epoch_after_live_catalog_advances() {
        let (engine, root) = fixture();
        let old = engine.snapshot().unwrap();
        engine.write(region_batch(region(2))).unwrap();
        let base = inspect_initial_checkpoint_base(old.as_ref(), &root).unwrap();
        assert_eq!(base.root_digest(), root.digest());
        assert_eq!(base.metadata_region().epoch_conf, 1);
        assert_eq!(base.flush_scope().version, 1);
        let current = engine.snapshot().unwrap();
        assert_eq!(
            inspect_initial_checkpoint_base(current.as_ref(), &root)
                .unwrap()
                .flush_scope()
                .version,
            2
        );
    }

    #[test]
    fn manifest_must_match_all_historical_scope_fields() {
        let (engine, root) = fixture();
        let view = engine.snapshot().unwrap();
        let base = inspect_initial_checkpoint_base(view.as_ref(), &root).unwrap();
        // This checks only the scope contract; SST/position validation belongs
        // to the engine and retained committed publication checks.
        let manifest = CheckpointManifest {
            scope: base.flush_scope(),
            term: 1,
            index: 1,
            files: Vec::new(),
        };
        base.check_manifest(&manifest).unwrap();
        for field in 0..4 {
            let mut wrong = manifest.clone();
            match field {
                0 => wrong.scope.cluster = "other-cluster".into(),
                1 => wrong.scope.region += 1,
                2 => wrong.scope.conf_ver += 1,
                _ => wrong.scope.version += 1,
            }
            assert!(base.check_manifest(&wrong).is_err());
        }
    }

    #[test]
    fn same_cluster_with_different_root_is_refused() {
        let (engine, expected) = fixture();
        let other = root(9);
        assert_eq!(expected.cluster_id, other.cluster_id);
        let view = engine.snapshot().unwrap();
        assert!(inspect_initial_checkpoint_base(view.as_ref(), &other).is_err());
    }

    #[test]
    fn missing_identity_records_are_not_inferred_from_other_rows() {
        for (table, id) in [
            (&ROOT_META_DESC, 0),
            (&CLUSTER_META_DESC, 0),
            (&SCHEMA_VERSION_DESC, 0),
            (&REGIONS_DESC, META_REGION_0.0),
        ] {
            let (engine, root) = fixture();
            let mut batch = WriteBatch::new();
            batch.delete(ColumnFamily::Default, key(table, id));
            engine.write(batch).unwrap();
            let view = engine.snapshot().unwrap();
            assert!(
                inspect_initial_checkpoint_base(view.as_ref(), &root).is_err(),
                "{}",
                table.name
            );
        }
    }

    #[test]
    fn malformed_identity_and_owner_scope_are_refused() {
        let corruptions = [
            (&ROOT_META_DESC, 0, 3, ColumnValue::Bytes(vec![0; 32])),
            (&CLUSTER_META_DESC, 0, 2, ColumnValue::Bytes(vec![9; 16])),
            (
                &SCHEMA_VERSION_DESC,
                0,
                2,
                ColumnValue::Uint(SCHEMA_VERSION as u64 + 1),
            ),
            (
                &REGIONS_DESC,
                META_REGION_0.0,
                1,
                ColumnValue::Uint(META_REGION_0.0 + 1),
            ),
            (&REGIONS_DESC, META_REGION_0.0, 2, ColumnValue::Uint(7)),
            (
                &REGIONS_DESC,
                META_REGION_0.0,
                3,
                ColumnValue::Bytes(vec![1]),
            ),
            (
                &REGIONS_DESC,
                META_REGION_0.0,
                4,
                ColumnValue::Bytes(vec![2]),
            ),
            (&REGIONS_DESC, META_REGION_0.0, 5, ColumnValue::Uint(0)),
            (&REGIONS_DESC, META_REGION_0.0, 6, ColumnValue::Uint(0)),
            (
                &REGIONS_DESC,
                META_REGION_0.0,
                5,
                ColumnValue::Bytes(vec![1]),
            ),
        ];
        for (table, id, column, value) in corruptions {
            let (engine, root) = fixture();
            let key = key(table, id);
            let mut row =
                RowValue::decode(&engine.get(ColumnFamily::Default, &key).unwrap().unwrap())
                    .unwrap();
            row.set(ColumnId(column), value);
            let mut batch = WriteBatch::new();
            batch.put(ColumnFamily::Default, key, row.encode());
            engine.write(batch).unwrap();
            let view = engine.snapshot().unwrap();
            assert!(
                inspect_initial_checkpoint_base(view.as_ref(), &root).is_err(),
                "{} column {column}",
                table.name
            );
        }
    }

    #[test]
    #[ignore = "requires a real MinIO server"]
    fn real_minio_checks_historical_identity_before_a_repairing_tail() {
        use kv9_engine::checkpoint::RemoteUploader;
        use kv9_engine::{MinioConfig, MinioObjectStore, WalEngine};
        let uploader = RemoteUploader::new(Arc::new(
            MinioObjectStore::connect(MinioConfig::from_env().unwrap()).unwrap(),
        ));
        for segmented in [false, true] {
            for invalid_base in [false, true] {
                let nonce = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos();
                let path = std::env::temp_dir()
                    .join(format!("kv9-base-identity-{}-{nonce}", std::process::id()));
                std::fs::create_dir(&path).unwrap();
                let expected = root(2);
                let base_root = if invalid_base {
                    root(9)
                } else {
                    expected.clone()
                };
                let wal_path = path.join("catalog.wal");
                let engine = WalEngine::open(&wal_path).unwrap().0;
                if segmented {
                    engine.enable_segmentation().unwrap();
                }
                engine
                    .write_applied(
                        identity_batch(&base_root, 1),
                        AppliedPosition { term: 1, index: 1 },
                    )
                    .unwrap();
                let frozen = engine
                    .freeze_with_scope(|view| {
                        Ok(inspect_initial_checkpoint_base(view, &base_root)?.flush_scope())
                    })
                    .unwrap();
                let manifest = uploader.upload(frozen).unwrap().into_manifest();
                // Deliberately overwrite immutable root records in the bad
                // fixture. Such a later repair must not hide the wrong base.
                engine
                    .write_applied(
                        identity_batch(&expected, 2),
                        AppliedPosition { term: 1, index: 2 },
                    )
                    .unwrap();
                engine.checkpoint_applied(&manifest).unwrap();
                drop(engine);
                let tail_calls = std::cell::Cell::new(0);
                let reopened = WalEngine::open_with_base_observer(
                    &wal_path,
                    Some(&uploader),
                    |_| Ok(()),
                    |manifest, view| {
                        inspect_initial_checkpoint_base(view, &expected)?.check_manifest(manifest)
                    },
                    |_, _| {
                        tail_calls.set(tail_calls.get() + 1);
                        Ok(())
                    },
                );
                if invalid_base {
                    assert!(reopened
                        .unwrap_err()
                        .to_string()
                        .contains("exact certified root"));
                    assert_eq!(tail_calls.get(), 0);
                } else {
                    let engine = reopened.unwrap().0;
                    let current = engine.snapshot().unwrap();
                    assert_eq!(
                        inspect_initial_checkpoint_base(current.as_ref(), &expected)
                            .unwrap()
                            .flush_scope()
                            .version,
                        2
                    );
                    assert!(tail_calls.get() > 0);
                }
                std::fs::remove_dir_all(path).unwrap();
            }
        }
    }
}
