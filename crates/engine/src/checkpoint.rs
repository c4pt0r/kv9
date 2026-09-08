//! Full-state remote checkpoints for the single-group KV implementation.
//!
//! Freeze is O(1) under the engine's write lock. Upload and recovery run outside
//! Raft apply. Full checkpoints include all column families and deletions through
//! their cut (absence in a complete snapshot is authoritative). This deliberately
//! precedes the incremental LSM/block-cache implementation.
use std::sync::Arc;

use kv9_common::{AppliedPosition, Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::minio::MinioObjectStore;
use crate::sst::{Sst, SstWriter};
use crate::{ColumnFamily, MemEngine, ObjectKey, ObjectStore, ReplicatedEngine, WriteBatch};

pub use crate::flush_journal::{FlushJournal, PendingFlush};

const MAGIC: &[u8] = b"KV9CHECKPOINT\x01";
const SST_TARGET: usize = 8 * 1024 * 1024;
/// Bound the initial fully resident engine checkpoint. Incremental SST reads and
/// memory admission will replace this implementation, not silently remove the cap.
pub const MAX_CHECKPOINT_BYTES: usize = 48 * 1024 * 1024;
const MAX_MANIFEST_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlushScope {
    pub cluster: String,
    pub region: u64,
    pub conf_ver: u64,
    pub version: u64,
}

/// A sealed snapshot and exact applied position; callers cannot mint or edit it.
/// ```compile_fail
/// fn duplicate(f: kv9_engine::checkpoint::FrozenFlush) { let _ = f.clone(); }
/// ```
pub struct FrozenFlush {
    pub(crate) view: crate::mem::MemSnapshot,
    pub(crate) position: AppliedPosition,
    pub(crate) scope: FlushScope,
}
impl FrozenFlush {
    pub fn position(&self) -> AppliedPosition {
        self.position
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SstReference {
    pub key: String,
    pub sha256: String,
    pub cf: u8,
    pub smallest: Vec<u8>,
    pub largest: Vec<u8>,
    pub size: u64,
    pub count: u64,
}

/// Minted only after an acknowledged PUT and verified GET through the remote
/// client. The capability is consumed into a manifest attempt, never cloned.
/// ```compile_fail
/// fn duplicate(s: kv9_engine::checkpoint::PreparedSst) { let _ = s.clone(); }
/// ```
#[derive(Debug)]
pub struct PreparedSst {
    reference: SstReference,
}

#[derive(Debug)]
pub struct PreparedFlush {
    scope: FlushScope,
    position: AppliedPosition,
    files: Vec<PreparedSst>,
}
impl PreparedFlush {
    pub(crate) fn descriptor(&self) -> CheckpointManifest {
        CheckpointManifest {
            scope: self.scope.clone(),
            term: self.position.term,
            index: self.position.index,
            files: self.files.iter().map(|sst| sst.reference.clone()).collect(),
        }
    }
    pub fn into_manifest(self) -> CheckpointManifest {
        CheckpointManifest {
            scope: self.scope,
            term: self.position.term,
            index: self.position.index,
            files: self.files.into_iter().map(|sst| sst.reference).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointManifest {
    pub scope: FlushScope,
    pub term: u64,
    pub index: u64,
    pub files: Vec<SstReference>,
}
impl CheckpointManifest {
    pub fn position(&self) -> AppliedPosition {
        AppliedPosition {
            term: self.term,
            index: self.index,
        }
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut bytes = MAGIC.to_vec();
        bytes.extend(serde_json::to_vec(self).map_err(codec_error)?);
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(Error::Engine("checkpoint manifest too large".into()));
        }
        Ok(bytes)
    }
    pub fn is_checkpoint(bytes: &[u8]) -> bool {
        bytes.starts_with(b"KV9CHECKPOINT")
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if !bytes.starts_with(MAGIC) || bytes.len() > MAX_MANIFEST_BYTES {
            return Err(Error::Engine(
                "invalid checkpoint manifest version or size".into(),
            ));
        }
        let manifest: Self = serde_json::from_slice(&bytes[MAGIC.len()..]).map_err(codec_error)?;
        let mut total = 0u64;
        let mut previous: Option<&SstReference> = None;
        if manifest.index == 0
            || manifest.term == 0
            || manifest.scope.cluster.is_empty()
            || manifest.files.is_empty()
        {
            return Err(Error::Engine("invalid empty checkpoint".into()));
        }
        for file in &manifest.files {
            total = total
                .checked_add(file.size)
                .ok_or_else(|| Error::Engine("checkpoint size overflow".into()))?;
            if file.cf > 2
                || file.count == 0
                || file.smallest > file.largest
                || file.sha256.len() != 64
                || !file
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                || file.key != object_name(&manifest.scope, &file.sha256)
                || total > MAX_CHECKPOINT_BYTES as u64
                || previous.is_some_and(|p| {
                    p.cf > file.cf || (p.cf == file.cf && p.largest >= file.smallest)
                })
            {
                return Err(Error::Engine("invalid checkpoint SST references".into()));
            }
            previous = Some(file);
        }
        Ok(manifest)
    }
    /// Fixed-order struct encoding gives one identity for a given predecessor
    /// and content. Neither file ids nor proposal ids are caller-picked.
    pub fn change_id(&self, generation: u64) -> Result<Vec<u8>> {
        let mut hash = Sha256::new();
        hash.update(b"kv9-manifest-v1");
        hash.update(generation.to_be_bytes());
        hash.update(self.encode()?);
        Ok(hash.finalize().to_vec())
    }
    pub fn covers_effect(&self, intended: &Self) -> bool {
        self.scope == intended.scope
            && self.index >= intended.index
            && intended
                .files
                .iter()
                .all(|f| self.files.iter().any(|a| a == f))
    }
}
fn codec_error(e: serde_json::Error) -> Error {
    Error::Engine(format!("checkpoint codec: {e}"))
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn object_name(scope: &FlushScope, hash: &str) -> String {
    format!(
        "clusters/{}/regions/{}/sst/{hash}",
        scope.cluster, scope.region
    )
}

/// Production construction accepts the real MinIO/S3 backend. Unit-test memory
/// stores cannot accidentally qualify as remote durability evidence.
#[derive(Debug)]
pub struct RemoteUploader {
    store: Arc<MinioObjectStore>,
}
impl RemoteUploader {
    pub fn new(store: Arc<MinioObjectStore>) -> Self {
        Self { store }
    }
    pub fn upload(&self, frozen: FrozenFlush) -> Result<PreparedFlush> {
        upload(self.store.as_ref(), frozen)
    }
    pub fn restore(&self, manifest: &CheckpointManifest) -> Result<MemEngine> {
        restore(self.store.as_ref(), manifest)
    }

    pub(crate) fn recover_prepared(&self, manifest: &CheckpointManifest) -> Result<PreparedFlush> {
        recover_prepared(self.store.as_ref(), manifest)
    }
}

fn upload(store: &dyn ObjectStore, frozen: FrozenFlush) -> Result<PreparedFlush> {
    let mut files = Vec::new();
    let mut total = 0usize;
    for (cf_id, cf) in ColumnFamily::ALL.into_iter().enumerate() {
        let mut writer = SstWriter::new(cf);
        let mut size = 0usize;
        for entry in frozen.view.iter_all(cf) {
            let (key, value) = entry?;
            size = size
                .checked_add(key.len() + value.len() + 8)
                .ok_or_else(|| Error::Engine("checkpoint size overflow".into()))?;
            if size > SST_TARGET && !writer.is_empty() {
                let sst = prepare(store, &frozen.scope, cf_id as u8, writer)?;
                total += sst.reference.size as usize;
                if total > MAX_CHECKPOINT_BYTES {
                    return Err(Error::Engine(
                        "checkpoint exceeds resident engine limit".into(),
                    ));
                }
                files.push(sst);
                writer = SstWriter::new(cf);
                size = key.len() + value.len() + 8;
            }
            writer.add(key, value)?;
        }
        if !writer.is_empty() {
            let sst = prepare(store, &frozen.scope, cf_id as u8, writer)?;
            total += sst.reference.size as usize;
            if total > MAX_CHECKPOINT_BYTES {
                return Err(Error::Engine(
                    "checkpoint exceeds resident engine limit".into(),
                ));
            }
            files.push(sst);
        }
    }
    if files.is_empty() {
        return Err(Error::Engine("refusing empty checkpoint".into()));
    }
    Ok(PreparedFlush {
        scope: frozen.scope,
        position: frozen.position,
        files,
    })
}
fn prepare(
    store: &dyn ObjectStore,
    scope: &FlushScope,
    cf: u8,
    writer: SstWriter,
) -> Result<PreparedSst> {
    let bytes = writer.finish()?;
    if bytes.len() > MAX_CHECKPOINT_BYTES {
        return Err(Error::Engine("SST exceeds checkpoint limit".into()));
    }
    let hash = digest(&bytes);
    let key = ObjectKey::new(object_name(scope, &hash))?;
    store.put(&key, &bytes)?;
    // A successful PUT followed by readable, hash-identical bytes is required.
    // Ambiguous PUT failure returns no capability; retry uses the same content id.
    if store.get(&key)?.as_deref() != Some(bytes.as_slice()) {
        return Err(Error::Engine("uploaded SST was not durably visible".into()));
    }
    let sst = Sst::parse(&bytes)?;
    Ok(PreparedSst {
        reference: SstReference {
            key: key.as_str().into(),
            sha256: hash,
            cf,
            smallest: sst.smallest_key().into(),
            largest: sst.largest_key().into(),
            size: bytes.len() as u64,
            count: sst.len() as u64,
        },
    })
}
fn restore(store: &dyn ObjectStore, manifest: &CheckpointManifest) -> Result<MemEngine> {
    // Revalidate descriptors supplied via an API as well as those decoded from disk.
    let manifest = CheckpointManifest::decode(&manifest.encode()?)?;
    let index = MemEngine::new();
    let mut batch = WriteBatch::new();
    for file in &manifest.files {
        let sst = verified_sst(store, file)?;
        for (key, value) in sst.iter() {
            batch.put(sst.column_family(), key.to_vec(), value.to_vec());
        }
    }
    index.write_applied(batch, manifest.position())?;
    Ok(index)
}

fn verified_sst(store: &dyn ObjectStore, file: &SstReference) -> Result<Sst> {
    let bytes = store
        .get(&ObjectKey::new(file.key.clone())?)?
        .ok_or_else(|| Error::Engine("checkpoint references a missing SST".into()))?;
    if bytes.len() as u64 != file.size || digest(&bytes) != file.sha256 {
        return Err(Error::Engine(
            "checkpoint SST checksum or size mismatch".into(),
        ));
    }
    let sst = Sst::parse(&bytes)?;
    if sst.column_family() != ColumnFamily::ALL[file.cf as usize]
        || sst.smallest_key() != file.smallest
        || sst.largest_key() != file.largest
        || sst.len() as u64 != file.count
    {
        return Err(Error::Engine("checkpoint SST metadata mismatch".into()));
    }
    Ok(sst)
}

fn recover_prepared(
    store: &dyn ObjectStore,
    manifest: &CheckpointManifest,
) -> Result<PreparedFlush> {
    let manifest = CheckpointManifest::decode(&manifest.encode()?)?;
    let mut files = Vec::with_capacity(manifest.files.len());
    for reference in manifest.files {
        // Restart never treats a local journal as proof that the remote bytes
        // are still readable. Recheck each immutable object before any resend.
        verified_sst(store, &reference)?;
        files.push(PreparedSst { reference });
    }
    Ok(PreparedFlush {
        scope: manifest.scope,
        position: AppliedPosition {
            term: manifest.term,
            index: manifest.index,
        },
        files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Engine, MemoryObjectStore, WalEngine};
    fn scope() -> FlushScope {
        FlushScope {
            cluster: "test-cluster".into(),
            region: 1,
            conf_ver: 1,
            version: 1,
        }
    }
    fn engine(name: &str) -> WalEngine {
        let path =
            std::env::temp_dir().join(format!("kv9-checkpoint-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        WalEngine::open(path.join("wal")).unwrap().0
    }
    #[test]
    fn pending_flush_journal_preserves_identity_and_exclusive_ownership() {
        let engine = engine("journal");
        let store = MemoryObjectStore::new();
        put(&engine, 1, b"first");
        let prepared = upload(&store, engine.freeze(scope()).unwrap()).unwrap();
        let identity = prepared.descriptor().change_id(7).unwrap();
        let path = engine.path().with_extension("pending");
        let mut journal = FlushJournal::new(&path);
        journal.stage(&prepared, 7).unwrap();
        journal.stage(&prepared, 7).unwrap(); // exact idempotent save
        assert!(
            journal.stage(&prepared, 8).is_err(),
            "unknown attempt cannot be overwritten"
        );
        drop(journal);
        let mut recovered = FlushJournal::new(&path);
        let pending = recovered.load().unwrap().unwrap();
        assert_eq!(pending.expected_generation(), 7);
        assert_eq!(pending.manifest().change_id(7).unwrap(), identity);
        let verified = recover_prepared(&store, pending.manifest()).unwrap();
        assert_eq!(verified.into_manifest().change_id(7).unwrap(), identity);
        recovered.clear_settled().unwrap();
        assert!(recovered.load().unwrap().is_none());
        recovered.stage(&prepared, 8).unwrap();
        assert_eq!(recovered.load().unwrap().unwrap().expected_generation(), 8);
    }

    #[test]
    fn torn_or_corrupt_pending_state_is_never_reset_to_a_fresh_slot() {
        let engine = engine("journal-torn");
        let store = MemoryObjectStore::new();
        put(&engine, 1, b"first");
        let prepared = upload(&store, engine.freeze(scope()).unwrap()).unwrap();
        let path = engine.path().with_extension("pending");
        let mut journal = FlushJournal::new(&path);
        journal.stage(&prepared, 0).unwrap();
        let complete = std::fs::read(&path).unwrap();
        for cut in 0..complete.len() {
            std::fs::write(&path, &complete[..cut]).unwrap();
            assert!(
                journal.load().is_err(),
                "partial published journal at cut {cut} must fail closed"
            );
            assert_eq!(std::fs::read(&path).unwrap(), complete[..cut]);
        }
        for byte in [0, 12, complete.len() / 2, complete.len() - 1] {
            let mut damaged = complete.clone();
            damaged[byte] ^= 1;
            std::fs::write(&path, &damaged).unwrap();
            assert!(
                journal.load().is_err(),
                "corruption in any covered field must fail closed"
            );
        }
        std::fs::write(&path, &complete).unwrap();
        assert!(journal.load().unwrap().is_some());
        std::fs::remove_file(&path).unwrap();
        std::fs::write(
            path.with_extension("pending.tmp"),
            b"interrupted temporary write",
        )
        .unwrap();
        assert!(
            journal.load().unwrap().is_none(),
            "a temporary write was never publish authority"
        );
        journal.stage(&prepared, 0).unwrap();
        assert!(journal.load().unwrap().is_some());
    }

    fn put(engine: &WalEngine, index: u64, value: &[u8]) {
        let mut batch = WriteBatch::new();
        for cf in ColumnFamily::ALL {
            batch.put(cf, b"\xff\x00key".to_vec(), value.to_vec());
        }
        engine
            .write_applied(batch, AppliedPosition { term: 3, index })
            .unwrap();
    }

    #[test]
    fn frozen_cut_excludes_later_updates_and_restores_all_column_families() {
        let engine = engine("frozen");
        let store = MemoryObjectStore::new();
        put(&engine, 4, b"before");
        let frozen = engine.freeze(scope()).unwrap();
        put(&engine, 9, b"after");
        let manifest = upload(&store, frozen).unwrap().into_manifest();
        assert_eq!(manifest.position(), AppliedPosition { term: 3, index: 4 });
        assert_eq!(manifest.files.len(), 3);
        let recovered = restore(&store, &manifest).unwrap();
        for cf in ColumnFamily::ALL {
            assert_eq!(
                recovered.get(cf, b"\xff\x00key").unwrap(),
                Some(b"before".to_vec()),
                "frozen bytes must correspond to the sealed position"
            );
            assert_eq!(
                engine.get(cf, b"\xff\x00key").unwrap(),
                Some(b"after".to_vec())
            );
        }
        assert_eq!(
            recovered.volatile_applied_position(),
            Some(manifest.position())
        );
    }

    #[test]
    fn missing_or_corrupt_remote_sst_never_recovers_as_empty_data() {
        let engine = engine("missing");
        put(&engine, 4, b"present");
        let store = MemoryObjectStore::new();
        let manifest = upload(&store, engine.freeze(scope()).unwrap())
            .unwrap()
            .into_manifest();
        let key = ObjectKey::new(manifest.files[0].key.clone()).unwrap();
        store.delete(&key).unwrap();
        assert!(restore(&store, &manifest)
            .unwrap_err()
            .to_string()
            .contains("missing SST"));
        store.put(&key, b"corrupt").unwrap();
        assert!(restore(&store, &manifest)
            .unwrap_err()
            .to_string()
            .contains("checksum or size mismatch"));
    }

    #[derive(Debug)]
    struct InvisibleStore;
    impl ObjectStore for InvisibleStore {
        fn put(&self, _: &ObjectKey, _: &[u8]) -> Result<()> {
            Ok(())
        }
        fn get(&self, _: &ObjectKey) -> Result<Option<Vec<u8>>> {
            Ok(None)
        }
        fn delete(&self, _: &ObjectKey) -> Result<()> {
            Ok(())
        }
        fn list(&self, _: &str) -> Result<Vec<ObjectKey>> {
            Ok(Vec::new())
        }
    }
    #[test]
    fn an_acknowledged_but_invisible_upload_cannot_mint_prepared_ssts() {
        let engine = engine("visibility");
        put(&engine, 4, b"present");
        let error = upload(&InvisibleStore, engine.freeze(scope()).unwrap()).unwrap_err();
        assert!(error.to_string().contains("not durably visible"));
    }

    #[test]
    fn local_wal_reclaim_refuses_unpositioned_records() {
        let engine = engine("unpositioned");
        engine.write(WriteBatch::new()).unwrap();
        put(&engine, 4, b"present");
        let manifest = upload(&MemoryObjectStore::new(), engine.freeze(scope()).unwrap())
            .unwrap()
            .into_manifest();
        assert!(!engine.checkpoint_applied(&manifest).unwrap());
        assert!(!engine.path().with_extension("checkpoint").exists());
    }
}
