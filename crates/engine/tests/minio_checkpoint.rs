//! Engine-level recovery against real MinIO. Run explicitly with the four
//! KV9_OBJECT_STORE_* variables. Raft certification is exercised separately by
//! scripts/minio-kv-e2e.sh; these tests isolate the remote recovery failure modes.
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use kv9_common::AppliedPosition;
use kv9_engine::checkpoint::{FlushScope, RemoteUploader};
use kv9_engine::{
    ColumnFamily, DurableAppliedPosition, Engine, MinioConfig, MinioObjectStore, ObjectKey,
    ObjectStore, ReplicatedEngine, WalEngine, WriteBatch,
};

fn fixture(
    name: &str,
) -> (
    std::path::PathBuf,
    Arc<MinioObjectStore>,
    RemoteUploader,
    FlushScope,
) {
    let id = format!(
        "{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let path = std::env::temp_dir()
        .join(format!("kv9-remote-checkpoint-{id}"))
        .join("catalog.wal");
    let store = Arc::new(
        MinioObjectStore::connect(
            MinioConfig::from_env().expect("real MinIO configuration required"),
        )
        .unwrap(),
    );
    let uploader = RemoteUploader::new(store.clone());
    let scope = FlushScope {
        cluster: id,
        region: 1,
        conf_ver: 1,
        version: 1,
    };
    (path, store, uploader, scope)
}

#[test]
#[ignore = "requires a real MinIO server"]
fn remote_checkpoint_reclaims_prefix_and_replays_the_exact_frozen_tail() {
    let (path, store, uploader, scope) = fixture("tail");
    let (engine, _) = WalEngine::open(&path).unwrap();
    let mut batch = WriteBatch::new();
    for cf in ColumnFamily::ALL {
        batch.put(cf, vec![255, 255], b"at-cut".to_vec());
    }
    batch.put(ColumnFamily::Default, b"deleted".to_vec(), b"old".to_vec());
    engine
        .write_applied(batch, AppliedPosition { term: 1, index: 1 })
        .unwrap();
    let frozen = engine.freeze(scope).unwrap();
    let mut later = WriteBatch::new();
    later.put(
        ColumnFamily::Default,
        vec![255, 255],
        b"tail-value".to_vec(),
    );
    later.delete(ColumnFamily::Default, b"deleted".to_vec());
    engine
        .write_applied(later, AppliedPosition { term: 2, index: 3 })
        .unwrap();
    let manifest = uploader.upload(frozen).unwrap().into_manifest();
    // Simulate the caller's ordered apply confirmation for this engine-only test.
    assert!(engine.checkpoint_applied(&manifest).unwrap());
    drop(engine);
    let (recovered, replay) = WalEngine::open_with_uploader(&path, Some(&uploader)).unwrap();
    assert_eq!(
        replay.positions,
        vec![Some(AppliedPosition { term: 2, index: 3 })],
        "covered records must actually leave the WAL"
    );
    assert_eq!(
        recovered.applied_position().unwrap(),
        DurableAppliedPosition::AppliedThrough(AppliedPosition { term: 2, index: 3 })
    );
    assert_eq!(
        recovered.get(ColumnFamily::Default, &[255, 255]).unwrap(),
        Some(b"tail-value".to_vec())
    );
    for cf in [ColumnFamily::Lock, ColumnFamily::Write] {
        assert_eq!(
            recovered.get(cf, &[255, 255]).unwrap(),
            Some(b"at-cut".to_vec())
        );
    }
    assert_eq!(
        recovered.get(ColumnFamily::Default, b"deleted").unwrap(),
        None
    );
    drop(recovered);
    for file in &manifest.files {
        store
            .delete(&ObjectKey::new(file.key.clone()).unwrap())
            .unwrap();
    }
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
#[ignore = "requires a real MinIO server"]
fn missing_or_corrupt_remote_sst_refuses_reopen_without_editing_the_wal() {
    let (path, store, uploader, scope) = fixture("corrupt");
    let (engine, _) = WalEngine::open(&path).unwrap();
    let mut batch = WriteBatch::new();
    batch.put(
        ColumnFamily::Default,
        b"required".to_vec(),
        b"durable".to_vec(),
    );
    engine
        .write_applied(batch, AppliedPosition { term: 1, index: 1 })
        .unwrap();
    let manifest = uploader
        .upload(engine.freeze(scope).unwrap())
        .unwrap()
        .into_manifest();
    assert!(engine.checkpoint_applied(&manifest).unwrap());
    drop(engine);
    let before = std::fs::read(&path).unwrap();
    let key = ObjectKey::new(manifest.files[0].key.clone()).unwrap();
    let original = store.get(&key).unwrap().unwrap();
    store.delete(&key).unwrap();
    let error = WalEngine::open_with_uploader(&path, Some(&uploader))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("missing SST"),
        "must reject absent remote bytes: {error}"
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    store.put(&key, b"corrupt").unwrap();
    let error = WalEngine::open_with_uploader(&path, Some(&uploader))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("checksum or size mismatch"),
        "must reject corrupt remote bytes: {error}"
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    // The production backend refuses overwriting immutable content. Repair the
    // isolated test object by removing the injected corrupt value first.
    store.delete(&key).unwrap();
    store.put(&key, &original).unwrap();
    let (recovered, _) = WalEngine::open_with_uploader(&path, Some(&uploader)).unwrap();
    assert_eq!(
        recovered.get(ColumnFamily::Default, b"required").unwrap(),
        Some(b"durable".to_vec()),
        "restoring the actual object must make recovery succeed"
    );
    drop(recovered);
    store.delete(&key).unwrap();
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
#[ignore = "requires a real MinIO server"]
fn pending_flush_recovers_the_same_identity_only_after_rechecking_remote_bytes() {
    use kv9_engine::checkpoint::FlushJournal;
    let (path, store, uploader, scope) = fixture("pending");
    let (engine, _) = WalEngine::open(&path).unwrap();
    let mut batch = WriteBatch::new();
    batch.put(
        ColumnFamily::Default,
        b"pending".to_vec(),
        b"original".to_vec(),
    );
    engine
        .write_applied(batch, AppliedPosition { term: 3, index: 17 })
        .unwrap();
    let prepared = uploader.upload(engine.freeze(scope).unwrap()).unwrap();
    let journal_path = path.with_extension("pending");
    let mut journal = FlushJournal::new(&journal_path);
    journal.stage(&prepared, 4).unwrap();
    let manifest = prepared.into_manifest();
    let expected_id = manifest.change_id(4).unwrap();
    drop(journal);
    drop(engine);
    let mut recovered = FlushJournal::new(&journal_path);
    let key = ObjectKey::new(manifest.files[0].key.clone()).unwrap();
    let original = store.get(&key).unwrap().unwrap();
    let before = std::fs::read(&journal_path).unwrap();
    store.delete(&key).unwrap();
    assert!(recovered
        .load()
        .unwrap()
        .unwrap()
        .recover(&uploader)
        .unwrap_err()
        .to_string()
        .contains("missing SST"));
    assert_eq!(
        std::fs::read(&journal_path).unwrap(),
        before,
        "unreadable remote state must retain the pending owner"
    );
    store.put(&key, &original).unwrap();
    let (prepared, generation) = recovered
        .load()
        .unwrap()
        .unwrap()
        .recover(&uploader)
        .unwrap();
    assert_eq!(generation, 4);
    assert_eq!(
        prepared.into_manifest().change_id(generation).unwrap(),
        expected_id,
        "restart must never invent another proposal identity"
    );
    recovered.clear_settled().unwrap();
    assert!(FlushJournal::new(&journal_path).load().unwrap().is_none());
    for file in &manifest.files {
        store
            .delete(&ObjectKey::new(file.key.clone()).unwrap())
            .unwrap();
    }
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
