//! Local pending/publication-file admission. Named debt survives journal drop;
//! unlinked files remain charged until their actual readers/writers close.
//! This accounting grants no checkpoint, retention or deletion authority.
use kv9_common::{Error, Result};
use std::collections::BTreeMap;
use std::fs::{self, File, Metadata};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub const DEFAULT_PENDING_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone, serde::Serialize)]
pub struct Snapshot {
    pub limit_bytes: u64,
    pub retained_bytes: u64,
    pub peak_bytes: u64,
    pub named_paths: usize,
    pub open_readers: u64,
    pub open_writers: u64,
    pub refused_publications: u64,
    pub prepared_updates_deferred: u64,
}
#[derive(Debug)]
pub struct PendingDiskBudget {
    state: Mutex<State>,
}
type Identity = (u64, u64);
#[derive(Debug)]
struct Charge {
    bytes: u64,
    identity: Option<Identity>,
    links: usize,
    readers: u64,
    writers: u64,
}
#[derive(Debug)]
struct State {
    counters: Snapshot,
    serial: u64,
    files: BTreeMap<u64, Charge>,
    identities: BTreeMap<Identity, u64>,
    paths: BTreeMap<PathBuf, u64>,
}
impl PendingDiskBudget {
    pub fn new(limit_bytes: u64) -> Result<Self> {
        if limit_bytes == 0 {
            return Err(Error::Config("pending disk limit must be positive".into()));
        }
        Ok(Self {
            state: Mutex::new(State {
                counters: Snapshot {
                    limit_bytes,
                    retained_bytes: 0,
                    peak_bytes: 0,
                    named_paths: 0,
                    open_readers: 0,
                    open_writers: 0,
                    refused_publications: 0,
                    prepared_updates_deferred: 0,
                },
                serial: 0,
                files: BTreeMap::new(),
                identities: BTreeMap::new(),
                paths: BTreeMap::new(),
            }),
        })
    }
    pub fn snapshot(&self) -> Snapshot {
        let state = self.state.lock().expect("pending disk budget");
        let mut out = state.counters.clone();
        out.named_paths = state.paths.len();
        out
    }
    /// Startup inventory is debt observation, not admission. Existing bytes may
    /// exceed a newly lowered limit; recovery/settlement must still be possible.
    pub fn inventory_journal(&self, path: &Path) -> Result<()> {
        for path in [path.to_path_buf(), path.with_extension("pending.tmp")] {
            match fs::metadata(&path) {
                Ok(metadata) if metadata.is_file() => {
                    self.state
                        .lock()
                        .expect("pending disk budget")
                        .observe(&path, &metadata)?;
                }
                // Wrong-type entries cannot be journal payloads or acquire a
                // publication writer. Their owning group still fails validation;
                // inventory must not promote that quarantine to a node-wide one.
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(io(error)),
            }
        }
        Ok(())
    }
    pub(crate) fn read(self: &Arc<Self>, path: &Path, file: File) -> Result<TrackedFile> {
        let metadata = file.metadata().map_err(io)?;
        let id = {
            let mut state = self.state.lock().expect("pending disk budget");
            let id = state.observe(path, &metadata)?;
            state.files.get_mut(&id).unwrap().readers += 1;
            state.counters.open_readers += 1;
            id
        };
        Ok(TrackedFile {
            file,
            _lease: Some(Lease {
                budget: self.clone(),
                id,
                writer: false,
            }),
        })
    }
    pub(crate) fn reserve(self: &Arc<Self>, path: &Path, bytes: u64) -> Result<Option<Lease>> {
        let mut state = self.state.lock().expect("pending disk budget");
        if state.paths.contains_key(path) {
            return Err(invalid("publication temporary slot has not been cleared"));
        }
        if bytes
            > state
                .counters
                .limit_bytes
                .saturating_sub(state.counters.retained_bytes)
        {
            state.counters.refused_publications =
                state.counters.refused_publications.saturating_add(1);
            return Ok(None);
        }
        let id = state.insert(bytes, None)?;
        state.link(path, id);
        state.files.get_mut(&id).unwrap().writers = 1;
        state.counters.open_writers += 1;
        Ok(Some(Lease {
            budget: self.clone(),
            id,
            writer: true,
        }))
    }
    pub(crate) fn check_no_writer(&self, path: &Path) -> Result<()> {
        let state = self.state.lock().expect("pending disk budget");
        if state
            .paths
            .get(path)
            .is_some_and(|id| state.files[id].writers != 0)
        {
            return Err(invalid("publication temporary slot has a live writer"));
        }
        Ok(())
    }
    pub(crate) fn published(&self, tmp: &Path, path: &Path) -> Result<()> {
        let mut state = self.state.lock().expect("pending disk budget");
        let id = *state
            .paths
            .get(tmp)
            .ok_or_else(|| invalid("publication reservation missing"))?;
        state.link(path, id);
        state.unlink(tmp);
        Ok(())
    }
    pub(crate) fn forgotten(&self, path: &Path) {
        self.state.lock().expect("pending disk budget").unlink(path);
    }
    pub(crate) fn deferred_prepared(&self) {
        let mut state = self.state.lock().expect("pending disk budget");
        state.counters.prepared_updates_deferred =
            state.counters.prepared_updates_deferred.saturating_add(1);
    }
    /// After authorized retirement: verify and sync absence, then retire named
    /// charges only. Any open inode lease remains charged independently.
    pub fn observe_removed_journal(&self, path: &Path) -> Result<()> {
        let tmp = path.with_extension("pending.tmp");
        for path in [path, tmp.as_path()] {
            self.check_no_writer(path)?;
            match fs::symlink_metadata(path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(io(error)),
                Ok(_) => return Err(invalid("journal file remains present")),
            }
        }
        crate::flush_journal::sync_parent(path)?;
        self.forgotten(path);
        self.forgotten(&tmp);
        Ok(())
    }
}
impl Default for PendingDiskBudget {
    fn default() -> Self {
        Self::new(DEFAULT_PENDING_BYTES).expect("positive pending disk budget")
    }
}
impl State {
    fn insert(&mut self, bytes: u64, identity: Option<Identity>) -> Result<u64> {
        let id = self
            .serial
            .checked_add(1)
            .ok_or_else(|| invalid("file serial exhausted"))?;
        let retained = self
            .counters
            .retained_bytes
            .checked_add(bytes)
            .ok_or_else(|| invalid("file debt overflow"))?;
        self.serial = id;
        self.counters.retained_bytes = retained;
        self.counters.peak_bytes = self.counters.peak_bytes.max(retained);
        self.files.insert(
            id,
            Charge {
                bytes,
                identity,
                links: 0,
                readers: 0,
                writers: 0,
            },
        );
        if let Some(identity) = identity {
            self.identities.insert(identity, id);
        }
        Ok(id)
    }
    fn observe(&mut self, path: &Path, metadata: &Metadata) -> Result<u64> {
        if !metadata.is_file() {
            return Err(invalid("journal path is not a regular file"));
        }
        let identity = file_identity(metadata);
        let id = match identity.and_then(|identity| self.identities.get(&identity).copied()) {
            Some(id) => {
                let old = self.files[&id].bytes;
                if metadata.len() > old {
                    self.counters.retained_bytes = self
                        .counters
                        .retained_bytes
                        .checked_add(metadata.len() - old)
                        .ok_or_else(|| invalid("file debt overflow"))?;
                    self.files.get_mut(&id).unwrap().bytes = metadata.len();
                    self.counters.peak_bytes =
                        self.counters.peak_bytes.max(self.counters.retained_bytes);
                }
                id
            }
            None => self.insert(metadata.len(), identity)?,
        };
        self.link(path, id);
        Ok(id)
    }
    fn link(&mut self, path: &Path, id: u64) {
        if self.paths.get(path) == Some(&id) {
            return;
        }
        self.files.get_mut(&id).unwrap().links += 1;
        if let Some(old) = self.paths.insert(path.to_path_buf(), id) {
            self.files.get_mut(&old).unwrap().links -= 1;
            self.collect(old);
        }
    }
    fn unlink(&mut self, path: &Path) {
        if let Some(id) = self.paths.remove(path) {
            self.files.get_mut(&id).unwrap().links -= 1;
            self.collect(id);
        }
    }
    fn collect(&mut self, id: u64) {
        let file = &self.files[&id];
        if file.links != 0 || file.readers != 0 || file.writers != 0 {
            return;
        }
        let file = self.files.remove(&id).unwrap();
        self.counters.retained_bytes -= file.bytes;
        if let Some(identity) = file.identity {
            self.identities.remove(&identity);
        }
    }
}

