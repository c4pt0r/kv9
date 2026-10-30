//! Full-state remote checkpoints for one Raft-owned engine.
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
#[cfg(test)]
use crate::sst::Sst;
use crate::sst::{SharedSst, SstView, SstWriter};
use crate::{ColumnFamily, ObjectKey, ObjectStore, VersionedMemEngine};
#[cfg(test)]
use crate::{ReplicatedEngine, WriteBatch};

pub use crate::flush_journal::{
    FlushJournal, PendingFlush, PendingTransfer, TransferJournal, TransferLock, TransferReader,
};
pub use crate::object_execution::{DrainedExecution, ExecutionIdentity};

const MAGIC: &[u8] = b"KV9CHECKPOINT\x01";
const LARGE_MAGIC: &[u8] = b"KV9CHECKPOINT\x02";
const SST_TARGET: usize = 8 * 1024 * 1024;
/// V1 whole-image ceiling and per-SST limit for both manifest versions.
pub const MAX_CHECKPOINT_BYTES: usize = 48 * 1024 * 1024;
pub const MAX_STREAMED_SSTS: usize = kv9_common::retention::MAX_OWNER_RESOURCES;
/// Derived descriptor/file bound, not a resident buffer allocation size.
pub const MAX_STREAMED_CHECKPOINT_BYTES: u64 =
    MAX_CHECKPOINT_BYTES as u64 * MAX_STREAMED_SSTS as u64;
pub const MAX_MANIFEST_BYTES: usize = 1024 * 1024;

mod execution;
pub use execution::{CheckpointReader, CheckpointRestore, DrainedRead};
mod manifest_codec;
mod selection;
mod spool;
pub(crate) use spool::DiskObjects;
pub use spool::SpooledFlush;

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
    pub(crate) view: crate::versioned::View,
    pub(crate) position: AppliedPosition,
    pub(crate) scope: FlushScope,
    pub(crate) data_revision: u64,
}
impl FrozenFlush {
    pub fn position(&self) -> AppliedPosition {
        self.position
    }
    /// Scheduling observation from this exact image; never a durability fence.
    pub fn data_revision(&self) -> u64 {
        self.data_revision
    }
}

