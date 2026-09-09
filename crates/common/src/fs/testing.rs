//! Deterministic append-log filesystem model. It runs the caller's real
//! serialization, write, sync and recovery code. No host files are modified.
//!
//! Data durability and directory-entry durability are independent. Each sync
//! commits only its own file or immediate directory children. Crashes discard
//! or partially persist unsynced state, then invalidate pre-crash handles.
//! Rename, unlink, symlinks, bit rot and hardware lying about successful sync
//! are outside this initial model; they must not be inferred from its results.

use super::{DurableFile, FileSystem};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    CreateDir,
    IsDir,
    Canonicalize,
    OpenAppend,
    Read,
    Write,
    Seek,
    SetLen,
    SyncData,
    SyncDir,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    Before(i32),
    After(i32),
    /// Return a short successful write, then the selected errno on the next
    /// write to that handle. This exercises `Write::write_all`'s real loop.
    ShortWrite {
        bytes: usize,
        errno: i32,
    },
}

#[derive(Clone, Debug)]
pub struct Event {
    pub number: usize,
    pub operation: Operation,
    pub path: PathBuf,
    pub fault: Option<Fault>,
}

#[derive(Clone, Copy, Debug)]
pub enum Crash {
    LoseUnsynced,
    KeepUnsynced,
    /// Replayable partial unsynced byte and directory-entry persistence.
    Seeded(u64),
}

struct Node {
    entry_durable: bool,
    file: Option<FileData>,
}

struct FileData {
    visible: Vec<u8>,
    durable: Vec<u8>,
}

struct State {
    nodes: BTreeMap<PathBuf, Node>,
    events: Vec<Event>,
    fault: Option<(usize, Fault)>,
    epoch: u64,
}

impl State {
    fn arrive(&mut self, operation: Operation, path: &Path) -> io::Result<Option<Fault>> {
        let number = self.events.len();
        let fault = match self.fault {
            Some((at, fault)) if at == number => {
                self.fault = None;
                Some(fault)
            }
            _ => None,
        };
        self.events.push(Event {
            number,
            operation,
            path: path.to_path_buf(),
            fault,
        });
        if let Some(Fault::Before(errno)) = fault {
            return Err(io::Error::from_raw_os_error(errno));
        }
        if matches!(fault, Some(Fault::ShortWrite { .. })) && operation != Operation::Write {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "short-write fault on a non-write",
            ));
        }
        Ok(fault)
    }

    fn file(&mut self, path: &Path, epoch: u64) -> io::Result<&mut FileData> {
        if self.epoch != epoch {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "handle invalidated by crash",
            ));
        }
        self.nodes
            .get_mut(path)
            .and_then(|n| n.file.as_mut())
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }

    fn require_dir(&self, path: &Path) -> io::Result<()> {
        match self.nodes.get(path) {
            Some(Node { file: None, .. }) => Ok(()),
            Some(_) => Err(io::Error::from(io::ErrorKind::NotADirectory)),
            None => Err(io::Error::from(io::ErrorKind::NotFound)),
        }
    }
}

fn complete<T>(fault: Option<Fault>, result: T) -> io::Result<T> {
    match fault {
        Some(Fault::After(errno)) => Err(io::Error::from_raw_os_error(errno)),
        _ => Ok(result),
    }
}

fn normalized(path: &Path) -> io::Result<PathBuf> {
    let mut result = PathBuf::new();
    for component in std::path::absolute(path)?.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            other => result.push(other),
        }
    }
    Ok(result)
}

#[derive(Clone)]
pub struct ModelFs(Arc<Mutex<State>>);

impl Default for ModelFs {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(State {
            nodes: BTreeMap::from([(
                PathBuf::from("/"),
                Node {
                    entry_durable: true,
                    file: None,
                },
            )]),
            events: Vec::new(),
            fault: None,
            epoch: 0,
        })))
    }
}

impl ModelFs {
    pub fn events(&self) -> Vec<Event> {
        self.0.lock().unwrap().events.clone()
    }

    pub fn clear_events(&self) {
        let mut state = self.0.lock().unwrap();
        assert!(
            state.fault.is_none(),
            "cannot erase an armed fault schedule"
        );
        state.events.clear();
    }

    pub fn fail_at(&self, number: usize, fault: Fault) {
        let mut state = self.0.lock().unwrap();
        assert!(
            state.fault.is_none() && number >= state.events.len(),
            "invalid fault schedule"
        );
        state.fault = Some((number, fault));
    }

    pub fn fault_arrived(&self) -> bool {
        self.0
            .lock()
            .unwrap()
            .events
            .iter()
            .any(|e| e.fault.is_some())
    }

    /// Inspect bytes independently of the caller's decoder and read position.
    pub fn visible_bytes(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.0
            .lock()
            .unwrap()
            .nodes
            .get(&normalized(path)?)
            .and_then(|n| n.file.as_ref())
            .map(|f| f.visible.clone())
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }

