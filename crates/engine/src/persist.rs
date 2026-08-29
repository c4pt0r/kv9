//! A durable [`Engine`]: an in-memory index in front of an append-only write-ahead log.
//!
//! This is the *simple* persistent engine — enough that a node can be killed and come back
//! with its data, which is what the multi-process Phase-1 acceptance needs. It is not the
//! disaggregated LSM of DESIGN §6.5: there are no SSTs, no compaction, no manifest, and no
//! object storage. Those are Phase 2, and this deliberately does not pretend to be them.
//!
//! ## Shape
//!
//! Every [`WriteBatch`] is appended to the log and fsynced **before** it is applied to the
//! in-memory index. That order is the whole point of a *write-ahead* log: if the process
//! dies between the two steps the record is on disk and replay applies it, so we lose
//! nothing. The reverse order would let a reader observe a value that then vanishes on
//! restart — durability must lead visibility, never trail it.
//!
//! Reads never touch the disk: the index is the authority for what is *visible*, the log
//! is the authority for what *survives*. Recovery reconciles them by replaying the log
//! into a fresh index.
//!
//! ## The cost, stated plainly
//!
//! The whole dataset lives in memory and the log grows without bound — there is no
//! compaction, so restart time and disk use grow with total writes, not with live data.
//! That is acceptable for a test/demo engine and unacceptable for production; the fix is
//! the Phase 2 flush-to-SST path, not a patch here.

use std::path::Path;
use std::sync::Mutex;

use kv9_common::{Result, Value};

use crate::cf::ColumnFamily;
use crate::mem::MemEngine;
use crate::wal::{Replay, Wal};
use crate::write_batch::WriteBatch;
use crate::{Durability, Engine, ReadView, ScanEntry};

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
        let (wal, replay) = Wal::open(path)?;
        let index = MemEngine::new();
        for batch in &replay.batches {
            // Replay goes straight to the index: these records are already durable, and
            // re-appending them would grow the log on every restart.
            index.write(batch.clone())?;
        }
        Ok((
            WalEngine {
                index,
                wal: Mutex::new(wal),
            },
            replay,
        ))
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

    /// The state machine stores its applied-index watermark under a `0x00`-prefixed key in
    /// the *data* column family, written in the same batch as the data it describes
    /// (`kv9_raft`'s `APPLIED_INDEX_KEY`). That pairing is what stops "durable data,
    /// volatile watermark" — but it only holds if such a key survives this engine's
    /// append/replay like any other.
    ///
    /// Nothing had exercised that: the watermark change was verified against `MemEngine`,
    /// where there is no log to round-trip through. `0x00` is not a byte any *physical*
    /// key starts with (those begin with a mode byte), so it is precisely the shape least
    /// likely to have been covered by accident.
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
