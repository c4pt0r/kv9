//! A durable [`Engine`]: an in-memory index backed by a positioned WAL.
//!
//! Replicated data and its exact applied position are one fsynced WAL v2
//! record, published together. Old v1 records remain readable and a verified
//! legacy in-band marker can be upgraded by atomic file replacement.
//!
//! With a [`RemoteUploader`], recovery restores a committed full-state SST
//! checkpoint and then its local WAL tail. Applied checkpoints permit atomic
//! copy/rename reclamation of the covered catalog WAL prefix. Upload happens
//! outside this engine's write lock and outside ordered Raft apply.
//!
//! The full dataset is still resident in memory. This is the initial remote
//! checkpoint engine, not an incremental LSM or a segmented/group-commit WAL.

use std::path::Path;
use std::sync::Mutex;

use kv9_common::{AppliedPosition, Result, Value};

use crate::cf::ColumnFamily;
use crate::checkpoint::{CheckpointManifest, FlushScope, FrozenFlush, RemoteUploader};
use crate::mem::MemEngine;
use crate::wal::{Replay, Wal};
use crate::write_batch::WriteBatch;
use crate::{Durability, DurableAppliedPosition, Engine, ReadView, ReplicatedEngine, ScanEntry};

/// An [`Engine`] whose writes survive a restart.
#[derive(Debug)]
pub struct WalEngine {
    /// Visible state. Rebuilt from the log at open.
    index: MemEngine,
    /// Durable state. Guarded separately so a write serializes on the log, which is also
    /// what keeps log order and index order identical.
    wal: Mutex<Wal>,
}

impl WalEngine {
    /// Open the engine at `path`, replaying any existing log.
    ///
    /// Returns the engine and the [`Replay`] report. The report is handed back rather than
    /// swallowed because `discarded_tail_bytes > 0` means an unclean shutdown truncated
    /// something — the caller should log that, not discover it later.
    pub fn open(path: impl AsRef<Path>) -> Result<(Self, Replay)> {
        Self::open_with_uploader(path, None)
    }

    /// Rebuild from a committed local manifest reference, remote SSTs and the
    /// surviving WAL tail. Missing/corrupt remote state refuses the entire open.
    pub fn open_with_uploader(
        path: impl AsRef<Path>,
        uploader: Option<&RemoteUploader>,
    ) -> Result<(Self, Replay)> {
        let checkpoint_path = path.as_ref().with_extension("checkpoint");
        let (index, base) = match std::fs::read(&checkpoint_path) {
            Ok(bytes) => {
                let manifest = CheckpointManifest::decode(&bytes)?;
                let uploader = uploader.ok_or_else(|| {
                    kv9_common::Error::Config(
                        "remote checkpoint requires MinIO configuration".into(),
                    )
                })?;
                (uploader.restore(&manifest)?, Some(manifest.index))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (MemEngine::new(), None),
            Err(e) => return Err(kv9_common::Error::Engine(format!("read checkpoint: {e}"))),
        };
        let (wal, replay) = Wal::open(path)?;
        for (batch, position) in replay.batches.iter().zip(&replay.positions) {
            // Replay goes straight to the index: these records are already durable, and
            // re-appending them would grow the log on every restart.
            if base.is_some() && position.is_none() {
                return Err(kv9_common::Error::Engine(
                    "unpositioned WAL record beside a remote checkpoint".into(),
                ));
            }
            match position {
                Some(at) if base.is_some_and(|base| at.index <= base) => continue,
                Some(at) => index.write_applied(batch.clone(), *at)?,
                None => index.write(batch.clone())?,
            }
        }
        Ok((
            WalEngine {
                index,
                wal: Mutex::new(wal),
            },
            replay,
        ))
    }

    /// Seal data and position under the same write lock. This is an O(1)
    /// persistent-map snapshot; serialization and all remote I/O happen later.
    pub fn freeze(&self, scope: FlushScope) -> Result<FrozenFlush> {
        let _wal = self.wal.lock().expect("wal lock poisoned");
        let (view, position) = self.index.freeze_parts();
        let position = position
            .ok_or_else(|| kv9_common::Error::Engine("nothing applied to freeze".into()))?;
        Ok(FrozenFlush {
            view,
            position,
            scope,
        })
    }

    pub fn data_revision(&self) -> u64 {
        self.index.data_revision()
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
        let mut wal = self.wal.lock().expect("wal lock poisoned");
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
        let (mut upgraded, _) = Wal::open(&tmp)?;
        upgraded.append_applied(&state, at)?;
        std::fs::rename(&tmp, wal.path()).map_err(checkpoint_io)?;
        upgraded.relocated(wal.path().to_path_buf());
        *wal = upgraded;
        sync_parent(wal.path())?;
        let mut remove_marker = WriteBatch::new();
        remove_marker.delete(ColumnFamily::Default, marker.to_vec());
        self.index.write_applied(remove_marker, at)
    }

    /// Install a reference whose ManifestChange has been confirmed by ordered
    /// Raft apply, then reclaim the covered local WAL prefix. The caller must
    /// supply the applied manifest, never a merely uploaded/prepared one.
    ///
    /// Copy/rename is the initial WAL compaction strategy: the old file remains
    /// valid until a fully fsynced tail replaces it. Segment unlink will replace
    /// this O(tail) operation when group commit/segmented WAL lands.
    pub fn checkpoint_applied(&self, manifest: &CheckpointManifest) -> Result<bool> {
        use std::io::Write;
        let mut wal = self.wal.lock().expect("wal lock poisoned");
        let position = self
            .index
            .volatile_applied_position()
            .ok_or_else(|| kv9_common::Error::Engine("checkpoint before apply".into()))?;
        if manifest.index > position.index {
            return Err(kv9_common::Error::Engine(
                "checkpoint ahead of applied data".into(),
            ));
        }
        let (reader, replay) = Wal::open(wal.path())?;
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
        let (mut tail, _) = Wal::open(&tail_path)?;
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
        sync_parent(wal.path())?;
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
        let _wal = self.wal.lock().expect("wal lock poisoned");
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
            engine.upgrade_legacy_applied(marker, at).unwrap();
            assert!(engine.get(ColumnFamily::Default, marker).unwrap().is_none());
            assert_eq!(
                engine.applied_position().unwrap(),
                DurableAppliedPosition::AppliedThrough(at)
            );
        }
        let (engine, replay) = WalEngine::open(&path).unwrap();
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
        let (recovered, replay) = WalEngine::open(&path).unwrap();
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
        assert!(replay.batches.is_empty());
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