    pub fn crash(&self, mode: Crash) {
        let mut state = self.0.lock().unwrap();
        assert!(
            state.fault.is_none(),
            "fault schedule never arrived before crash"
        );
        let mut seed = match mode {
            Crash::Seeded(seed) => seed,
            _ => 0,
        };
        let mut keep = || match mode {
            Crash::LoseUnsynced => false,
            Crash::KeepUnsynced => true,
            Crash::Seeded(_) => {
                // Fixed arithmetic and sorted paths make the complete cut replayable.
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                seed >> 63 != 0
            }
        };
        for node in state.nodes.values_mut() {
            node.entry_durable |= keep();
            if let Some(file) = &mut node.file {
                let len = if keep() {
                    file.visible.len()
                } else {
                    file.durable.len()
                };
                let mut recovered = Vec::with_capacity(len);
                for i in 0..len {
                    let old = file.durable.get(i).copied().unwrap_or(0);
                    let new = file.visible.get(i).copied().unwrap_or(old);
                    recovered.push(if keep() { new } else { old });
                }
                file.visible = recovered.clone();
                file.durable = recovered;
            }
        }
        let lost: BTreeSet<_> = state
            .nodes
            .iter()
            .filter(|(_, n)| !n.entry_durable)
            .map(|(p, _)| p.clone())
            .collect();
        state
            .nodes
            .retain(|path, _| !path.ancestors().any(|p| lost.contains(p)));
        state.epoch += 1;
    }
}

impl FileSystem for ModelFs {
    type File = ModelFile;

    fn create_dir(&self, path: &Path) -> io::Result<()> {
        let path = normalized(path)?;
        let mut state = self.0.lock().unwrap();
        let fault = state.arrive(Operation::CreateDir, &path)?;
        if state.nodes.contains_key(&path) {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
        state.require_dir(path.parent().ok_or(io::ErrorKind::NotFound)?)?;
        state.nodes.insert(
            path,
            Node {
                entry_durable: false,
                file: None,
            },
        );
        complete(fault, ())
    }

    fn is_dir(&self, path: &Path) -> io::Result<bool> {
        let path = normalized(path)?;
        let mut state = self.0.lock().unwrap();
        let fault = state.arrive(Operation::IsDir, &path)?;
        let dir = state
            .nodes
            .get(&path)
            .ok_or(io::ErrorKind::NotFound)?
            .file
            .is_none();
        complete(fault, dir)
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        let path = normalized(path)?;
        let mut state = self.0.lock().unwrap();
        let fault = state.arrive(Operation::Canonicalize, &path)?;
        state.require_dir(&path)?;
        complete(fault, path)
    }

    fn open_append(&self, path: &Path) -> io::Result<Self::File> {
        let path = normalized(path)?;
        let mut state = self.0.lock().unwrap();
        let fault = state.arrive(Operation::OpenAppend, &path)?;
        state.require_dir(path.parent().ok_or(io::ErrorKind::NotFound)?)?;
        let node = state.nodes.entry(path.clone()).or_insert_with(|| Node {
            entry_durable: false,
            file: Some(FileData {
                visible: Vec::new(),
                durable: Vec::new(),
            }),
        });
        if node.file.is_none() {
            return Err(io::Error::from(io::ErrorKind::IsADirectory));
        }
        complete(
            fault,
            ModelFile {
                fs: self.clone(),
                path,
                offset: 0,
                epoch: state.epoch,
                write_error: None,
            },
        )
    }

    fn open_existing_append(&self, path: &Path) -> io::Result<Self::File> {
        let path = normalized(path)?;
        let mut state = self.0.lock().unwrap();
        let fault = state.arrive(Operation::OpenAppend, &path)?;
        let node = state.nodes.get(&path).ok_or(io::ErrorKind::NotFound)?;
        if node.file.is_none() {
            return Err(io::Error::from(io::ErrorKind::IsADirectory));
        }
        complete(
            fault,
            ModelFile {
                fs: self.clone(),
                path,
                offset: 0,
                epoch: state.epoch,
                write_error: None,
            },
        )
    }

    fn sync_dir(&self, path: &Path) -> io::Result<()> {
        let path = normalized(path)?;
        let mut state = self.0.lock().unwrap();
        let fault = state.arrive(Operation::SyncDir, &path)?;
        state.require_dir(&path)?;
        for (child, node) in &mut state.nodes {
            if child.parent() == Some(path.as_path()) {
                node.entry_durable = true;
            }
        }
        complete(fault, ())
    }
}

pub struct ModelFile {
    fs: ModelFs,
    path: PathBuf,
    offset: usize,
    epoch: u64,
    write_error: Option<i32>,
}

impl Read for ModelFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut state = self.fs.0.lock().unwrap();
        let fault = state.arrive(Operation::Read, &self.path)?;
        let data = &state.file(&self.path, self.epoch)?.visible;
        let n = buf.len().min(data.len().saturating_sub(self.offset));
        if n != 0 {
            buf[..n].copy_from_slice(&data[self.offset..self.offset + n]);
        }
        self.offset += n;
        complete(fault, n)
    }
}

