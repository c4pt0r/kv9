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

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use protobuf::Message as PbMessage;
use raft::prelude::{ConfState, Entry, HardState};
use raft::storage::MemStorage;
use raft::{GetEntriesContext, RaftState};

use kv9_common::{Error, Result};

use crate::rawnode::PersistentRaftStorage;

const REC_CONF_STATE: u8 = 1;
const REC_HARD_STATE: u8 = 2;
const REC_ENTRY: u8 = 3;

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
pub struct DiskRaftStorage {
    mem: MemStorage,
    file: Mutex<File>,
    path: PathBuf,
}

impl DiskRaftStorage {
    /// Open (or create) the raft state under `data_dir`. Replays any surviving
    /// records into the runtime view; a fresh directory starts pristine with
    /// `voters` as the initial configuration (persisted immediately, so a
    /// restart before the first vote still knows its membership).
    ///
    /// Returns `(storage, was_pristine)` — `was_pristine = false` means this
    /// data-dir carries raft history: the caller must treat the node as a
    /// rejoining member (bootstrap fencing rule (c)), never re-initialize.
    pub fn open(data_dir: &Path, voters: &[u64]) -> Result<(DiskRaftStorage, bool)> {
        std::fs::create_dir_all(data_dir)
            .map_err(|e| Error::Raft(format!("create {}: {e}", data_dir.display())))?;
        let path = data_dir.join("raft.log");
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(&path)
            .map_err(|e| Error::Raft(format!("open {}: {e}", path.display())))?;

        let mut bytes = Vec::new();
        file.seek(SeekFrom::Start(0))
            .and_then(|_| file.read_to_end(&mut bytes))
            .map_err(|e| Error::Raft(format!("read {}: {e}", path.display())))?;

        let mem = MemStorage::new();
        let mut valid_len: u64 = 0;
        let mut saw_any = false;
        let mut cursor: usize = 0;
        loop {
            let Some((kind, payload, next)) = next_record(&bytes, cursor) else {
                break; // torn tail (short/checksum-fail) or clean end: keep prefix
            };
            // From here the record is checksum-valid: decode failures are real
            // inconsistencies, not crash artifacts — refuse to open.
            match kind {
                REC_CONF_STATE => {
                    let cs = ConfState::parse_from_bytes(payload).map_err(|e| {
                        Error::Raft(format!("checksum-valid ConfState undecodable: {e}"))
                    })?;
                    mem.wl().set_conf_state(cs);
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
        // Drop the torn/corrupt tail so future appends start at a clean point.
        if valid_len < bytes.len() as u64 {
            file.set_len(valid_len)
                .map_err(|e| Error::Raft(format!("truncate torn tail: {e}")))?;
            file.seek(SeekFrom::End(0))
                .map_err(|e| Error::Raft(format!("seek: {e}")))?;
        }

        let storage = DiskRaftStorage {
            mem,
            file: Mutex::new(file),
            path,
        };
        let was_pristine = !saw_any;
        if was_pristine {
            let cs = ConfState::from((voters.to_vec(), vec![]));
            storage.mem.wl().set_conf_state(cs.clone());
            storage.write_record(
                REC_CONF_STATE,
                &cs.write_to_bytes()
                    .map_err(|e| Error::Raft(format!("confstate encode: {e}")))?,
            )?;
        }
        Ok((storage, was_pristine))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn write_record(&self, kind: u8, payload: &[u8]) -> Result<()> {
        let mut body = Vec::with_capacity(1 + payload.len());
        body.push(kind);
        body.extend_from_slice(payload);
        let mut rec = Vec::with_capacity(8 + body.len());
        rec.extend_from_slice(&(body.len() as u32).to_be_bytes());
        rec.extend_from_slice(&fnv1a(&body).to_be_bytes());
        rec.extend_from_slice(&body);
        let mut file = self.file.lock().expect("raft log file poisoned");
        file.write_all(&rec)
            .and_then(|_| file.sync_data())
            .map_err(|e| Error::Raft(format!("raft log append: {e}")))
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

impl raft::Storage for DiskRaftStorage {
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

impl PersistentRaftStorage for DiskRaftStorage {
    /// Entries hit disk (fsync'd) BEFORE the runtime view exposes them — the
    /// Ready loop sends no message until this returns.
    fn append(&self, entries: &[Entry]) -> Result<()> {
        for e in entries {
            let bytes = e
                .write_to_bytes()
                .map_err(|err| Error::Raft(format!("entry encode: {err}")))?;
            self.write_record(REC_ENTRY, &bytes)?;
        }
        self.mem.wl().append(entries).map_err(|e| Error::Raft(e.to_string()))
    }

    fn set_hardstate(&self, hs: &HardState) -> Result<()> {
        let bytes = hs
            .write_to_bytes()
            .map_err(|e| Error::Raft(format!("hardstate encode: {e}")))?;
        self.write_record(REC_HARD_STATE, &bytes)?;
        self.mem.wl().set_hardstate(hs.clone());
        Ok(())
    }

    /// A post-conf-change membership record (task #24). Replay is last-write-
    /// wins for `REC_CONF_STATE`, so the newest committed membership survives
    /// restart; omitting this write would resurrect the pre-change voter set.
    fn set_conf_state(&self, cs: &ConfState) -> Result<()> {
        let bytes = cs
            .write_to_bytes()
            .map_err(|e| Error::Raft(format!("confstate encode: {e}")))?;
        self.write_record(REC_CONF_STATE, &bytes)?;
        self.mem.wl().set_conf_state(cs.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(!pristine, "surviving state must be detected (fencing rule c)");
        let state = raft::Storage::initial_state(&s).unwrap();
        assert_eq!(state.hard_state.term, 7);
        assert_eq!(state.hard_state.vote, 2);
        assert_eq!(raft::Storage::last_index(&s).unwrap(), 2);
        assert_eq!(raft::Storage::term(&s, 2).unwrap(), 7);
        let _ = std::fs::remove_dir_all(&dir);
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
            s.append(&[entry(1, 1, b"one"), entry(2, 1, b"two")]).unwrap();
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
                let _ = peer.pump();
                if peer.role() == crate::Role::Leader {
                    break;
                }
            }
            let at = peer.propose_traced(b"durable".to_vec()).unwrap();
            for _ in 0..50 {
                peer.tick_once();
                let _ = peer.pump();
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
