//! Durable raft storage (Phase 1-final): HardState + log on local disk.
//!
//! **This is a raft safety requirement, not a durability nicety**: a vote must
//! be persisted before the reply leaves the node. A restarted node on volatile
//! storage forgets its vote and can vote twice in the same term — two leaders.
//! The Ready loop guarantees the ordering (persist → then send); this type
//! guarantees the persistence.
//!
//! Format: one append-only file, `raft.log`, of self-delimiting records:
//!
//! ```text
//! len   u32 BE   (record body length)
//! fnv   u32 BE   (FNV-1a over the body)
//! body  = kind u8 (1 = ConfState, 2 = HardState, 3 = Entry; protobuf payload)
//! ```
//!
//! Crash semantics (aligned with the engine WAL contract):
//! - a **short or checksum-failing** record is the normal crash shape (torn
//!   tail): replay keeps everything before it and truncates the tail;
//! - a record that **passes its checksum but cannot be decoded** (unknown kind
//!   or protobuf parse failure) is a real inconsistency, not a tail — `open`
//!   returns a typed error instead of silently dropping data. Guessing is
//!   worse than stopping (DESIGN principle "never panic on the unknown").
//!   HardState/ConfState records are last-write-wins; Entry records append (a
//!   re-appended index overwrites the suffix, mirroring raft log truncation).
//!
//! Scope honesty: no compaction, no snapshots-to-disk — the log grows until
//! Phase 2's real log store (DESIGN "6.4 Raft log vs. WAL stream") replaces
//! this. `persisted()`-style flush watermarks live in the engine (Ren's lane);
//! this file is only the raft-protocol state.

use kv9_common::metrics::WalIoMetrics;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use protobuf::Message as PbMessage;
use raft::prelude::{ConfState, Entry, HardState};
use raft::storage::MemStorage;
use raft::{GetEntriesContext, RaftState};

use kv9_common::fs::{self, DurableFile, FileSystem, OsFileSystem};
use kv9_common::{Error, Result};

use crate::lease_policy::{LeaseEpoch, LeasePolicy};
use crate::rawnode::PersistentRaftStorage;

const REC_CONF_STATE: u8 = 1;
const REC_HARD_STATE: u8 = 2;
const REC_ENTRY: u8 = 3;
/// A post-conf-change membership PAIRED with the log index it took effect at
/// (8-byte BE index + ConfState protobuf). One crash-safe record: the index is
/// the replay guard, the ConfState is the membership — splitting them would
/// let a crash land between the two (task #24). Kind 1 remains the index-0
/// initial configuration written at open.
const REC_CONF_STATE_AT: u8 = 4;
/// Always decoded. Builds predating leases refuse this unknown record kind.
const REC_LEASE_EPOCH: u8 = 5;

/// Max record body; anything larger is corrupt (same spirit as the frame cap).
const MAX_RECORD_LEN: u32 = 64 * 1024 * 1024;

fn fnv1a(bytes: &[u8]) -> u32 {
    let mut hash: u32 = 0x811c9dc5;
    for &b in bytes {
        hash ^= u32::from(b);
        hash = hash.wrapping_mul(0x01000193);
    }
    hash
}

/// Durable raft storage: an in-memory [`MemStorage`] runtime view backed by an
/// append-only, checksummed, torn-tail-tolerant log file.
pub struct DiskRaftStorage<F: FileSystem = OsFileSystem> {
    mem: MemStorage,
    /// None after any failed persistence/update operation. Recovery must repair
    /// the log before this writer can be used again. The guard also orders the
    /// durable record and its in-memory publication as one writer operation.
    file: Mutex<Option<F::File>>,
    path: PathBuf,
    /// Highest conf-change index recorded via `REC_CONF_STATE_AT` (0 = only
    /// the initial configuration exists). The replay guard boundary.
    conf_index: Mutex<u64>,
    lease_epoch: Mutex<Option<LeaseEpoch>>,
    io_metrics: Arc<WalIoMetrics>,
}

