//! Store-bound, finite local recovery of actual drained-scope observations.
//! A saved report is replayed verbatim; absence never reconstructs one. Neither
//! a report nor local process exit proves that unknown remote PUTs terminated.
use super::*;
use kv9_common::retention::OwnerToken;
use kv9_common::StoreIdentity;
use kv9_meta::retention::decode_request;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const SLOTS: usize = 1024;
const MAX_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy)]
enum Slot {
    Vacant,
    Reserved(OwnerToken, u64),
    Recover(OwnerToken),
    Ready(OwnerToken, RootDigest),
    Acknowledged(OwnerToken, RootDigest),
}
struct State {
    slots: Vec<Slot>,
    after: usize,
    serial: u64,
}
pub(crate) struct DrainOutbox {
    directory: PathBuf,
    identity: StoreIdentity,
    state: Mutex<State>,
}
#[derive(serde::Serialize)]
pub(crate) struct Snapshot {
    slots_limit: usize,
    record_bytes_limit: usize,
    reserved: usize,
    ready: usize,
    recovery: usize,
    acknowledged: usize,
}
pub(crate) struct ReportSlot {
    outbox: Arc<DrainOutbox>,
    index: usize,
    executor: OwnerToken,
    serial: u64,
    persisted: Mutex<Option<RootDigest>>,
}
fn error(message: impl std::fmt::Display) -> Error {
    Error::Engine(format!("checkpoint drain outbox: {message}"))
}
fn file(path: &Path) -> Result<Option<Vec<u8>>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(error(e)),
    };
    if !metadata.is_file() || metadata.len() > MAX_BYTES as u64 {
        return Err(error("record type or size differs"));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(path)
        .map_err(error)?
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    if bytes.len() > MAX_BYTES {
        return Err(error("record grew beyond bound"));
    }
    Ok(Some(bytes))
}
fn remove(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(error(e)),
    }
}
fn complete(bytes: &[u8]) -> bool {
    bytes
        .len()
        .checked_sub(32)
        .is_some_and(|end| RootDigest::sha256(&bytes[..end]).as_bytes().as_slice() == &bytes[end..])
}
impl DrainOutbox {
    pub(crate) fn open(store: &Path, identity: StoreIdentity) -> Result<Arc<Self>> {
        let directory = store.join("checkpoint-drains");
        fs::create_dir_all(&directory).map_err(error)?;
        if !fs::symlink_metadata(&directory).map_err(error)?.is_dir() {
            return Err(error("directory is not a real directory"));
        }
        File::open(store)
            .and_then(|f| f.sync_all())
            .map_err(error)?;
        let outbox = Arc::new(Self {
            directory,
            identity,
            state: Mutex::new(State {
                slots: vec![Slot::Vacant; SLOTS],
                after: SLOTS - 1,
                serial: 0,
            }),
        });
        // Fixed names cap inventory and future files independently of groups,
        // attempts and historical generations. Unknown entries are never erased.
        let mut occupied = std::collections::BTreeSet::new();
        let mut entries = 0;
        for entry in fs::read_dir(&outbox.directory).map_err(error)? {
            entries += 1;
            if entries > SLOTS * 2 {
                return Err(error("file count exceeds bound"));
            }
            let entry = entry.map_err(error)?;
            let name = entry.file_name();
            let name = name.to_str().ok_or_else(|| error("non-Unicode filename"))?;
            let (number, extension) = name
                .split_once('.')
                .ok_or_else(|| error("unknown filename"))?;
            let index: usize = number.parse().map_err(|_| error("invalid slot number"))?;
            if index >= SLOTS
                || number != format!("{index:04}")
                || !matches!(extension, "drain" | "tmp")
            {
                return Err(error("unknown slot filename"));
            }
            occupied.insert(index);
        }
        let mut state = outbox.state.lock().expect("drain outbox");
        let mut owners = std::collections::BTreeSet::new();
        for index in occupied {
            let slot = outbox.recover(index)?;
            if let Slot::Ready(token, _) = slot {
                if !owners.insert(token.owner) {
                    return Err(error("duplicate executor reports"));
                }
            }
            state.slots[index] = slot;
        }
        drop(state);
        Ok(outbox)
    }
    fn path(&self, index: usize, temporary: bool) -> PathBuf {
        self.directory.join(format!(
            "{index:04}.{}",
            if temporary { "tmp" } else { "drain" }
        ))
    }
    fn sync(&self) -> Result<()> {
        File::open(&self.directory)
            .and_then(|f| f.sync_all())
            .map_err(error)
    }
    fn validate(&self, bytes: &[u8]) -> Result<OwnerToken> {
        if bytes.len() > MAX_BYTES {
            return Err(error("record exceeds byte bound"));
        }
        let request = decode_request(bytes, self.identity.root_digest)?;
        let (executor, source_generation, kind) = match &request {
            LedgerRequest::FinishMigrationPublisher {
                operation,
                publisher,
                issued,
                resolved,
            } => {
                if operation == &[0; 16] || resolved > issued {
                    return Err(error("migration publisher report bounds"));
                }
                (publisher, 1, OwnerKind::Publisher)
            }
            LedgerRequest::FinishCheckpointExecution {
                pending,
                executor,
                issued,
                resolved,
            } => {
                if resolved > issued {
                    return Err(error("resolved exceeds issued"));
                }
                (executor, pending.generation.get(), OwnerKind::Build)
            }
            LedgerRequest::FinishCheckpointRead { successor, reader }
            | LedgerRequest::FinishImageRead { successor, reader } => {
                (reader, successor.generation.get(), OwnerKind::Reader)
            }
            _ => return Err(error("record is not a drained execution or read")),
        };
        let local = executor
            .descriptor
            .local
            .ok_or_else(|| error("executor lifetime missing"))?;
        if executor.descriptor.kind != kind
            || local.node != self.identity.node_id.0
            || local.store != *self.identity.store_incarnation.as_bytes()
            || source_generation != 1
            || encode_request(self.identity.root_digest, &request)? != bytes
        {
            return Err(error("record store, binding or encoding differs"));
        }
        Ok(executor.token())
    }
    fn recover(&self, index: usize) -> Result<Slot> {
        let path = self.path(index, false);
        let temporary = self.path(index, true);
        if let Some(bytes) = file(&path)? {
            let executor = self.validate(&bytes)?;
            if let Some(temp) = file(&temporary)? {
                if temp != bytes {
                    return Err(error("published and temporary records differ"));
                }
                remove(&temporary)?;
            }
            File::open(&path)
                .and_then(|f| f.sync_all())
                .map_err(error)?;
            self.sync()?;
            return Ok(Slot::Ready(executor, RootDigest::sha256(&bytes)));
        }
        if let Some(bytes) = file(&temporary)? {
            // An incomplete temporary write never reached metadata submission.
            // It supplies no drain evidence; its original global pin remains.
            // A complete record is fsynced and promoted before replay.
            match self.validate(&bytes) {
                Ok(executor) => {
                    File::open(&temporary)
                        .and_then(|f| f.sync_all())
                        .map_err(error)?;
                    fs::rename(&temporary, &path).map_err(error)?;
                    self.sync()?;
                    return Ok(Slot::Ready(executor, RootDigest::sha256(&bytes)));
                }
                Err(e) if complete(&bytes) => return Err(e),
                Err(_) => remove(&temporary)?,
            }
        }
        self.sync()?;
        Ok(Slot::Vacant)
    }
    pub(crate) fn reserve(self: &Arc<Self>, executor: OwnerToken) -> Result<ReportSlot> {
        let mut state = self.state.lock().expect("drain outbox");
        if state.slots.iter().any(|slot| matches!(slot, Slot::Reserved(t, _) | Slot::Recover(t) | Slot::Ready(t, _) | Slot::Acknowledged(t, _) if t.owner == executor.owner)) {
            return Err(error("executor still has a report reservation or unacknowledged report"));
        }
        let index = state
            .slots
            .iter()
            .position(|s| matches!(s, Slot::Vacant))
            .ok_or_else(|| error("report slots exhausted"))?;
        state.serial = state
            .serial
            .checked_add(1)
            .ok_or_else(|| error("reservation serial exhausted"))?;
        let serial = state.serial;
        state.slots[index] = Slot::Reserved(executor, serial);
        Ok(ReportSlot {
            outbox: self.clone(),
            index,
            executor,
            serial,
            persisted: Mutex::new(None),
        })
    }
    pub(crate) fn snapshot(&self) -> Snapshot {
        let state = self.state.lock().expect("drain outbox");
        Snapshot {
            slots_limit: SLOTS,
            record_bytes_limit: MAX_BYTES,
            reserved: state
                .slots
                .iter()
                .filter(|s| matches!(s, Slot::Reserved(..)))
                .count(),
            ready: state
                .slots
                .iter()
                .filter(|s| matches!(s, Slot::Ready(..)))
                .count(),
            recovery: state
                .slots
                .iter()
                .filter(|s| matches!(s, Slot::Recover(_)))
                .count(),
            acknowledged: state
                .slots
                .iter()
                .filter(|s| matches!(s, Slot::Acknowledged(..)))
                .count(),
        }
    }
    pub(crate) fn has_pending(&self) -> bool {
        self.state
            .lock()
            .expect("drain outbox")
            .slots
            .iter()
            .any(|s| {
                matches!(
                    s,
                    Slot::Recover(_) | Slot::Ready(..) | Slot::Acknowledged(..)
                )
            })
    }
    fn acknowledge(&self, index: usize, executor: OwnerToken, digest: RootDigest) -> Result<()> {
        let mut state = self.state.lock().expect("drain outbox");
        // Another exact replay may have acknowledged and reused the slot.
        let (Slot::Ready(current, current_digest) | Slot::Acknowledged(current, current_digest)) =
            state.slots[index]
        else {
            return Ok(());
        };
        if current != executor {
            return Ok(());
        }
        if current_digest != digest {
            return Err(error("acknowledgement digest differs"));
        }
        state.slots[index] = Slot::Acknowledged(executor, digest);
        remove(&self.path(index, false))?;
        self.sync()?;
        state.slots[index] = Slot::Vacant;
        Ok(())
    }
    /// One slot per maintenance pass. The filesystem mutex is not held over
    /// consensus/RPC waits; duplicate replay of identical bytes is idempotent.
    pub(crate) fn replay_one(&self, ledger: &impl CheckpointLedger) -> Result<bool> {
        let (index, executor, digest) = {
            let mut state = self.state.lock().expect("drain outbox");
            let Some(index) = (1..=SLOTS).map(|n| (state.after + n) % SLOTS).find(|i| {
                matches!(
                    state.slots[*i],
                    Slot::Recover(_) | Slot::Ready(..) | Slot::Acknowledged(..)
                )
            }) else {
                return Ok(false);
            };
            state.after = index;
            if let Slot::Acknowledged(executor, digest) = state.slots[index] {
                drop(state);
                self.acknowledge(index, executor, digest)?;
                return Ok(true);
            }
            if matches!(state.slots[index], Slot::Recover(_)) {
                state.slots[index] = self.recover(index)?;
            }
            let Slot::Ready(executor, digest) = state.slots[index] else {
                return Ok(false);
            };
            (index, executor, digest)
        };
        let bytes =
            file(&self.path(index, false))?.ok_or_else(|| error("saved report disappeared"))?;
        if self.validate(&bytes)? != executor || RootDigest::sha256(&bytes) != digest {
            return Err(error("saved report changed"));
        }
        ledger.apply(bytes)?;
        self.acknowledge(index, executor, digest)?;
        Ok(true)
    }
}
impl ReportSlot {
    pub(crate) fn release_unused(&self) -> Result<()> {
        let mut state = self.outbox.state.lock().expect("drain outbox");
        if matches!(state.slots[self.index], Slot::Reserved(t, serial) if t == self.executor && serial == self.serial)
        {
            if file(&self.outbox.path(self.index, false))?.is_some()
                || file(&self.outbox.path(self.index, true))?.is_some()
            {
                return Err(error("unused reservation contains a report"));
            }
            state.slots[self.index] = Slot::Vacant;
        }
        Ok(())
    }
    pub(crate) fn persist(&self, bytes: &[u8]) -> Result<()> {
        if self.outbox.validate(bytes)? != self.executor {
            return Err(error("reserved executor differs"));
        }
        let digest = RootDigest::sha256(bytes);
        let mut persisted = self.persisted.lock().expect("drain report receipt");
        if let Some(old) = *persisted {
            return if old == digest {
                Ok(())
            } else {
                Err(error("saved report is immutable"))
            };
        }
        let mut state = self.outbox.state.lock().expect("drain outbox");
        if !matches!(state.slots[self.index], Slot::Reserved(t, serial) if t == self.executor && serial == self.serial)
        {
            return Err(error("report reservation lost"));
        }
        let path = self.outbox.path(self.index, false);
        let temporary = self.outbox.path(self.index, true);
        if let Some(old) = file(&path)? {
            if old != bytes {
                return Err(error("saved report is immutable"));
            }
            File::open(&path)
                .and_then(|f| f.sync_all())
                .map_err(error)?;
        } else {
            let existing = file(&temporary)?;
            if existing.as_deref() != Some(bytes) {
                if existing.as_ref().is_some_and(|old| complete(old)) {
                    return Err(error("complete temporary report differs"));
                }
                remove(&temporary)?;
                let mut file = OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&temporary)
                    .map_err(error)?;
                file.write_all(bytes)
                    .and_then(|_| file.sync_all())
                    .map_err(error)?;
            } else {
                File::open(&temporary)
                    .and_then(|f| f.sync_all())
                    .map_err(error)?;
            }
            fs::rename(&temporary, &path).map_err(error)?;
        }
        self.outbox.sync()?;
        state.slots[self.index] = Slot::Ready(self.executor, digest);
        *persisted = Some(digest);
        Ok(())
    }
    pub(crate) fn acknowledge(&self, bytes: &[u8]) -> Result<()> {
        self.outbox
            .acknowledge(self.index, self.executor, RootDigest::sha256(bytes))
    }
}
impl Drop for ReportSlot {
    fn drop(&mut self) {
        let mut state = self.outbox.state.lock().expect("drain outbox");
        if matches!(state.slots[self.index], Slot::Reserved(t, serial) if t == self.executor && serial == self.serial)
        {
            state.slots[self.index] = Slot::Recover(self.executor);
        }
    }
}

#[cfg(test)]
mod tests;