impl Write for ModelFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut state = self.fs.0.lock().unwrap();
        let fault = state.arrive(Operation::Write, &self.path)?;
        if let Some(errno) = self.write_error.take() {
            return Err(io::Error::from_raw_os_error(errno));
        }
        let n = if let Some(Fault::ShortWrite { bytes, errno }) = fault {
            assert!(bytes < buf.len(), "short-write control must leave a suffix");
            self.write_error = Some(errno);
            bytes
        } else {
            buf.len()
        };
        let data = &mut state.file(&self.path, self.epoch)?.visible;
        data.extend_from_slice(&buf[..n]);
        self.offset = data.len();
        complete(fault, n)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for ModelFile {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let mut state = self.fs.0.lock().unwrap();
        let fault = state.arrive(Operation::Seek, &self.path)?;
        let len = state.file(&self.path, self.epoch)?.visible.len() as i128;
        let offset = match to {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::End(n) => len + i128::from(n),
            SeekFrom::Current(n) => self.offset as i128 + i128::from(n),
        };
        self.offset = usize::try_from(offset).map_err(|_| io::ErrorKind::InvalidInput)?;
        complete(fault, self.offset as u64)
    }
}

impl DurableFile for ModelFile {
    fn sync_data(&self) -> io::Result<()> {
        let mut state = self.fs.0.lock().unwrap();
        let fault = state.arrive(Operation::SyncData, &self.path)?;
        let file = state.file(&self.path, self.epoch)?;
        file.durable.clone_from(&file.visible);
        complete(fault, ())
    }

    fn set_len(&self, len: u64) -> io::Result<()> {
        let mut state = self.fs.0.lock().unwrap();
        let fault = state.arrive(Operation::SetLen, &self.path)?;
        let len = usize::try_from(len).map_err(|_| io::ErrorKind::InvalidInput)?;
        state.file(&self.path, self.epoch)?.visible.resize(len, 0);
        complete(fault, ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::{create_dirs, sync_ancestors};

    #[test]
    fn file_sync_does_not_publish_its_name() {
        let fs = ModelFs::default();
        create_dirs(&fs, Path::new("/data")).unwrap();
        sync_ancestors(&fs, Path::new("/data")).unwrap();
        let mut f = fs.open_append(Path::new("/data/log")).unwrap();
        f.write_all(b"vote").unwrap();
        f.sync_data().unwrap();
        fs.crash(Crash::LoseUnsynced);
        assert!(
            fs.visible_bytes(Path::new("/data/log")).is_err(),
            "file data sync must not publish its name"
        );
    }

    #[test]
    fn directory_sync_does_not_publish_file_data_or_ancestors() {
        let fs = ModelFs::default();
        create_dirs(&fs, Path::new("/a/b")).unwrap();
        let mut f = fs.open_append(Path::new("/a/b/log")).unwrap();
        f.write_all(b"unsynced").unwrap();
        fs.sync_dir(Path::new("/a/b")).unwrap();
        fs.crash(Crash::LoseUnsynced);
        assert!(
            fs.visible_bytes(Path::new("/a/b/log")).is_err(),
            "leaf sync must not publish ancestor names"
        );

        create_dirs(&fs, Path::new("/a/b")).unwrap();
        let mut f = fs.open_append(Path::new("/a/b/log")).unwrap();
        f.write_all(b"unsynced").unwrap();
        sync_ancestors(&fs, Path::new("/a/b")).unwrap();
        fs.crash(Crash::LoseUnsynced);
        assert!(
            fs.visible_bytes(Path::new("/a/b/log")).unwrap().is_empty(),
            "directory sync must not persist file contents"
        );
    }

    #[test]
    fn failed_sync_can_have_persisted_its_effect_and_old_handles_expire() {
        let fs = ModelFs::default();
        let mut f = fs.open_append(Path::new("/log")).unwrap();
        f.write_all(b"vote").unwrap();
        fs.sync_dir(Path::new("/")).unwrap();
        fs.fail_at(fs.events().len(), Fault::After(5));
        assert!(f.sync_data().is_err());
        assert!(fs.fault_arrived());
        fs.crash(Crash::LoseUnsynced);
        assert_eq!(fs.visible_bytes(Path::new("/log")).unwrap(), b"vote");
        assert!(f.write_all(b"stale").is_err());
    }

    #[test]
    fn seeded_crashes_preserve_synced_prefix_and_replay_exactly() {
        let run = |seed| {
            let fs = ModelFs::default();
            let mut f = fs.open_append(Path::new("/log")).unwrap();
            f.write_all(b"stable").unwrap();
            f.sync_data().unwrap();
            fs.sync_dir(Path::new("/")).unwrap();
            f.write_all(b"volatile tail").unwrap();
            fs.crash(Crash::Seeded(seed));
            fs.visible_bytes(Path::new("/log")).unwrap()
        };
        let mut outcomes = BTreeSet::new();
        for seed in 0..32 {
            let bytes = run(seed);
            assert!(bytes.starts_with(b"stable"));
            assert_eq!(
                bytes,
                run(seed),
                "seed must reproduce exact persisted bytes"
            );
            outcomes.insert(bytes);
        }
        assert!(
            outcomes.len() > 2,
            "seeded mode must explore partial persistence"
        );
    }
}
