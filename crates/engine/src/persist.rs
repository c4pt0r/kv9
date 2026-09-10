//! A durable [`Engine`]: an in-memory index backed by a positioned WAL.
//!
//! Replicated data and its exact applied position share one fsynced record.
//! The runtime upgrades verified legacy markers before atomically switching to
//! segmented storage; recovery continues to read legacy v1/v2 layouts.
//!
//! With a [`RemoteUploader`], recovery restores a committed full-state SST
//! checkpoint and then its local WAL tail. Applied checkpoints permit atomic
//! whole-segment reclamation of the covered catalog WAL prefix. Upload happens
//! outside this engine's write lock and outside ordered Raft apply.
//!
//! Segmented recovery streams batches directly into the index. The full dataset
//! is still resident in memory; incremental LSM and group commit are later work.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use kv9_common::{AppliedPosition, Result, Value};

use crate::cf::ColumnFamily;
use crate::checkpoint::{CheckpointManifest, FlushScope, FrozenFlush, RemoteUploader};
use crate::mem::MemEngine;
use crate::wal::Wal;
use crate::wal_stream::{RecoveryPlan, SegmentedWal, DEFAULT_SEGMENT_BYTES};
use crate::write_batch::WriteBatch;
use crate::{Durability, DurableAppliedPosition, Engine, ReadView, ReplicatedEngine, ScanEntry};

/// An [`Engine`] whose writes survive a restart.
#[derive(Debug)]
pub struct WalEngine {
    /// Visible state. Rebuilt from the log at open.
    index: MemEngine,
    /// Durable state. Guarded separately so a write serializes on the log, which is also
    /// what keeps log order and index order identical.
    wal: Mutex<EngineWal>,
    io_metrics: std::sync::Arc<kv9_common::metrics::WalIoMetrics>,
}

/// Aggregate engine recovery evidence. Streaming recovery does not retain a
/// second copy of every recovered batch after building the visible index.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EngineReplay {
    pub replayed_records: u64,
    pub covered_records: u64,
    pub discarded_tail_bytes: u64,
}

#[derive(Debug)]
enum WalBacking {
    Legacy(Wal),
    Segmented(Box<SegmentedWal>),
}
#[derive(Debug)]
struct EngineWal {
    path: PathBuf,
    metrics: std::sync::Arc<kv9_common::metrics::WalIoMetrics>,
    // None fences writes during an ambiguous offline layout publication.
    backing: Option<WalBacking>,
}
impl EngineWal {
    fn ensure_available(&self) -> Result<()> {
        if self.backing.is_none() {
            return Err(kv9_common::Error::Engine(
                "WAL layout transition requires recovery".into(),
            ));
        }
        Ok(())
    }
    fn path(&self) -> &Path {
        &self.path
    }
    fn io_metrics(&self) -> std::sync::Arc<kv9_common::metrics::WalIoMetrics> {
        self.metrics.clone()
    }
    fn append(&mut self, batch: &WriteBatch) -> Result<()> {
        match &mut self.backing {
            Some(WalBacking::Legacy(wal)) => wal.append(batch),
            Some(WalBacking::Segmented(wal)) => wal.append(batch, None),
            None => Err(kv9_common::Error::Engine(
                "WAL layout transition requires recovery".into(),
            )),
        }
    }
    fn append_applied(&mut self, batch: &WriteBatch, at: AppliedPosition) -> Result<()> {
        match &mut self.backing {
            Some(WalBacking::Legacy(wal)) => wal.append_applied(batch, at),
            Some(WalBacking::Segmented(wal)) => wal.append(batch, Some(at)),
            None => Err(kv9_common::Error::Engine(
                "WAL layout transition requires recovery".into(),
            )),
        }
    }
}

impl WalEngine {
    /// Open the engine at `path`, replaying any existing log.
    ///
    /// Returns the engine and the [`EngineReplay`] report. The report is handed back rather than
    /// swallowed because `discarded_tail_bytes > 0` means an unclean shutdown truncated
    /// something — the caller should log that, not discover it later.
    pub fn open(path: impl AsRef<Path>) -> Result<(Self, EngineReplay)> {
        Self::open_with_uploader(path, None)
    }