#[derive(Debug)]
pub(crate) struct Lease {
    budget: Arc<PendingDiskBudget>,
    id: u64,
    writer: bool,
}
impl Lease {
    pub(crate) fn opened(self, file: File) -> Result<TrackedFile> {
        let metadata = file.metadata().map_err(io)?;
        {
            let mut state = self.budget.state.lock().expect("pending disk budget");
            let identity = file_identity(&metadata);
            if identity.is_some_and(|key| state.identities.contains_key(&key)) {
                return Err(invalid("new publication reused a tracked inode"));
            }
            if metadata.len() > state.files[&self.id].bytes {
                return Err(invalid("new publication exceeds reservation"));
            }
            state.files.get_mut(&self.id).unwrap().identity = identity;
            if let Some(identity) = identity {
                state.identities.insert(identity, self.id);
            }
        }
        Ok(TrackedFile {
            file,
            _lease: Some(self),
        })
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        let mut state = self.budget.state.lock().expect("pending disk budget");
        if self.writer {
            state.files.get_mut(&self.id).unwrap().writers -= 1;
            state.counters.open_writers -= 1;
        } else {
            state.files.get_mut(&self.id).unwrap().readers -= 1;
            state.counters.open_readers -= 1;
        }
        state.collect(self.id);
    }
}
/// Field order closes the real FD before returning its inode charge.
#[derive(Debug)]
pub(crate) struct TrackedFile {
    file: File,
    _lease: Option<Lease>,
}
impl TrackedFile {
    pub fn untracked(file: File) -> Self {
        Self { file, _lease: None }
    }
    pub fn sync_all(&self) -> std::io::Result<()> {
        self.file.sync_all()
    }
    pub fn metadata(&self) -> std::io::Result<Metadata> {
        self.file.metadata()
    }
}
impl Read for TrackedFile {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(bytes)
    }
}
impl std::io::Write for TrackedFile {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        std::io::Write::write(&mut self.file, bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        std::io::Write::flush(&mut self.file)
    }
}
impl Seek for TrackedFile {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.file.seek(position)
    }
}
#[cfg(unix)]
fn file_identity(metadata: &Metadata) -> Option<Identity> {
    use std::os::unix::fs::MetadataExt;
    Some((metadata.dev(), metadata.ino()))
}
// Without stable inode identity, distinct opens are conservatively charged
// separately until close. Production qualification is currently Linux only.
#[cfg(not(unix))]
fn file_identity(_: &Metadata) -> Option<Identity> {
    None
}
fn invalid(message: &str) -> Error {
    Error::Engine(format!("pending disk budget: {message}"))
}
fn io(error: std::io::Error) -> Error {
    invalid(&error.to_string())
}