impl DiskRaftStorage<OsFileSystem> {
    /// Open (or create) the raft state under `data_dir`. Replays any surviving
    /// records into the runtime view; a fresh directory starts pristine with
    /// `voters` as the initial configuration (persisted immediately, so a
    /// restart before the first vote still knows its membership).
    ///
    /// Returns `(storage, was_pristine)` — `was_pristine = false` means this
    /// data-dir carries raft history: the caller must treat the node as a
    /// rejoining member (bootstrap fencing rule (c)), never re-initialize.
    pub fn open(data_dir: &Path, voters: &[u64]) -> Result<(DiskRaftStorage, bool)> {
        Self::open_on(OsFileSystem, data_dir, voters)
    }

    /// Recover an activated store. Never create a missing log or initialize an
    /// empty/torn log with a fresh voting configuration.
    pub fn recover(data_dir: &Path) -> Result<DiskRaftStorage> {
        Self::open_mode(OsFileSystem, data_dir, &[], false).map(|(storage, _)| storage)
    }
}

impl<F: FileSystem> DiskRaftStorage<F> {
    fn open_on(fs: F, data_dir: &Path, voters: &[u64]) -> Result<(Self, bool)> {
        Self::open_mode(fs, data_dir, voters, true)
    }

    fn open_mode(fs: F, data_dir: &Path, voters: &[u64], initialize: bool) -> Result<(Self, bool)> {
        let io_metrics = WalIoMetrics::shared();
        if initialize {
            fs::create_dirs(&fs, data_dir)
                .map_err(|e| Error::Raft(format!("create {}: {e}", data_dir.display())))?;
        }
        let path = data_dir.join("raft.log");
        let mut file = (if initialize {
            fs.open_append(&path)
        } else {
            fs.open_existing_append(&path)
        })
        .map_err(|e| Error::Raft(format!("open {}: {e}", path.display())))?;

        let mut bytes = Vec::new();
        file.seek(SeekFrom::Start(0))
            .and_then(|_| file.read_to_end(&mut bytes))
            .map_err(|e| Error::Raft(format!("read {}: {e}", path.display())))?;

        let mem = MemStorage::new();
        let mut valid_len: u64 = 0;
        let mut saw_any = false;
        let mut conf_idx: u64 = 0;
        let mut lease_epoch: Option<LeaseEpoch> = None;
        let mut cursor: usize = 0;
        // The loop ends where records stop parsing — a torn tail
        // (short/checksum-fail) or the clean end of file; either way the valid
        // prefix is kept and the tail truncated below.
        while let Some((kind, payload, next)) = next_record(&bytes, cursor) {
            // From here the record is checksum-valid: decode failures are real
            // inconsistencies, not crash artifacts — refuse to open.
            match kind {
                REC_LEASE_EPOCH => {
                    let next = LeaseEpoch::decode(payload)?;
                    if LeaseEpoch::successor(lease_epoch.as_ref(), &next.policy)? != next {
                        return Err(Error::Raft(
                            "non-sequential durable lease incarnation".into(),
                        ));
                    }
                    lease_epoch = Some(next);
                }
                REC_CONF_STATE => {
                    let cs = ConfState::parse_from_bytes(payload).map_err(|e| {
                        Error::Raft(format!("checksum-valid ConfState undecodable: {e}"))
                    })?;
                    mem.wl().set_conf_state(cs);
                }
                REC_CONF_STATE_AT => {
                    if payload.len() < 8 {
                        return Err(Error::Raft(
                            "checksum-valid ConfStateAt record shorter than its index".into(),
                        ));
                    }
                    let idx = u64::from_be_bytes(payload[..8].try_into().expect("8 bytes"));
                    let cs = ConfState::parse_from_bytes(&payload[8..]).map_err(|e| {
                        Error::Raft(format!("checksum-valid ConfStateAt undecodable: {e}"))
                    })?;
                    mem.wl().set_conf_state(cs);
                    conf_idx = idx; // last write wins, in file order
                }
                REC_HARD_STATE => {
                    let hs = HardState::parse_from_bytes(payload).map_err(|e| {
                        Error::Raft(format!("checksum-valid HardState undecodable: {e}"))
                    })?;
                    mem.wl().set_hardstate(hs);
                }
                REC_ENTRY => {
                    let e = Entry::parse_from_bytes(payload).map_err(|err| {
                        Error::Raft(format!("checksum-valid Entry undecodable: {err}"))
                    })?;
                    // Re-appended index overwrites the suffix (raft truncation).
                    mem.wl()
                        .append(&[e])
                        .map_err(|err| Error::Raft(format!("replay append: {err}")))?;
                }
                other => {
                    return Err(Error::Raft(format!(
                        "unknown raft-log record kind {other} (newer format?) — refusing to guess"
                    )))
                }
            }
            saw_any = true;
            cursor = next;
            valid_len = cursor as u64;
        }
        if !initialize && !saw_any {
            return Err(Error::Raft(
                "activated store has no recoverable Raft log; refusing reinitialization".into(),
            ));
        }
        // Drop the torn/corrupt tail so future appends start at a clean point.
        if valid_len < bytes.len() as u64 {
            file.set_len(valid_len)
                .map_err(|e| Error::Raft(format!("truncate torn tail: {e}")))?;
            file.seek(SeekFrom::End(0))
                .map_err(|e| Error::Raft(format!("seek: {e}")))?;
        }
        // A process restart can expose valid bytes that were never synchronized
        // by the previous process. Make the replayed prefix (and tail repair)
        // durable before it may justify a Raft response in this incarnation.
        io_metrics
            .recovery_sync
            .measure(|| file.sync_data())
            .map_err(|e| Error::Raft(format!("sync recovered raft log: {e}")))?;

        let storage = DiskRaftStorage {
            mem,
            file: Mutex::new(Some(file)),
            path,
            conf_index: Mutex::new(conf_idx),
            lease_epoch: Mutex::new(lease_epoch),
            io_metrics,
        };
        let was_pristine = !saw_any;
        if was_pristine {
            let cs = ConfState::from((voters.to_vec(), vec![]));
            storage.with_writer(|file| {
                Self::write_record(
                    &storage.io_metrics,
                    file,
                    REC_CONF_STATE,
                    &cs.write_to_bytes()
                        .map_err(|e| Error::Raft(format!("confstate encode: {e}")))?,
                )?;
                storage.mem.wl().set_conf_state(cs);
                Ok(())
            })?;
        }
        // File sync alone cannot make the directory entry durable. Existing
        // directories also need publication after an earlier interrupted open.
        storage
            .io_metrics
            .namespace_publish
            .measure(|| fs::sync_ancestors(&fs, data_dir))
            .map_err(|e| Error::Raft(format!("publish raft log directory: {e}")))?;
        Ok((storage, was_pristine))
    }