    /// Rebuild from a committed local manifest reference, remote SSTs and the
    /// surviving WAL tail. Missing/corrupt remote state refuses the entire open.
    pub fn open_with_uploader(
        path: impl AsRef<Path>,
        uploader: Option<&RemoteUploader>,
    ) -> Result<(Self, EngineReplay)> {
        let path = path.as_ref();
        if path
            .with_extension("segments")
            .try_exists()
            .map_err(checkpoint_io)?
        {
            match std::fs::metadata(path) {
                Ok(metadata) if metadata.len() >= 5 => {}
                Ok(_) => {
                    return Err(kv9_common::Error::Engine(
                        "segment directory exists with a truncated WAL topology".into(),
                    ))
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Err(kv9_common::Error::Engine(
                        "segment directory exists without its WAL topology".into(),
                    ))
                }
                Err(e) => return Err(checkpoint_io(e)),
            }
        }
        if RecoveryPlan::is_segmented(path)? {
            let plan = RecoveryPlan::read(path)?;
            let index = std::cell::RefCell::new(MemEngine::new());
            let metrics = kv9_common::metrics::WalIoMetrics::shared();
            let (wal, report) = plan.recover(
                DEFAULT_SEGMENT_BYTES,
                metrics.clone(),
                |manifest| {
                    if let Some(manifest) = manifest {
                        let uploader = uploader.ok_or_else(|| {
                            kv9_common::Error::Config(
                                "remote checkpoint requires MinIO configuration".into(),
                            )
                        })?;
                        *index.borrow_mut() = uploader.restore(manifest)?;
                    }
                    Ok(())
                },
                |batch, position| match position {
                    Some(at) => index.borrow().write_applied(batch, at),
                    None => index.borrow().write(batch),
                },
            )?;
            return Ok((
                Self {
                    index: index.into_inner(),
                    io_metrics: metrics.clone(),
                    wal: Mutex::new(EngineWal {
                        path: path.to_path_buf(),
                        metrics,
                        backing: Some(WalBacking::Segmented(Box::new(wal))),
                    }),
                },
                EngineReplay {
                    replayed_records: report.replayed_records,
                    covered_records: report.covered_records,
                    discarded_tail_bytes: report.discarded_tail_bytes,
                },
            ));
        }
        let checkpoint_path = path.with_extension("checkpoint");
        let (index, base) = match std::fs::read(&checkpoint_path) {
            Ok(bytes) => {
                let manifest = CheckpointManifest::decode(&bytes)?;
                let uploader = uploader.ok_or_else(|| {
                    kv9_common::Error::Config(
                        "remote checkpoint requires MinIO configuration".into(),
                    )
                })?;
                (uploader.restore(&manifest)?, Some(manifest.position()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (MemEngine::new(), None),
            Err(e) => return Err(kv9_common::Error::Engine(format!("read checkpoint: {e}"))),
        };
        let (wal, replay) = Wal::open_strict(path)?;
        let mut report = EngineReplay {
            discarded_tail_bytes: replay.discarded_tail_bytes,
            ..EngineReplay::default()
        };
        for (batch, position) in replay.batches.iter().zip(&replay.positions) {
            // Replay goes straight to the index: these records are already durable, and
            // re-appending them would grow the log on every restart.
            if let Some(base) = base {
                if checkpoint_covers(base, *position)? {
                    report.covered_records += 1;
                    continue;
                }
            }
            match position {
                Some(at) => index.write_applied(batch.clone(), *at)?,
                None => index.write(batch.clone())?,
            }
            report.replayed_records += 1;
        }
        Ok((
            WalEngine {
                index,
                io_metrics: wal.io_metrics(),
                wal: Mutex::new(EngineWal {
                    path: path.to_path_buf(),
                    metrics: wal.io_metrics(),
                    backing: Some(WalBacking::Legacy(wal)),
                }),
            },
            report,
        ))
    }

    /// Seal data and position under the same write lock. This is an O(1)
    /// persistent-map snapshot; serialization and all remote I/O happen later.
    pub fn freeze(&self, scope: FlushScope) -> Result<FrozenFlush> {
        let wal = self.wal.lock().expect("wal lock poisoned");
        wal.ensure_available()?;
        let (view, position) = self.index.freeze_parts();
        let position = position
            .ok_or_else(|| kv9_common::Error::Engine("nothing applied to freeze".into()))?;
        Ok(FrozenFlush {
            view,
            position,
            scope,
        })
    }

    pub fn io_metrics(&self) -> std::sync::Arc<kv9_common::metrics::WalIoMetrics> {
        self.io_metrics.clone()
    }

    pub fn data_revision(&self) -> u64 {
        self.index.data_revision()
    }

    /// Read the actual selected checkpoint without opening or repairing a log.
    /// The runtime certifies these exact bytes against committed Raft history.
    pub fn checkpoint_reference(path: impl AsRef<Path>) -> Result<Option<CheckpointManifest>> {
        let path = path.as_ref();
        if RecoveryPlan::is_segmented(path)? {
            return Ok(RecoveryPlan::read(path)?.checkpoint().cloned());
        }
        match std::fs::read(path.with_extension("checkpoint")) {
            Ok(bytes) => Ok(Some(CheckpointManifest::decode(&bytes)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(checkpoint_io(e)),
        }
    }

    /// Offline, crash-safe transition into the segmented layout. The runtime
    /// calls this after checking recovered positions and legacy marker authority,
    /// before starting Raft ownership or public Serving. The old layout remains
    /// selected until every copied segment and its topology are durable.
    pub fn enable_segmentation(&self) -> Result<()> {
        self.enable_segmentation_with_target(DEFAULT_SEGMENT_BYTES)
    }

    fn enable_segmentation_with_target(&self, target_bytes: u64) -> Result<()> {
        let mut log = self.wal.lock().expect("wal lock poisoned");
        if matches!(log.backing, Some(WalBacking::Segmented(_))) {
            return Ok(());
        }
        let path = log.path.clone();
        let checkpoint = Self::checkpoint_reference(&path)?;
        let Some(WalBacking::Legacy(mut legacy)) = log.backing.take() else {
            return Err(kv9_common::Error::Engine(
                "WAL layout transition requires recovery".into(),
            ));
        };
        // A zero-length legacy tail is valid, but once segment directories
        // exist it is indistinguishable from a truncated new topology. Make
        // the old selected source explicitly framed and durable BEFORE any
        // staging directory can survive a failed migration. The empty record
        // changes no data; a checkpoint-backed tail retains its exact position.
        if std::fs::metadata(legacy.path())
            .map_err(checkpoint_io)?
            .len()
            == 0
        {
            let empty = WriteBatch::new();
            match self.index.volatile_applied_position() {
                Some(at) => legacy.append_applied(&empty, at)?,
                None => legacy.append(&empty)?,
            }
        }
        let temporary = path.with_extension("migration");
        match std::fs::remove_file(&temporary) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(checkpoint_io(e)),
        }
        let identity = *kv9_common::StoreIncarnation::mint()?.as_bytes();
        let mut migrated = SegmentedWal::create_with_checkpoint(
            &temporary,
            identity,
            target_bytes,
            log.io_metrics(),
            checkpoint.clone(),
        )?;
        Wal::visit_strict(legacy.path(), |batch, at| {
            // Unpositioned empty batches have neither data effects nor applied
            // authority. Omitting only those no-ops prevents the empty-source
            // framing record from pinning the new stream forever. Positioned
            // empty batches still carry progress and must be preserved.
            if at.is_none() && batch.is_empty() {
                return Ok(());
            }
            if let Some(base) = checkpoint.as_ref() {
                if checkpoint_covers(base.position(), at)? {
                    return Ok(());
                }
            }
            migrated.append(&batch, at)
        })?;
        if migrated.applied_position() != self.index.volatile_applied_position() {
            return Err(kv9_common::Error::Engine(
                "segmented migration changed the exact applied position".into(),
            ));
        }
        migrated.install_as(&path)?;
        log.backing = Some(WalBacking::Segmented(Box::new(migrated)));
        drop(legacy);
        Ok(())
    }

    /// Upgrade an old in-band watermark after the caller verifies its exact
    /// term against committed Raft history. Atomically replace the unpositioned
    /// WAL with one full-state positioned record, so it can later be reclaimed.
    /// A crash sees either the original marker/log or the complete v2 state.
    /// If the legacy state exceeds the single-record limit, preserve its prefix
    /// and append only the marker transition; reads/restarts remain available.
    pub fn upgrade_legacy_applied(&self, marker: &[u8], at: AppliedPosition) -> Result<()> {
        self.upgrade_legacy_with_limit(marker, at, crate::wal_v2::MAX_RECORD_LEN as usize)
    }

    fn upgrade_legacy_with_limit(
        &self,
        marker: &[u8],
        at: AppliedPosition,
        limit: usize,
    ) -> Result<()> {
        let mut log = self.wal.lock().expect("wal lock poisoned");
        let Some(WalBacking::Legacy(wal)) = log.backing.as_mut() else {
            return Err(kv9_common::Error::Engine(
                "legacy marker upgrade must precede segmentation".into(),
            ));
        };
        let (view, position) = self.index.freeze_parts();
        if position.is_some()
            || view.get(ColumnFamily::Default, marker)?.as_deref()
                != Some(at.index.to_be_bytes().as_slice())
        {
            return Err(kv9_common::Error::Engine(
                "legacy applied upgrade requires an exact unpositioned marker".into(),
            ));
        }
        let mut state = WriteBatch::new();
        let mut encoded_size = 4usize;
        for cf in ColumnFamily::ALL {
            for entry in view.iter_all(cf) {
                let (key, value) = entry?;
                if cf != ColumnFamily::Default || key != marker {
                    encoded_size = encoded_size
                        .saturating_add(10)
                        .saturating_add(key.len())
                        .saturating_add(value.len());
                    if encoded_size > limit {
                        // A large existing database must not become unopenable
                        // solely because migration cannot fit one WAL record.
                        // Keeping unpositioned records also keeps the reclaim ban.
                        let mut transition = WriteBatch::new();
                        transition.delete(ColumnFamily::Default, marker.to_vec());
                        wal.append_applied(&transition, at)?;
                        return self.index.write_applied(transition, at);
                    }
                    state.put(cf, key, value);
                }
            }
        }
        let tmp = wal.path().with_extension("upgrade.tmp");
        match std::fs::remove_file(&tmp) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(checkpoint_io(e)),
        }
        let (mut upgraded, _) = Wal::open_with_metrics(&tmp, self.io_metrics.clone())?;
        upgraded.append_applied(&state, at)?;
        std::fs::rename(&tmp, wal.path()).map_err(checkpoint_io)?;
        upgraded.relocated(wal.path().to_path_buf());
        *wal = upgraded;
        self.io_metrics
            .namespace_publish
            .measure(|| sync_parent(wal.path()))?;
        let mut remove_marker = WriteBatch::new();
        remove_marker.delete(ColumnFamily::Default, marker.to_vec());
        self.index.write_applied(remove_marker, at)
    }

    /// Install a reference whose ManifestChange has been confirmed by ordered
    /// Raft apply, then reclaim the covered local WAL prefix. The caller must
    /// supply the applied manifest, never a merely uploaded/prepared one.
    ///
    /// Segmented storage publishes the new recovery anchor before unlinking
    /// covered closed segments. Legacy storage retains its copy/rename path
    /// until the caller completes the offline layout transition.
    pub fn checkpoint_applied(&self, manifest: &CheckpointManifest) -> Result<bool> {
        use std::io::Write;
        let mut log = self.wal.lock().expect("wal lock poisoned");
        let wal = match log.backing.as_mut() {
            Some(WalBacking::Segmented(wal)) => return wal.checkpoint_applied(manifest),
            Some(WalBacking::Legacy(wal)) => wal,
            None => {
                return Err(kv9_common::Error::Engine(
                    "WAL layout transition requires recovery".into(),
                ))
            }
        };
        let position = self
            .index
            .volatile_applied_position()
            .ok_or_else(|| kv9_common::Error::Engine("checkpoint before apply".into()))?;
        if manifest.index > position.index {
            return Err(kv9_common::Error::Engine(
                "checkpoint ahead of applied data".into(),
            ));
        }
        let (reader, replay) = Wal::open_with_metrics(wal.path(), self.io_metrics.clone())?;
        drop(reader);
        // Unpositioned records carry no reclaim authority; retain the entire log.
        if replay.positions.iter().any(Option::is_none) {
            return Ok(false);
        }
        let checkpoint_path = wal.path().with_extension("checkpoint");
        match std::fs::read(&checkpoint_path) {
            Ok(bytes) if CheckpointManifest::decode(&bytes)?.index > manifest.index => {
                return Ok(false)
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(checkpoint_io(e)),
        }
        let tmp = checkpoint_path.with_extension("checkpoint.tmp");
        let mut file = std::fs::File::create(&tmp).map_err(checkpoint_io)?;
        file.write_all(&manifest.encode()?).map_err(checkpoint_io)?;
        file.sync_all().map_err(checkpoint_io)?;
        std::fs::rename(&tmp, &checkpoint_path).map_err(checkpoint_io)?;
        sync_parent(&checkpoint_path)?;
        // From here both old-full-WAL and new-tail-WAL recover correctly against
        // the durable checkpoint. A failed rewrite leaves the old WAL live.
        let tail_path = wal.path().with_extension("tail.tmp");
        match std::fs::remove_file(&tail_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(checkpoint_io(e)),
        }
        let (mut tail, _) = Wal::open_with_metrics(&tail_path, self.io_metrics.clone())?;
        for (batch, at) in replay.batches.iter().zip(&replay.positions) {
            if let Some(at) = at.filter(|at| at.index > manifest.index) {
                tail.append_applied(batch, at)?;
            }
        }
        std::fs::rename(&tail_path, wal.path()).map_err(checkpoint_io)?;
        // Switch the live file handle immediately after rename. Even if the
        // directory sync fails, future appends must never hit the unlinked inode.
        tail.relocated(wal.path().to_path_buf());
        *wal = tail;
        self.io_metrics
            .namespace_publish
            .measure(|| sync_parent(wal.path()))?;
        Ok(true)
    }

    /// The log's path, for diagnostics.
    pub fn path(&self) -> std::path::PathBuf {
        self.wal
            .lock()
            .expect("wal lock poisoned")
            .path()
            .to_path_buf()
    }
}

// A checkpoint may replace only the prefix of the same monotonic history.
// Exact committed authority remains the runtime caller's obligation.
fn checkpoint_covers(base: AppliedPosition, at: Option<AppliedPosition>) -> Result<bool> {
    let at = at.ok_or_else(|| {
        kv9_common::Error::Engine("unpositioned WAL record beside a remote checkpoint".into())
    })?;
    let agrees = match at.index.cmp(&base.index) {
        std::cmp::Ordering::Less => at.term <= base.term,
        std::cmp::Ordering::Equal => at.term == base.term,
        std::cmp::Ordering::Greater => at.term >= base.term,
    };
    if !agrees {
        return Err(kv9_common::Error::Engine(
            "WAL record disagrees with checkpoint term".into(),
        ));
    }
    Ok(at.index <= base.index)
}

fn checkpoint_io(e: std::io::Error) -> kv9_common::Error {
    kv9_common::Error::Engine(format!("checkpoint io: {e}"))
}
fn sync_parent(path: &Path) -> Result<()> {
    std::fs::File::open(
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )
    .and_then(|f| f.sync_all())
    .map_err(checkpoint_io)
}

impl Engine for WalEngine {
    fn get(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<Value>> {
        self.index.get(cf, key)
    }

    fn write(&self, batch: WriteBatch) -> Result<()> {
        // Durable first, visible second. Holding the log lock across both steps keeps the
        // log's order and the index's order the same, so replay reconstructs exactly the
        // state readers saw.
        let mut wal = self.wal.lock().expect("wal lock poisoned");
        wal.append(&batch)?;
        self.index.write(batch)?;
        Ok(())
    }

    fn scan(
        &self,
        cf: ColumnFamily,
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<ScanEntry>> {
        self.index.scan(cf, start, end, limit)
    }

    fn delete_range(&self, cf: ColumnFamily, start: &[u8], end: &[u8]) -> Result<()> {
        // Expand the range to explicit deletes so the log records exactly what happened.
        // Recording the *range* instead would be smaller, but replaying it against a
        // different index state could delete keys the original call never touched.
        let doomed = self.index.scan(cf, start, end, usize::MAX)?;
        if doomed.is_empty() {
            return Ok(());
        }
        let mut batch = WriteBatch::new();
        for (k, _) in doomed {
            batch.delete(cf, k);
        }
        self.write(batch)
    }

    fn checksum(&self, cf: ColumnFamily, start: &[u8], end: &[u8]) -> Result<u64> {
        self.index.checksum(cf, start, end)
    }

    fn snapshot(&self) -> Result<Box<dyn ReadView + '_>> {
        self.index.snapshot()
    }

    fn durability(&self) -> Durability {
        // Every accepted write was fsynced before it became visible, so anything a reader
        // can see has already landed.
        Durability::DurableThroughLastWrite
    }
}

impl ReplicatedEngine for WalEngine {
    fn write_applied(&self, batch: WriteBatch, at: AppliedPosition) -> Result<()> {
        let mut wal = self.wal.lock().expect("wal lock poisoned");
        if self
            .index
            .volatile_applied_position()
            .is_some_and(|p| at.index <= p.index)
        {
            return Err(kv9_common::Error::Engine(
                "applied position must advance".into(),
            ));
        }
        wal.append_applied(&batch, at)?;
        self.index.write_applied(batch, at)
    }

    fn applied_position(&self) -> Result<DurableAppliedPosition> {
        // Serialize with writes: the reported position describes the visible index
        // and its durable record, never the midpoint between append and publication.
        let wal = self.wal.lock().expect("wal lock poisoned");
        wal.ensure_available()?;
        Ok(match self.index.volatile_applied_position() {
            Some(at) => DurableAppliedPosition::AppliedThrough(at),
            None => DurableAppliedPosition::AppliedNothing,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("kv9-persist-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(engine: &WalEngine, k: &[u8], v: &[u8]) {
        let mut b = WriteBatch::new();
        b.put(ColumnFamily::Default, k.to_vec(), v.to_vec());
        engine.write(b).unwrap();
    }

    fn scope() -> FlushScope {
        FlushScope {
            cluster: "segmented-engine-test".into(),
            region: 1,
            conf_ver: 1,
            version: 1,
        }
    }

    fn positioned(engine: &WalEngine, index: u64) {
        let mut batch = WriteBatch::new();
        for cf in ColumnFamily::ALL {
            batch.put(cf, index.to_be_bytes().to_vec(), vec![index as u8; 40]);
        }
        engine
            .write_applied(batch, AppliedPosition { term: 2, index })
            .unwrap();
    }

    #[test]
    fn segmentation_preserves_legacy_data_exact_progress_and_write_observer() {
        let path = tmpdir("segmented-migration").join("catalog.wal");
        let (engine, _) = WalEngine::open(&path).unwrap();
        put(&engine, b"legacy", b"pinned");
        for index in [1, 7, 20] {
            positioned(&engine, index);
        }
        let metrics = engine.io_metrics();
        engine.enable_segmentation_with_target(256).unwrap();
        assert!(RecoveryPlan::is_segmented(&path).unwrap());
        assert!(std::sync::Arc::ptr_eq(
            &metrics,
            &engine.wal.lock().unwrap().io_metrics()
        ));
        let topology = std::fs::read(&path).unwrap();
        assert!(
            Wal::open(&path).is_err(),
            "old writers must reject the new layout"
        );
        assert_eq!(std::fs::read(&path).unwrap(), topology);
        positioned(&engine, 33);
        drop(engine);
        let (recovered, report) = WalEngine::open(&path).unwrap();
        assert_eq!(report.replayed_records, 5);
        assert_eq!(report.discarded_tail_bytes, 0);
        assert_eq!(
            recovered.get(ColumnFamily::Default, b"legacy").unwrap(),
            Some(b"pinned".to_vec())
        );
        for index in [1u64, 7, 20, 33] {
            for cf in ColumnFamily::ALL {
                assert_eq!(
                    recovered.get(cf, &index.to_be_bytes()).unwrap(),
                    Some(vec![index as u8; 40])
                );
            }
        }
        assert_eq!(
            recovered.applied_position().unwrap(),
            DurableAppliedPosition::AppliedThrough(AppliedPosition { term: 2, index: 33 })
        );
        positioned(&recovered, 40);
    }

    #[test]
    fn failed_layout_staging_preserves_old_root_and_fences_the_owner() {
        let path = tmpdir("segmented-failed-staging").join("catalog.wal");
        let (engine, _) = WalEngine::open(&path).unwrap();
        positioned(&engine, 10);
        let original = std::fs::read(&path).unwrap();
        std::fs::create_dir(path.with_extension("migration")).unwrap();
        assert!(engine.enable_segmentation().is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert!(engine
            .write_applied(WriteBatch::new(), AppliedPosition { term: 2, index: 11 })
            .is_err());
        assert!(engine.applied_position().is_err());
        assert!(engine.freeze(scope()).is_err());
        drop(engine);
        std::fs::remove_dir(path.with_extension("migration")).unwrap();
        let (engine, _) = WalEngine::open(&path).unwrap();
        engine.enable_segmentation().unwrap();
        assert_eq!(
            engine.applied_position().unwrap(),
            DurableAppliedPosition::AppliedThrough(AppliedPosition { term: 2, index: 10 })
        );
    }

    #[test]
    fn engine_refuses_complete_legacy_corruption_and_missing_segmented_roots() {
        let path = tmpdir("segmented-corruption").join("catalog.wal");
        let (engine, _) = WalEngine::open(&path).unwrap();
        positioned(&engine, 10);
        drop(engine);
        let original = std::fs::read(&path).unwrap();
        let mut damaged = original.clone();
        *damaged.last_mut().unwrap() ^= 1;
        std::fs::write(&path, &damaged).unwrap();
        assert!(WalEngine::open(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), damaged);
        std::fs::write(&path, original).unwrap();
        let (engine, _) = WalEngine::open(&path).unwrap();
        engine.enable_segmentation().unwrap();
        drop(engine);
        let topology = std::fs::read(&path).unwrap();
        for size in 0..5 {
            std::fs::write(&path, &topology[..size]).unwrap();
            assert!(WalEngine::open(&path).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), topology[..size]);
        }
        std::fs::remove_file(&path).unwrap();
        assert!(WalEngine::open(&path).is_err());
        assert!(
            !path.exists(),
            "missing topology must not be recreated as an empty legacy WAL"
        );
    }

    #[test]
    fn legacy_upgrade_rewrites_all_state_and_leaves_only_positioned_records() {
        let path = tmpdir("legacy-upgrade").join("catalog.wal");
        let marker = b"\x00kv9\x00applied_index";
        let at = AppliedPosition { term: 3, index: 17 };
        {
            let (engine, _) = WalEngine::open(&path).unwrap();
            put(&engine, b"obsolete", b"gone");
            let mut batch = WriteBatch::new();
            for cf in ColumnFamily::ALL {
                batch.put(cf, vec![255, 255], b"kept".to_vec());
            }
            batch.delete(ColumnFamily::Default, b"obsolete".to_vec());
            batch.put(
                ColumnFamily::Default,
                marker.to_vec(),
                at.index.to_be_bytes().to_vec(),
            );
            engine.write(batch).unwrap();
            let original = std::fs::read(&path).unwrap();
            assert!(engine
                .upgrade_legacy_applied(marker, AppliedPosition { index: 18, ..at })
                .is_err());
            assert_eq!(
                std::fs::read(&path).unwrap(),
                original,
                "refused migration cannot edit WAL"
            );
            let observer = engine.io_metrics();
            let writes_before = observer.write.snapshot().outcomes
                [kv9_common::metrics::Outcome::Success as usize]
                .count;
            engine.upgrade_legacy_applied(marker, at).unwrap();
            assert!(std::sync::Arc::ptr_eq(
                &observer,
                &engine.wal.lock().unwrap().io_metrics()
            ));
            assert!(
                observer.write.snapshot().outcomes[kv9_common::metrics::Outcome::Success as usize]
                    .count
                    > writes_before,
                "WAL replacement lost its process-lifetime observer"
            );

            assert!(engine.get(ColumnFamily::Default, marker).unwrap().is_none());
            assert_eq!(
                engine.applied_position().unwrap(),
                DurableAppliedPosition::AppliedThrough(at)
            );
        }
        let (engine, report) = WalEngine::open(&path).unwrap();
        assert_eq!(report.replayed_records, 1);
        let (_, replay) = Wal::open(&path).unwrap();
        assert_eq!(
            replay.positions,
            vec![Some(at)],
            "legacy prefix must be reclaimable"
        );
        for cf in ColumnFamily::ALL {
            assert_eq!(engine.get(cf, &[255, 255]).unwrap(), Some(b"kept".to_vec()));
        }
        assert!(engine
            .get(ColumnFamily::Default, b"obsolete")
            .unwrap()
            .is_none());
        assert!(engine.get(ColumnFamily::Default, marker).unwrap().is_none());
        engine
            .write_applied(WriteBatch::new(), AppliedPosition { index: 18, ..at })
            .unwrap();
        drop(engine);
        assert_eq!(
            WalEngine::open(&path)
                .unwrap()
                .0
                .applied_position()
                .unwrap(),
            DurableAppliedPosition::AppliedThrough(AppliedPosition { index: 18, ..at }),
            "new positioned writes must remain durable after migration"
        );
    }

    #[test]
    fn legacy_upgrade_preserves_large_histories_when_a_full_record_would_overflow() {
        let path = tmpdir("legacy-large").join("catalog.wal");
        let marker = b"\x00kv9\x00applied_index";
        let at = AppliedPosition { term: 2, index: 5 };
        let (engine, _) = WalEngine::open(&path).unwrap();
        put(&engine, b"large", &[7; 128]);
        put(&engine, marker, &at.index.to_be_bytes());
        engine.upgrade_legacy_with_limit(marker, at, 64).unwrap();
        drop(engine);
        let (recovered, _) = WalEngine::open(&path).unwrap();
        let (_, replay) = Wal::open(&path).unwrap();
        assert!(
            replay.positions.iter().any(Option::is_none),
            "oversized legacy prefix must be retained"
        );
        assert_eq!(
            recovered.applied_position().unwrap(),
            DurableAppliedPosition::AppliedThrough(at)
        );
        assert_eq!(
            recovered.get(ColumnFamily::Default, b"large").unwrap(),
            Some(vec![7; 128])
        );
        assert_eq!(recovered.get(ColumnFamily::Default, marker).unwrap(), None);
    }

    /// The point of the whole exercise: kill the process, come back with the data.
    #[test]
    fn data_survives_reopen() {
        let path = tmpdir("survives").join("wal");
        {
            let (e, _) = WalEngine::open(&path).unwrap();
            put(&e, b"a", b"1");
            put(&e, b"b", b"2");
            let mut del = WriteBatch::new();
            del.delete(ColumnFamily::Default, b"a".to_vec());
            e.write(del).unwrap();
        }
        let (e, replay) = WalEngine::open(&path).unwrap();
        assert_eq!(replay.discarded_tail_bytes, 0);
        assert_eq!(
            e.get(ColumnFamily::Default, b"a").unwrap(),
            None,
            "the delete survived too"
        );
        assert_eq!(
            e.get(ColumnFamily::Default, b"b").unwrap(),
            Some(b"2".to_vec())
        );
    }

    /// Control for the test above: without it, `data_survives_reopen` would also pass
    /// against an engine that simply never forgot anything because nothing was ever
    /// removed. A fresh directory must come back empty.
    #[test]
    fn a_fresh_engine_is_empty() {
        let path = tmpdir("fresh").join("wal");
        let (e, replay) = WalEngine::open(&path).unwrap();
        assert_eq!(replay.replayed_records, 0);
        assert_eq!(e.get(ColumnFamily::Default, b"a").unwrap(), None);
    }

    #[test]
    fn cross_column_family_state_survives() {
        let path = tmpdir("cfs").join("wal");
        {
            let (e, _) = WalEngine::open(&path).unwrap();
            let mut b = WriteBatch::new();
            b.put(ColumnFamily::Default, b"k".to_vec(), b"d".to_vec());
            b.put(ColumnFamily::Lock, b"k".to_vec(), b"l".to_vec());
            b.put(ColumnFamily::Write, b"k".to_vec(), b"w".to_vec());
            e.write(b).unwrap();
        }
        let (e, _) = WalEngine::open(&path).unwrap();
        assert_eq!(
            e.get(ColumnFamily::Lock, b"k").unwrap(),
            Some(b"l".to_vec())
        );
        assert_eq!(
            e.get(ColumnFamily::Write, b"k").unwrap(),
            Some(b"w".to_vec())
        );
    }

    #[test]
    fn delete_range_survives_reopen() {
        let path = tmpdir("delrange").join("wal");
        {
            let (e, _) = WalEngine::open(&path).unwrap();
            for k in [b"a", b"b", b"c", b"d"] {
                put(&e, k, b"v");
            }
            e.delete_range(ColumnFamily::Default, b"b", b"d").unwrap();
        }
        let (e, _) = WalEngine::open(&path).unwrap();
        assert!(e.get(ColumnFamily::Default, b"a").unwrap().is_some());
        assert!(e.get(ColumnFamily::Default, b"b").unwrap().is_none());
        assert!(e.get(ColumnFamily::Default, b"c").unwrap().is_none());
        assert!(e.get(ColumnFamily::Default, b"d").unwrap().is_some());
    }

    /// A crash mid-append must not cost previously committed writes.
    #[test]
    fn committed_writes_survive_a_torn_tail() {
        let path = tmpdir("torn").join("wal");
        {
            let (e, _) = WalEngine::open(&path).unwrap();
            put(&e, b"a", b"1");
            put(&e, b"b", b"2");
        }
        // Simulate dying part-way through writing a third record.
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            f.write_all(b"KV9W\x01\x20\x00\x00").unwrap();
        }
        let (e, replay) = WalEngine::open(&path).unwrap();
        assert!(
            replay.discarded_tail_bytes > 0,
            "the tear should be reported"
        );
        assert_eq!(
            e.get(ColumnFamily::Default, b"a").unwrap(),
            Some(b"1".to_vec())
        );
        assert_eq!(
            e.get(ColumnFamily::Default, b"b").unwrap(),
            Some(b"2".to_vec())
        );
    }

    /// Writes made after recovering from a torn log must themselves survive — otherwise
    /// recovery appears to work but leaves the log permanently unwritable.
    #[test]
    fn writes_after_recovery_also_survive() {
        let path = tmpdir("after").join("wal");
        {
            let (e, _) = WalEngine::open(&path).unwrap();
            put(&e, b"a", b"1");
        }
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            f.write_all(b"KV9W\x01\x99").unwrap();
        }
        {
            let (e, _) = WalEngine::open(&path).unwrap();
            put(&e, b"c", b"3");
        }
        let (e, replay) = WalEngine::open(&path).unwrap();
        assert_eq!(replay.discarded_tail_bytes, 0);
        assert_eq!(
            e.get(ColumnFamily::Default, b"c").unwrap(),
            Some(b"3".to_vec())
        );
        assert_eq!(
            e.get(ColumnFamily::Default, b"a").unwrap(),
            Some(b"1".to_vec())
        );
    }

    /// The batch-atomicity contract must hold across a restart too: a batch is one log
    /// record, so replay applies all of it or none of it.
    #[test]
    fn a_batch_is_all_or_nothing_across_restart() {
        let path = tmpdir("atomic").join("wal");
        {
            let (e, _) = WalEngine::open(&path).unwrap();
            let mut b = WriteBatch::new();
            b.put(ColumnFamily::Default, b"x".to_vec(), b"1".to_vec());
            b.put(ColumnFamily::Lock, b"x".to_vec(), b"1".to_vec());
            e.write(b).unwrap();
        }
        // Chop one byte off: the whole record is now incomplete.
        let len = std::fs::metadata(&path).unwrap().len();
        let f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        f.set_len(len - 1).unwrap();
        drop(f);

        let (e, _) = WalEngine::open(&path).unwrap();
        assert_eq!(
            e.get(ColumnFamily::Default, b"x").unwrap(),
            None,
            "half a batch must not be recovered"
        );
        assert_eq!(e.get(ColumnFamily::Lock, b"x").unwrap(), None);
    }

    #[test]
    fn durability_is_reported_honestly() {
        let path = tmpdir("durability").join("wal");
        let (e, _) = WalEngine::open(&path).unwrap();
        assert_eq!(e.durability(), Durability::DurableThroughLastWrite);
        // ...whereas the volatile engine must never claim otherwise.
        assert_eq!(MemEngine::new().durability(), Durability::Volatile);
    }

    /// Data and the exact applied position share one positioned WAL-v2 record and
    /// CRC. Replay restores both together. A partial final record restores neither;
    /// a complete record with an invalid position order refuses the whole open.
    #[test]
    fn a_reserved_prefix_key_survives_replay_with_its_data() {
        let path = tmpdir("watermark").join("wal");
        let watermark = b"\x00kv9\x00applied_index".to_vec();
        // Precondition, not decoration: this test writes and reads the *same* literal, so
        // a mis-escaped key would sail through while exercising an ordinary key instead of
        // the reserved prefix. Pin that the first byte really is NUL.
        assert_eq!(watermark[0], 0u8, "the reserved prefix must be a NUL byte");
        {
            let (e, _) = WalEngine::open(&path).unwrap();
            // Exactly the shape the state machine writes: data and watermark, one batch.
            let mut b = WriteBatch::new();
            b.put(ColumnFamily::Default, b"tkey".to_vec(), b"row".to_vec());
            b.put(
                ColumnFamily::Default,
                watermark.clone(),
                7u64.to_be_bytes().to_vec(),
            );
            e.write(b).unwrap();

            // A second round, to catch a replay that only ever restores the first record.
            let mut b = WriteBatch::new();
            b.put(ColumnFamily::Default, b"tkey".to_vec(), b"row2".to_vec());
            b.put(
                ColumnFamily::Default,
                watermark.clone(),
                9u64.to_be_bytes().to_vec(),
            );
            e.write(b).unwrap();
        }

        let (e, replay) = WalEngine::open(&path).unwrap();
        assert_eq!(replay.discarded_tail_bytes, 0);
        assert_eq!(
            e.get(ColumnFamily::Default, &watermark).unwrap(),
            Some(9u64.to_be_bytes().to_vec()),
            "the watermark must come back at its latest value, not its first"
        );
        assert_eq!(
            e.get(ColumnFamily::Default, b"tkey").unwrap(),
            Some(b"row2".to_vec()),
            "and the data it describes must come back with it"
        );
    }

    /// The pairing, not just the presence: a torn tail must not restore the data of a
    /// batch while losing its watermark, or the two would disagree after a crash — the
    /// exact mismatch writing them in one batch is meant to prevent.
    #[test]
    fn data_and_watermark_are_lost_together_or_not_at_all() {
        let path = tmpdir("watermark-torn").join("wal");
        let watermark = b"\x00kv9\x00applied_index".to_vec();
        {
            let (e, _) = WalEngine::open(&path).unwrap();
            let mut b = WriteBatch::new();
            b.put(ColumnFamily::Default, b"tkey".to_vec(), b"row".to_vec());
            b.put(
                ColumnFamily::Default,
                watermark.clone(),
                1u64.to_be_bytes().to_vec(),
            );
            e.write(b).unwrap();

            let mut b = WriteBatch::new();
            b.put(ColumnFamily::Default, b"tkey".to_vec(), b"row2".to_vec());
            b.put(
                ColumnFamily::Default,
                watermark.clone(),
                2u64.to_be_bytes().to_vec(),
            );
            e.write(b).unwrap();
        }
        // Cut inside the second record so it cannot be recovered.
        let len = std::fs::metadata(&path).unwrap().len();
        let f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        f.set_len(len - 1).unwrap();
        drop(f);

        let (e, _) = WalEngine::open(&path).unwrap();
        let data = e.get(ColumnFamily::Default, b"tkey").unwrap();
        let mark = e.get(ColumnFamily::Default, &watermark).unwrap();
        assert_eq!(data, Some(b"row".to_vec()), "the first batch stands");
        assert_eq!(
            mark,
            Some(1u64.to_be_bytes().to_vec()),
            "the watermark must match the data: both at the first batch, never split"
        );
    }

    #[test]
    fn snapshots_work_on_the_durable_engine_too() {
        let path = tmpdir("snap").join("wal");
        let (e, _) = WalEngine::open(&path).unwrap();
        put(&e, b"k", b"v1");
        let view = e.snapshot().unwrap();
        put(&e, b"k", b"v2");
        assert_eq!(
            view.get(ColumnFamily::Default, b"k").unwrap(),
            Some(b"v1".to_vec()),
            "a view must keep its version on the durable engine as well"
        );
    }
}