/// Exact bounded object bytes prepared without external I/O. This is not a
/// remotely verified capability and cannot construct a manifest proposal.
/// ```compile_fail
/// fn prepared(p: kv9_engine::checkpoint::PlannedFlush) -> kv9_engine::checkpoint::PreparedFlush { p }
/// ```
#[derive(Debug)]
pub struct PlannedFlush {
    pub(crate) manifest: CheckpointManifest,
    pub(crate) objects: Vec<Vec<u8>>,
    // Declared after the buffers so ordinary drop frees them before admission.
    pub(crate) reservation: Option<crate::checkpoint_memory::Reservation>,
}
impl PlannedFlush {
    pub fn manifest(&self) -> &CheckpointManifest {
        &self.manifest
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
        manifest_codec::encode(self)
    }
    pub fn is_checkpoint(bytes: &[u8]) -> bool {
        bytes.starts_with(b"KV9CHECKPOINT")
    }
    pub fn requires_large_format(&self) -> Result<bool> {
        let total = self.files.iter().try_fold(0u64, |sum, file| {
            sum.checked_add(file.size)
                .ok_or_else(|| Error::Engine("checkpoint size overflow".into()))
        })?;
        Ok(total > MAX_CHECKPOINT_BYTES as u64)
    }
    /// Legacy log tags must never reinterpret a previously invalid checkpoint.
    pub fn decode_legacy(bytes: &[u8]) -> Result<Self> {
        if !bytes.starts_with(MAGIC) {
            return Err(Error::Engine(
                "legacy checkpoint command requires V1 manifest".into(),
            ));
        }
        Self::decode(bytes)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let large = bytes.starts_with(LARGE_MAGIC);
        if (!large && !bytes.starts_with(MAGIC)) || bytes.len() > MAX_MANIFEST_BYTES {
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
                || file.size > MAX_CHECKPOINT_BYTES as u64
                || (!large && total > MAX_CHECKPOINT_BYTES as u64)
                || previous.is_some_and(|p| {
                    p.cf > file.cf || (p.cf == file.cf && p.largest >= file.smallest)
                })
            {
                return Err(Error::Engine("invalid checkpoint SST references".into()));
            }
            previous = Some(file);
        }
        if large
            && (total <= MAX_CHECKPOINT_BYTES as u64 || manifest.files.len() > MAX_STREAMED_SSTS)
        {
            return Err(Error::Engine("invalid large checkpoint shape".into()));
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
    store: RemoteBackend,
    image_budget: Arc<crate::checkpoint_memory::ImageBudget>,
    spool_budget: Arc<crate::checkpoint_spool_budget::SpoolBudget>,
    pending_budget: Arc<crate::checkpoint_pending_budget::PendingDiskBudget>,
    execution: Option<Arc<crate::object_execution::Execution>>,
}
#[derive(Debug, Clone)]
enum RemoteBackend {
    Minio(Arc<MinioObjectStore>),
    #[cfg(any(test, feature = "testing"))]
    Harness(Arc<dyn ObjectStore>),
}
impl RemoteUploader {
    pub fn object_memory_snapshot(&self) -> Option<crate::object_memory::Snapshot> {
        match &self.store {
            RemoteBackend::Minio(store) => Some(store.object_memory_snapshot()),
            #[cfg(any(test, feature = "testing"))]
            RemoteBackend::Harness(_) => None,
        }
    }
    pub fn object_io_snapshot(&self) -> Option<crate::object_io::ObjectIoSnapshot> {
        match &self.store {
            RemoteBackend::Minio(store) => store.object_io_snapshot(),
            #[cfg(any(test, feature = "testing"))]
            RemoteBackend::Harness(_) => None,
        }
    }
    pub fn new(store: Arc<MinioObjectStore>) -> Self {
        Self {
            store: RemoteBackend::Minio(store),
            image_budget: Arc::default(),
            spool_budget: Arc::default(),
            pending_budget: Arc::default(),
            execution: None,
        }
    }
    /// Component tests only. This exercises SST codecs and replay, and is
    /// explicitly NOT evidence of remote durability. No such constructor is
    /// present in an ordinary build; production construction stays MinIO-only.
    #[cfg(any(test, feature = "testing"))]
    #[doc(hidden)]
    pub fn for_harness(store: Arc<dyn ObjectStore>) -> Self {
        Self {
            store: RemoteBackend::Harness(store),
            image_budget: Arc::default(),
            spool_budget: Arc::default(),
            pending_budget: Arc::default(),
            execution: None,
        }
    }
    pub fn with_image_budget(mut self, budget: Arc<crate::checkpoint_memory::ImageBudget>) -> Self {
        self.image_budget = budget;
        self
    }
    pub fn image_memory_snapshot(&self) -> crate::checkpoint_memory::Snapshot {
        self.image_budget.snapshot()
    }
    pub fn with_spool_budget(
        mut self,
        budget: Arc<crate::checkpoint_spool_budget::SpoolBudget>,
    ) -> Self {
        self.spool_budget = budget;
        self
    }
    pub fn spool_disk_snapshot(&self) -> crate::checkpoint_spool_budget::Snapshot {
        self.spool_budget.snapshot()
    }
    pub fn with_pending_budget(
        mut self,
        budget: Arc<crate::checkpoint_pending_budget::PendingDiskBudget>,
    ) -> Self {
        self.pending_budget = budget;
        self
    }
    pub fn pending_disk_snapshot(&self) -> crate::checkpoint_pending_budget::Snapshot {
        self.pending_budget.snapshot()
    }
    pub fn observe_removed_journal(&self, path: &std::path::Path) -> Result<()> {
        self.pending_budget.observe_removed_journal(path)
    }
    /// Journals created from this uploader share admission with every group.
    pub fn journal(&self, path: impl Into<std::path::PathBuf>) -> FlushJournal {
        FlushJournal::new(path)
            .with_image_budget(self.image_budget.clone())
            .with_pending_budget(self.pending_budget.clone())
            .with_streaming_objects()
    }

    /// One exclusively locked durable transfer slot, sharing checkpoint budgets.
    pub fn transfer_journal(
        &self,
        path: impl Into<std::path::PathBuf>,
    ) -> Result<crate::flush_journal::TransferJournal> {
        crate::flush_journal::TransferJournal::lock(self.journal(path))
    }
    /// Borrowed preflight precedes any object allocation. The reservation moves
    /// with the completed plan and survives every upload/readback return path.
    pub fn plan_checkpoint(&self, frozen: FrozenFlush) -> Result<PlannedFlush> {
        let layout = image_layout(&frozen, SST_TARGET, MAX_CHECKPOINT_BYTES)?;
        let reservation = self.image_budget.reserve(layout.total as u64)?;
        encode_plan(frozen, layout, Some(reservation))
    }
    fn store(&self) -> execution::CheckpointStore<'_> {
        execution::CheckpointStore(self)
    }
    pub fn upload(&self, frozen: FrozenFlush) -> Result<PreparedFlush> {
        self.upload_planned(self.plan_checkpoint(frozen)?)
    }
    /// Plan every immutable object before registering an owner or performing I/O.
    pub fn plan(frozen: FrozenFlush) -> Result<PlannedFlush> {
        plan(frozen)
    }
    pub fn upload_planned(&self, mut planned: PlannedFlush) -> Result<PreparedFlush> {
        // Compatibility callers can still construct a plan with the static
        // planner. Admit that existing image before issuing its first PUT.
        if planned.reservation.is_none() {
            let bytes = planned.objects.iter().map(|o| o.len() as u64).sum();
            planned.reservation = Some(self.image_budget.reserve(bytes)?);
        }
        upload_planned(&self.store(), planned)
    }
    pub fn restore(&self, manifest: &CheckpointManifest) -> Result<VersionedMemEngine> {
        restore(&self.store(), manifest)
    }

    pub(crate) fn recover_prepared(&self, manifest: &CheckpointManifest) -> Result<PreparedFlush> {
        recover_prepared(&self.store(), manifest)
    }

    /// GET and validate every referenced SST, draining actual local jobs before
    /// return. No PUT, restored index, publication capability or GC authority.
    /// Callers must hold their committed retention protection throughout.
    pub fn verify_checkpoint_objects(&self, manifest: &CheckpointManifest) -> Result<()> {
        self.with_local_execution(|uploader| uploader.recover_prepared(manifest).map(|_| ()))
    }

    /// PUT one small descriptor object (a backup bundle) under the budgets and
    /// the execution discipline every other PUT observes. Idempotent for equal
    /// bytes; a different object under the same key is refused by the store.
    pub fn put_object(&self, key: &ObjectKey, bytes: &[u8]) -> Result<()> {
        self.with_local_execution(|uploader| uploader.store().put(key, bytes))
    }

    /// GET one small descriptor object, bounded by `max_bytes`.
    pub fn get_object(&self, key: &ObjectKey, max_bytes: usize) -> Result<Option<Vec<u8>>> {
        self.with_local_execution(|uploader| uploader.store().get_bounded(key, max_bytes))
    }
}

#[cfg(test)]
fn upload(store: &dyn ObjectStore, frozen: FrozenFlush) -> Result<PreparedFlush> {
    upload_planned(store, plan(frozen)?)
}
fn plan(frozen: FrozenFlush) -> Result<PlannedFlush> {
    plan_with_limits(frozen, SST_TARGET, MAX_CHECKPOINT_BYTES)
}
fn plan_with_limits(frozen: FrozenFlush, target: usize, limit: usize) -> Result<PlannedFlush> {
    let layout = image_layout(&frozen, target, limit.min(MAX_CHECKPOINT_BYTES))?;
    encode_plan(frozen, layout, None)
}

struct TableLayout {
    count: usize,
    bytes: usize,
}
struct ImageLayout {
    tables: [Vec<TableLayout>; 3],
    total: usize,
}

fn encode_plan(
    frozen: FrozenFlush,
    layout: ImageLayout,
    reservation: Option<crate::checkpoint_memory::Reservation>,
) -> Result<PlannedFlush> {
    let mut files = Vec::new();
    let mut objects = Vec::new();
    for ((cf_id, cf), tables) in ColumnFamily::ALL.into_iter().enumerate().zip(layout.tables) {
        let mut rows = frozen.view.iter_all(cf);
        for table in tables {
            // One table's rows at a time (≤ SST_TARGET of entry bytes): the versioned
            // view hands out owned pages, the encoder borrows from this bounded set.
            let owned: Vec<(Vec<u8>, Vec<u8>)> = rows.by_ref().take(table.count).collect();
            let bytes = crate::sst::encode_borrowed(
                cf,
                table.count,
                table.bytes,
                owned.iter().map(|(k, v)| (k.as_slice(), v.as_slice())),
            )?;
            let (sst, bytes) = plan_sst(&frozen.scope, cf_id as u8, bytes)?;
            files.push(sst);
            objects.push(bytes);
        }
        if rows.next().is_some() {
            return Err(Error::Engine("checkpoint layout omitted rows".into()));
        }
    }
    if objects.iter().map(Vec::len).sum::<usize>() != layout.total {
        return Err(Error::Engine("checkpoint layout size mismatch".into()));
    }
    let manifest = CheckpointManifest {
        scope: frozen.scope,
        term: frozen.position.term,
        index: frozen.position.index,
        files,
    };
    CheckpointManifest::decode(&manifest.encode()?)?;
    Ok(PlannedFlush {
        manifest,
        objects,
        reservation,
    })
}
/// Borrowed preflight retains only count/size per SST. The original target is
/// based on entry bytes, while every prefix's limit includes header and footer.
fn image_layout(frozen: &FrozenFlush, target: usize, limit: usize) -> Result<ImageLayout> {
    let mut tables: [Vec<TableLayout>; 3] = std::array::from_fn(|_| Vec::new());
    let mut total = 0usize;
    for (cf, tables) in ColumnFamily::ALL.into_iter().zip(&mut tables) {
        let (mut entries, mut first, mut last) = (0usize, 0usize, 0usize);
        let mut count = 0;
        for (key, value) in frozen.view.iter_all(cf) {
            let row = key
                .len()
                .checked_add(value.len())
                .and_then(|n| n.checked_add(8))
                .ok_or_else(|| Error::Engine("checkpoint size overflow".into()))?;
            if entries.saturating_add(row) > target && count != 0 {
                let bytes = SstWriter::table_bytes(entries, first, last)?;
                total += bytes;
                tables.push(TableLayout { count, bytes });
                entries = 0;
                count = 0;
            }
            if count == 0 {
                first = key.len();
            }
            entries = entries
                .checked_add(row)
                .ok_or_else(|| Error::Engine("checkpoint size overflow".into()))?;
            last = key.len();
            count += 1;
            let size = SstWriter::table_bytes(entries, first, last)?;
            if size > MAX_CHECKPOINT_BYTES || size > limit - total {
                return Err(Error::Engine(
                    "checkpoint exceeds resident engine limit".into(),
                ));
            }
        }
        if count != 0 {
            let bytes = SstWriter::table_bytes(entries, first, last)?;
            total += bytes;
            tables.push(TableLayout { count, bytes });
        }
    }
    if total == 0 {
        return Err(Error::Engine("refusing empty checkpoint".into()));
    }
    Ok(ImageLayout { tables, total })
}
#[cfg(test)]
fn image_bytes(frozen: &FrozenFlush) -> Result<usize> {
    Ok(image_layout(frozen, SST_TARGET, MAX_CHECKPOINT_BYTES)?.total)
}
fn plan_sst(scope: &FlushScope, cf: u8, bytes: Vec<u8>) -> Result<(SstReference, Vec<u8>)> {
    if bytes.len() > MAX_CHECKPOINT_BYTES {
        return Err(Error::Engine("SST exceeds checkpoint limit".into()));
    }
    let hash = digest(&bytes);
    let key = ObjectKey::new(object_name(scope, &hash))?;
    let sst = SstView::parse(&bytes)?;
    Ok((
        SstReference {
            key: key.as_str().into(),
            sha256: hash,
            cf,
            smallest: sst.smallest_key().into(),
            largest: sst.largest_key().into(),
            size: bytes.len() as u64,
            count: sst.len() as u64,
        },
        bytes,
    ))
}
pub(crate) fn upload_planned(
    store: &dyn ObjectStore,
    planned: PlannedFlush,
) -> Result<PreparedFlush> {
    let _reservation = planned.reservation;
    let mut files = Vec::with_capacity(planned.manifest.files.len());
    for (reference, bytes) in planned.manifest.files.into_iter().zip(planned.objects) {
        let key = ObjectKey::new(reference.key.clone())?;
        store.put(&key, &bytes)?;
        // An ambiguous PUT retains the original journal and owner. It cannot
        // mint a prepared capability; retries use these exact bytes and keys.
        if store.get_shared_bounded(&key, bytes.len())?.as_deref() != Some(bytes.as_slice()) {
            return Err(Error::Engine("uploaded SST was not durably visible".into()));
        }
        files.push(PreparedSst { reference });
    }
    Ok(PreparedFlush {
        scope: planned.manifest.scope,
        position: AppliedPosition {
            term: planned.manifest.term,
            index: planned.manifest.index,
        },
        files,
    })
}
fn restore(store: &dyn ObjectStore, manifest: &CheckpointManifest) -> Result<VersionedMemEngine> {
    // Revalidate descriptors supplied via an API as well as those decoded from disk.
    let manifest = CheckpointManifest::decode(&manifest.encode()?)?;
    let mut builder = crate::versioned::CheckpointBuilder::default();
    for file in &manifest.files {
        let sst = verified_sst(store, file)?;
        for (key, value) in sst.iter() {
            builder.put(sst.column_family(), key, value)?;
        }
    }
    // Neither an engine nor its position escapes before every SST has passed
    // verification. A later-object failure drops the entire unpublished builder.
    Ok(builder.finish(manifest.position()))
}

fn verified_sst(store: &dyn ObjectStore, file: &SstReference) -> Result<SharedSst> {
    let bytes = store
        .get_shared_bounded(
            &ObjectKey::new(file.key.clone())?,
            usize::try_from(file.size)
                .map_err(|_| Error::Engine("SST read limit overflow".into()))?,
        )?
        .ok_or_else(|| Error::Engine("checkpoint references a missing SST".into()))?;
    check_sst_digest(file, &bytes)?;
    let sst = SharedSst::parse(bytes)?;
    check_sst_metadata(file, &sst.view())?;
    Ok(sst)
}
fn check_sst_digest(file: &SstReference, bytes: &[u8]) -> Result<()> {
    if bytes.len() as u64 != file.size || digest(bytes) != file.sha256 {
        return Err(Error::Engine(
            "checkpoint SST checksum or size mismatch".into(),
        ));
    }
    Ok(())
}
fn check_sst_metadata(file: &SstReference, sst: &SstView<'_>) -> Result<()> {
    if Some(sst.column_family()) != ColumnFamily::ALL.get(file.cf as usize).copied()
        || sst.smallest_key() != file.smallest
        || sst.largest_key() != file.largest
        || sst.len() as u64 != file.count
    {
        return Err(Error::Engine("checkpoint SST metadata mismatch".into()));
    }
    Ok(())
}
pub(crate) fn checked_sst_bytes<'a>(file: &SstReference, bytes: &'a [u8]) -> Result<SstView<'a>> {
    check_sst_digest(file, bytes)?;
    let sst = SstView::parse(bytes)?;
    check_sst_metadata(file, &sst)?;
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

#[cfg(test)]
mod memory_tests;