    /// Recover/validate the exact term of an applied position from durable
    /// committed Raft history. Never guess a term while upgrading a v1 WAL.
    pub fn committed_term(&self, index: u64) -> Result<u64> {
        use raft::Storage;
        if index
            > self
                .mem
                .initial_state()
                .map_err(|e| Error::Raft(e.to_string()))?
                .hard_state
                .commit
        {
            return Err(Error::Raft(
                "engine applied position exceeds durable Raft commit".into(),
            ));
        }
        self.mem
            .term(index)
            .map_err(|e| Error::Raft(format!("applied position missing from Raft history: {e}")))
    }

    pub fn has_committed_checkpoint(&self, bytes: &[u8]) -> Result<bool> {
        use raft::Storage;
        let commit = self
            .mem
            .initial_state()
            .map_err(|e| Error::Raft(e.to_string()))?
            .hard_state
            .commit;
        if commit == 0 {
            return Ok(false);
        }
        let first = self
            .mem
            .first_index()
            .map_err(|e| Error::Raft(e.to_string()))?;
        for index in first..=commit {
            let entries = self
                .mem
                .entries(
                    index,
                    index + 1,
                    None,
                    raft::GetEntriesContext::empty(false),
                )
                .map_err(|e| Error::Raft(e.to_string()))?;
            for entry in entries {
                if entry.entry_type == raft::eraftpb::EntryType::EntryNormal
                    && !entry.data.is_empty()
                {
                    if let crate::Command::ManifestChange(payload) =
                        crate::Command::decode(&entry.data)?
                    {
                        if payload.changeset() == bytes {
                            return Ok(true);
                        }
                    }
                }
            }
        }
        Ok(false)
    }

    pub fn io_metrics(&self) -> Arc<WalIoMetrics> {
        self.io_metrics.clone()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn with_writer(&self, action: impl FnOnce(&mut F::File) -> Result<()>) -> Result<()> {
        let mut writer = self.file.lock().expect("raft log file poisoned");
        let file = writer.as_mut().ok_or_else(|| {
            Error::Raft(
                "raft log writer unavailable after persistence failure; reopen required".into(),
            )
        })?;
        let result = action(file);
        if result.is_err() {
            *writer = None;
        }
        result
    }

    fn write_record(
        metrics: &WalIoMetrics,
        file: &mut F::File,
        kind: u8,
        payload: &[u8],
    ) -> Result<()> {
        Self::write_record_unsynced(metrics, file, kind, payload)?;
        Self::sync_records(metrics, file)
    }

    fn write_entries_unsynced(
        metrics: &WalIoMetrics,
        file: &mut F::File,
        entries: &[Entry],
    ) -> Result<()> {
        for entry in entries {
            let bytes = entry
                .write_to_bytes()
                .map_err(|err| Error::Raft(format!("entry encode: {err}")))?;
            Self::write_record_unsynced(metrics, file, REC_ENTRY, &bytes)?;
        }
        Ok(())
    }

    /// The caller must synchronize before publishing the corresponding state.
    /// Keep the existing frame format and one-frame temporary allocation bound.
    fn write_record_unsynced(
        metrics: &WalIoMetrics,
        file: &mut F::File,
        kind: u8,
        payload: &[u8],
    ) -> Result<()> {
        if payload.len() >= MAX_RECORD_LEN as usize {
            return Err(Error::Raft(
                "raft log record exceeds the replay format limit".into(),
            ));
        }
        let mut body = Vec::with_capacity(1 + payload.len());
        body.push(kind);
        body.extend_from_slice(payload);
        let mut rec = Vec::with_capacity(8 + body.len());
        rec.extend_from_slice(&(body.len() as u32).to_be_bytes());
        rec.extend_from_slice(&fnv1a(&body).to_be_bytes());
        rec.extend_from_slice(&body);
        metrics
            .write
            .measure(|| file.write_all(&rec))
            .map_err(|e| Error::Raft(format!("raft log append: {e}")))
    }

    fn sync_records(metrics: &WalIoMetrics, file: &mut F::File) -> Result<()> {
        let result = metrics.sync.measure(|| file.sync_data());
        result.map_err(|e| Error::Raft(format!("raft log append: {e}")))
    }
}

/// Parse the record at `at`; `None` on clean end, torn tail, or bad checksum.
fn next_record(bytes: &[u8], at: usize) -> Option<(u8, &[u8], usize)> {
    let header = bytes.get(at..at + 8)?;
    let len = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
    if len == 0 || len > MAX_RECORD_LEN {
        return None;
    }
    let sum = u32::from_be_bytes([header[4], header[5], header[6], header[7]]);
    let body = bytes.get(at + 8..at + 8 + len as usize)?;
    if fnv1a(body) != sum {
        return None;
    }
    Some((body[0], &body[1..], at + 8 + len as usize))
}

impl<F: FileSystem> raft::Storage for DiskRaftStorage<F> {
    fn initial_state(&self) -> raft::Result<RaftState> {
        self.mem.initial_state()
    }

    fn entries(
        &self,
        low: u64,
        high: u64,
        max_size: impl Into<Option<u64>>,
        context: GetEntriesContext,
    ) -> raft::Result<Vec<Entry>> {
        self.mem.entries(low, high, max_size, context)
    }

    fn term(&self, idx: u64) -> raft::Result<u64> {
        self.mem.term(idx)
    }

    fn first_index(&self) -> raft::Result<u64> {
        self.mem.first_index()
    }

    fn last_index(&self) -> raft::Result<u64> {
        self.mem.last_index()
    }

    fn snapshot(&self, request_index: u64, to: u64) -> raft::Result<raft::prelude::Snapshot> {
        self.mem.snapshot(request_index, to)
    }
}

impl<F: FileSystem> PersistentRaftStorage for DiskRaftStorage<F> {
    fn recovered_lease_epoch(&self) -> Option<LeaseEpoch> {
        self.lease_epoch
            .lock()
            .expect("lease epoch poisoned")
            .clone()
    }

    fn begin_lease_incarnation(&self, policy: &LeasePolicy) -> Result<LeaseEpoch> {
        let mut published = None;
        self.with_writer(|file| {
            let mut current = self.lease_epoch.lock().expect("lease epoch poisoned");
            let next = LeaseEpoch::successor(current.as_ref(), policy)?;
            Self::write_record(&self.io_metrics, file, REC_LEASE_EPOCH, &next.encode())?;
            *current = Some(next.clone());
            published = Some(next);
            Ok(())
        })?;
        Ok(published.expect("successful writer publishes epoch"))
    }

    /// One synchronization covers the complete append slice before the runtime
    /// view exposes any member. The Ready loop sends no message until return.
    fn append(&self, entries: &[Entry]) -> Result<()> {
        self.with_writer(|file| {
            Self::write_entries_unsynced(&self.io_metrics, file, entries)?;
            if !entries.is_empty() {
                Self::sync_records(&self.io_metrics, file)?;
            }
            self.mem
                .wl()
                .append(entries)
                .map_err(|e| Error::Raft(e.to_string()))
        })
    }

    fn persist_ready(&self, entries: &[Entry], hs: Option<&HardState>) -> Result<()> {
        let mut operation = "ready writer";
        let result = self.with_writer(|file| {
            if entries.is_empty() && hs.is_none() {
                return Ok(());
            }
            operation = "append";
            Self::write_entries_unsynced(&self.io_metrics, file, entries)?;
            if let Some(hs) = hs {
                operation = "hardstate";
                let bytes = hs
                    .write_to_bytes()
                    .map_err(|err| Error::Raft(format!("hardstate encode: {err}")))?;
                Self::write_record_unsynced(&self.io_metrics, file, REC_HARD_STATE, &bytes)?;
            }
            if !entries.is_empty() || hs.is_some() {
                operation = "ready sync";
                Self::sync_records(&self.io_metrics, file)?;
            }
            // One memory write lock publishes the complete successful Ready.
            // No entry or vote/commit update is exposed before the common sync.
            operation = "ready memory publication";
            let mut memory = self.mem.wl();
            if !entries.is_empty() {
                memory
                    .append(entries)
                    .map_err(|err| Error::Raft(err.to_string()))?;
            }
            if let Some(hs) = hs {
                memory.set_hardstate(hs.clone());
            }
            Ok(())
        });
        result.map_err(|cause| Error::Raft(format!("during {operation}: {cause}")))
    }

    fn set_hardstate(&self, hs: &HardState) -> Result<()> {
        let bytes = hs
            .write_to_bytes()
            .map_err(|e| Error::Raft(format!("hardstate encode: {e}")))?;
        self.with_writer(|file| {
            Self::write_record(&self.io_metrics, file, REC_HARD_STATE, &bytes)?;
            self.mem.wl().set_hardstate(hs.clone());
            Ok(())
        })
    }

    /// A post-conf-change membership record (task #24). Replay is last-write-
    /// wins for `REC_CONF_STATE`, so the newest committed membership survives
    /// restart; omitting this write would resurrect the pre-change voter set.
    fn set_conf_state(&self, cs: &ConfState, at_index: u64) -> Result<()> {
        let pb = cs
            .write_to_bytes()
            .map_err(|e| Error::Raft(format!("confstate encode: {e}")))?;
        let mut bytes = Vec::with_capacity(8 + pb.len());
        bytes.extend_from_slice(&at_index.to_be_bytes());
        bytes.extend_from_slice(&pb);
        self.with_writer(|file| {
            Self::write_record(&self.io_metrics, file, REC_CONF_STATE_AT, &bytes)?;
            self.mem.wl().set_conf_state(cs.clone());
            *self.conf_index.lock().expect("conf index poisoned") = at_index;
            Ok(())
        })
    }

    fn recovered_conf_index(&self) -> u64 {
        *self.conf_index.lock().expect("conf index poisoned")
    }
}

#[cfg(test)]
mod persistence_model;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::OpenOptions;

    fn tmp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kv9-raftlog-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn entry(index: u64, term: u64, data: &[u8]) -> Entry {
        Entry {
            index,
            term,
            data: data.to_vec().into(),
            ..Default::default()
        }
    }

    #[test]
    fn light_ready_commit_survives_immediate_reopen() {
        use crate::rawnode::RaftPeer;
        use crate::RaftGroup;
        use kv9_common::{NodeId, RegionId};

        let dir = tmp();
        let at;
        {
            let (storage, _) = DiskRaftStorage::open(&dir, &[1]).unwrap();
            let peer = RaftPeer::with_storage(NodeId(1), RegionId(1), storage).unwrap();
            peer.campaign().unwrap();
            peer.pump().unwrap();
            at = peer.propose_traced(b"committed".to_vec()).unwrap();
            peer.pump().unwrap();
            assert_eq!(peer.raft_committed(), at.index);
        }
        let (storage, _) = DiskRaftStorage::open(&dir, &[1]).unwrap();
        let hard_state = raft::Storage::initial_state(&storage).unwrap().hard_state;
        assert_eq!(hard_state.term, at.term);
        assert_eq!(hard_state.vote, 1);
        assert_eq!(storage.committed_term(at.index.0).unwrap(), at.term);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// HardState + entries survive a reopen — the double-vote scenario is dead:
    /// the reopened storage still knows the term and vote.
    #[test]
    fn hardstate_and_log_survive_reopen() {
        let dir = tmp();
        {
            let (s, pristine) = DiskRaftStorage::open(&dir, &[1, 2, 3]).unwrap();
            assert!(pristine);
            let hs = HardState {
                term: 7,
                vote: 2,
                commit: 2,
                ..Default::default()
            };
            s.set_hardstate(&hs).unwrap();
            s.append(&[entry(1, 7, b"a"), entry(2, 7, b"b")]).unwrap();
        }
        let (s, pristine) = DiskRaftStorage::open(&dir, &[1, 2, 3]).unwrap();
        assert!(
            !pristine,
            "surviving state must be detected (fencing rule c)"
        );
        let state = raft::Storage::initial_state(&s).unwrap();
        assert_eq!(state.hard_state.term, 7);
        assert_eq!(state.hard_state.vote, 2);
        assert_eq!(raft::Storage::last_index(&s).unwrap(), 2);
        assert_eq!(raft::Storage::term(&s, 2).unwrap(), 7);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn activated_store_recovery_never_creates_or_reinitializes_a_log() {
        let dir = tmp();
        assert!(DiskRaftStorage::recover(&dir).is_err());
        assert!(!dir.exists(), "recovery created a missing store directory");
        let (storage, pristine) = DiskRaftStorage::open(&dir, &[1, 2, 3]).unwrap();
        assert!(pristine);
        storage.append(&[entry(1, 7, b"committed")]).unwrap();
        storage
            .set_hardstate(&HardState {
                term: 7,
                vote: 2,
                commit: 1,
                ..Default::default()
            })
            .unwrap();
        drop(storage);
        let recovered = DiskRaftStorage::recover(&dir).unwrap();
        assert_eq!(
            raft::Storage::initial_state(&recovered)
                .unwrap()
                .hard_state
                .vote,
            2
        );
        assert_eq!(raft::Storage::last_index(&recovered).unwrap(), 1);
        drop(recovered);
        let file = dir.join("raft.log");
        let original = std::fs::read(&file).unwrap();
        std::fs::remove_file(&file).unwrap();
        assert!(DiskRaftStorage::recover(&dir).is_err());
        assert!(!file.exists(), "recovery recreated a missing log");
        for invalid in [Vec::new(), original[..3].to_vec()] {
            std::fs::write(&file, &invalid).unwrap();
            for _ in 0..2 {
                assert!(
                    DiskRaftStorage::recover(&dir).is_err(),
                    "activated store reinitialized an empty or torn log"
                );
                assert_eq!(
                    std::fs::read(&file).unwrap(),
                    invalid,
                    "refusal modified the corrupt log"
                );
            }
        }
        std::fs::write(&file, original).unwrap();
        assert_eq!(
            raft::Storage::last_index(&DiskRaftStorage::recover(&dir).unwrap()).unwrap(),
            1
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A torn tail (half-written record) is tolerated: everything before it
    /// survives, the tail is truncated, and appends continue cleanly.
    #[test]
    fn torn_tail_is_tolerated_and_truncated() {
        let dir = tmp();
        {
            let (s, _) = DiskRaftStorage::open(&dir, &[1]).unwrap();
            s.append(&[entry(1, 1, b"keep")]).unwrap();
        }
        // Simulate a crash mid-write: append garbage half-record.
        let path = dir.join("raft.log");
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(&[0x00, 0x00, 0x00, 0x10, 0xAA]).unwrap(); // len says 16, then EOF
        drop(f);

        let (s, pristine) = DiskRaftStorage::open(&dir, &[1]).unwrap();
        assert!(!pristine);
        assert_eq!(raft::Storage::last_index(&s).unwrap(), 1);
        // The torn bytes are gone; a new append lands and survives another reopen.
        s.append(&[entry(2, 1, b"after")]).unwrap();
        drop(s);
        let (s, _) = DiskRaftStorage::open(&dir, &[1]).unwrap();
        assert_eq!(raft::Storage::last_index(&s).unwrap(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A checksum-VALID record with an unknown kind is a real inconsistency,
    /// not a torn tail: open must refuse, never silently drop (engine-WAL
    /// aligned contract).
    #[test]
    fn checksum_valid_unknown_kind_refuses_to_open() {
        let dir = tmp();
        {
            let (s, _) = DiskRaftStorage::open(&dir, &[1]).unwrap();
            s.append(&[entry(1, 1, b"ok")]).unwrap();
        }
        // Append a well-formed record with an unknown kind byte.
        let body = vec![99u8, 1, 2, 3];
        let mut rec = Vec::new();
        rec.extend_from_slice(&(body.len() as u32).to_be_bytes());
        rec.extend_from_slice(&fnv1a(&body).to_be_bytes());
        rec.extend_from_slice(&body);
        let path = dir.join("raft.log");
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(&rec).unwrap();
        drop(f);
        assert!(DiskRaftStorage::open(&dir, &[1]).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Sensitivity control for the torn-tail tolerance: a corrupted record in
    /// the MIDDLE must also stop replay (checksum catches it) — but then the
    /// suffix after it is dropped too. Corruption never passes silently.
    #[test]
    fn control_corrupt_middle_record_stops_replay() {
        let dir = tmp();
        {
            let (s, _) = DiskRaftStorage::open(&dir, &[1]).unwrap();
            s.append(&[entry(1, 1, b"one"), entry(2, 1, b"two")])
                .unwrap();
        }
        let path = dir.join("raft.log");
        let mut bytes = std::fs::read(&path).unwrap();
        // Flip a byte inside the FIRST entry record's payload (skip the
        // ConfState record: locate second record by walking one record).
        let first_len = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        let second_start = 8 + first_len;
        let target = second_start + 8 + 3; // inside second record's body
        bytes[target] ^= 0xFF;
        std::fs::write(&path, &bytes).unwrap();

        let (s, _) = DiskRaftStorage::open(&dir, &[1]).unwrap();
        // Replay stopped at the corrupt record: no entries survived it.
        assert_eq!(raft::Storage::last_index(&s).unwrap(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A raft peer over disk storage: propose/commit on one incarnation, then
    /// reopen — the log and term are still there (restart safety, library level).
    #[test]
    fn raft_peer_state_survives_restart() {
        use crate::rawnode::RaftPeer;
        use crate::RaftGroup;
        use kv9_common::{NodeId, RegionId};

        let dir = tmp();
        let committed_index;
        {
            let (storage, pristine) = DiskRaftStorage::open(&dir, &[1]).unwrap();
            assert!(pristine);
            let peer = RaftPeer::with_storage(NodeId(1), RegionId(1), storage).unwrap();
            peer.campaign().unwrap();
            // Single-node: pump until leader, then propose.
            for _ in 0..50 {
                peer.tick_once();
                peer.pump().unwrap();
                if peer.role() == crate::Role::Leader {
                    break;
                }
            }
            let at = peer.propose_traced(b"durable".to_vec()).unwrap();
            for _ in 0..50 {
                peer.tick_once();
                peer.pump().unwrap();
                if peer.raft_committed() >= at.index {
                    break;
                }
            }
            committed_index = at.index;
            assert!(peer.raft_committed() >= committed_index);
        }
        // "Restart": reopen the same data-dir.
        let (storage, pristine) = DiskRaftStorage::open(&dir, &[1]).unwrap();
        assert!(!pristine, "restarted node must see its history");
        assert!(raft::Storage::last_index(&storage).unwrap() >= committed_index.0);
        let state = raft::Storage::initial_state(&storage).unwrap();
        assert!(state.hard_state.term >= 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
