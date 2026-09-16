//! Durable local preparation for dynamically allocated data groups.
//!
//! A committed metadata intent authorizes preparation on its exact store
//! incarnation. StorageReady is not permission to vote, serve or publish a
//! range. Network capability negotiation and activation are subsequent steps.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kv9_common::{Error, RegionId, Result, RootDigest, StoreIdentity, StoreIncarnation};
use kv9_engine::{DurableAppliedPosition, ReplicatedEngine, WalEngine};
use kv9_meta::data_groups::{
    committed_creations, CommittedCreation, CreationIntent, MAX_INTENT_BYTES,
};
use kv9_raft::storage::DiskRaftStorage;

const RECORD: &str = "group-record";
const LOCK: &str = "group-lock";
const MAGIC: &[u8; 8] = b"KV9LOC01";
const MAX_RECORD_BYTES: usize = 8 + 1 + 8 + 16 + MAX_INTENT_BYTES + 32;
const MAX_LOCAL_GROUPS: usize = 255;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    IntentDurable = 0,
    StorageReady = 1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Record {
    phase: Phase,
    node: kv9_common::NodeId,
    incarnation: StoreIncarnation,
    intent: CreationIntent,
}
impl Record {
    fn encode(&self) -> Vec<u8> {
        let mut bytes = MAGIC.to_vec();
        bytes.push(self.phase as u8);
        bytes.extend_from_slice(&self.node.0.to_be_bytes());
        bytes.extend_from_slice(self.incarnation.as_bytes());
        bytes.extend_from_slice(&self.intent.encode());
        bytes.extend_from_slice(RootDigest::sha256(&bytes).as_bytes());
        bytes
    }

    fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 65 + 73 || bytes.len() > MAX_RECORD_BYTES || &bytes[..8] != MAGIC {
            return Err(invalid("invalid local group record"));
        }
        let end = bytes.len() - 32;
        if RootDigest::sha256(&bytes[..end]).as_bytes() != &bytes[end..] {
            return Err(invalid("local group record checksum mismatch"));
        }
        let phase = match bytes[8] {
            0 => Phase::IntentDurable,
            1 => Phase::StorageReady,
            _ => return Err(invalid("unknown local group phase")),
        };
        Ok(Self {
            phase,
            node: kv9_common::NodeId(u64::from_be_bytes(bytes[9..17].try_into().unwrap())),
            incarnation: StoreIncarnation::from_bytes(bytes[17..33].try_into().unwrap()),
            intent: CreationIntent::decode(&bytes[33..end])?,
        })
    }
}

/// Observation of durable preparation only, never a routing/activation receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupPreparation {
    pub task: u64,
    pub region: RegionId,
    pub intent_digest: RootDigest,
}

struct PreparedLocal {
    observation: GroupPreparation,
    // Held until manager shutdown under the parent StoreGuard. No driver or
    // public engine handle can escape while activation is not implemented.
    _storage: DiskRaftStorage,
    _engine: Arc<WalEngine>,
    _lock: File,
}

enum LocalGroup {
    Ready(Box<PreparedLocal>),
    Failed(String),
}

/// The enclosing NodeRuntime owns the exclusive parent store lock. Each child
/// also has its own lock; a second preparation cannot open its logs concurrently.
pub(crate) struct RegionManager {
    directory: PathBuf,
    identity: StoreIdentity,
    groups: BTreeMap<RegionId, LocalGroup>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrepareStep {
    IntentFileSync,
    IntentRename,
    IntentDirectorySync,
    RaftOpen,
    EngineOpen,
    ReadyFileSync,
    ReadyRename,
    ReadyDirectorySync,
}

fn invalid(message: &str) -> Error {
    Error::Config(message.into())
}
fn io(error: std::io::Error) -> Error {
    Error::Engine(format!("data group storage: {error}"))
}

fn read_record(path: &Path) -> Result<Option<Record>> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io(e)),
    };
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_RECORD_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    let record = Record::decode(&bytes)?;
    // Visible after a failed directory publication is not yet a durable receipt.
    file.sync_all().map_err(io)?;
    kv9_common::fs::sync_ancestors(&kv9_common::fs::OsFileSystem, path.parent().unwrap())
        .map_err(io)?;
    Ok(Some(record))
}

fn operation<T>(
    step: PrepareStep,
    observe: &mut impl FnMut(PrepareStep, bool) -> Result<()>,
    run: impl FnOnce() -> Result<T>,
) -> Result<T> {
    observe(step, false)?;
    let value = run()?;
    observe(step, true)?;
    Ok(value)
}

fn publish(
    directory: &Path,
    record: &Record,
    observe: &mut impl FnMut(PrepareStep, bool) -> Result<()>,
) -> Result<()> {
    let steps = match record.phase {
        Phase::IntentDurable => [
            PrepareStep::IntentFileSync,
            PrepareStep::IntentRename,
            PrepareStep::IntentDirectorySync,
        ],
        Phase::StorageReady => [
            PrepareStep::ReadyFileSync,
            PrepareStep::ReadyRename,
            PrepareStep::ReadyDirectorySync,
        ],
    };
    let temporary = directory.join(format!(".group-record-{}.tmp", StoreIncarnation::mint()?));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(io)?;
    file.write_all(&record.encode()).map_err(io)?;
    operation(steps[0], observe, || file.sync_all().map_err(io))?;
    operation(steps[1], observe, || {
        fs::rename(&temporary, directory.join(RECORD)).map_err(io)
    })?;
    operation(steps[2], observe, || {
        kv9_common::fs::sync_ancestors(&kv9_common::fs::OsFileSystem, directory).map_err(io)
    })
}

impl RegionManager {
    pub(crate) fn new(directory: &Path, identity: StoreIdentity) -> Self {
        Self {
            directory: directory.join("data-groups"),
            identity,
            groups: BTreeMap::new(),
        }
    }

    pub(crate) fn prepare(&mut self, creation: &CommittedCreation) -> Result<GroupPreparation> {
        self.prepare_observed(creation, &mut |_, _| Ok(()))
    }

    fn prepare_observed(
        &mut self,
        creation: &CommittedCreation,
        observe: &mut impl FnMut(PrepareStep, bool) -> Result<()>,
    ) -> Result<GroupPreparation> {
        let intent = creation.intent();
        if intent.root() != self.identity.root_digest
            || !intent.replicas().iter().any(|r| {
                r.node == self.identity.node_id && r.incarnation == self.identity.store_incarnation
            })
        {
            return Err(invalid(
                "group intent does not authorize this store incarnation",
            ));
        }
        if let Some(group) = self.groups.get(&intent.region()) {
            return match group {
                LocalGroup::Ready(prepared)
                    if prepared.observation.intent_digest == intent.digest() =>
                {
                    Ok(prepared.observation.clone())
                }
                LocalGroup::Ready(_) => Err(invalid("group ID is already bound to another intent")),
                LocalGroup::Failed(cause) => Err(invalid(&format!(
                    "group requires recovery after failure: {cause}"
                ))),
            };
        }
        if self.groups.len() >= MAX_LOCAL_GROUPS {
            return Err(invalid("local group preparation capacity reached"));
        }
        match self.open_local(intent, observe) {
            Ok(prepared) => {
                let observation = prepared.observation.clone();
                self.groups
                    .insert(intent.region(), LocalGroup::Ready(Box::new(prepared)));
                Ok(observation)
            }
            Err(error) => {
                self.groups
                    .insert(intent.region(), LocalGroup::Failed(error.to_string()));
                Err(error)
            }
        }
    }

    fn open_local(
        &self,
        intent: &CreationIntent,
        observe: &mut impl FnMut(PrepareStep, bool) -> Result<()>,
    ) -> Result<PreparedLocal> {
        let directory = self.directory.join(intent.region().0.to_string());
        fs::create_dir_all(&directory).map_err(io)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join(LOCK))
            .map_err(io)?;
        lock.try_lock()
            .map_err(|e| invalid(&format!("data group is already owned: {e}")))?;
        let expected = Record {
            phase: Phase::IntentDurable,
            node: self.identity.node_id,
            incarnation: self.identity.store_incarnation,
            intent: intent.clone(),
        };
        let record = match read_record(&directory.join(RECORD))? {
            Some(record) => {
                if (Record {
                    phase: Phase::IntentDurable,
                    ..record.clone()
                }) != expected
                {
                    return Err(invalid(
                        "local group record disagrees with committed creation",
                    ));
                }
                record
            }
            None => {
                // Before the first durable intent only our lock and incomplete
                // publication files may exist. Never adopt orphaned data/logs.
                for entry in fs::read_dir(&directory).map_err(io)? {
                    let entry = entry.map_err(io)?;
                    let name = entry.file_name();
                    if name == LOCK {
                        continue;
                    }
                    if name
                        .to_str()
                        .is_some_and(|s| s.starts_with(".group-record-") && s.ends_with(".tmp"))
                        && entry.file_type().map_err(io)?.is_file()
                    {
                        continue;
                    }
                    return Err(invalid("group data exists without a durable intent record"));
                }
                publish(&directory, &expected, observe)?;
                expected
            }
        };
        let storage = operation(PrepareStep::RaftOpen, observe, || {
            if record.phase == Phase::StorageReady {
                DiskRaftStorage::recover(&directory.join("raft"))
            } else {
                let voters: Vec<_> = intent.replicas().iter().map(|r| r.node.0).collect();
                DiskRaftStorage::open(&directory.join("raft"), &voters).map(|s| s.0)
            }
        })?;
        let engine = operation(PrepareStep::EngineOpen, observe, || {
            let voters: Vec<_> = intent.replicas().iter().map(|r| r.node.0).collect();
            storage.validate_unstarted_group(&voters)?;
            let path = directory.join("data.wal");
            if record.phase == Phase::StorageReady
                && (!path.is_file() || !path.with_extension("segments").is_dir())
            {
                return Err(invalid("prepared group is missing its engine WAL topology"));
            }
            let (engine, _) = WalEngine::open_with_replay_observer(
                &path,
                None,
                |base| {
                    if base.is_none() {
                        Ok(())
                    } else {
                        Err(invalid("unstarted group has a checkpoint"))
                    }
                },
                |batch, at| {
                    if batch.is_empty() && at.is_none() {
                        Ok(())
                    } else {
                        Err(invalid(
                            "unstarted group contains user data or applied history",
                        ))
                    }
                },
            )?;
            if engine.applied_position()? != DurableAppliedPosition::AppliedNothing {
                return Err(invalid("unstarted group has an applied position"));
            }
            engine.enable_segmentation()?;
            Ok(Arc::new(engine))
        })?;
        if record.phase != Phase::StorageReady {
            publish(
                &directory,
                &Record {
                    phase: Phase::StorageReady,
                    ..record
                },
                observe,
            )?;
        }
        Ok(PreparedLocal {
            observation: GroupPreparation {
                task: intent.task(),
                region: intent.region(),
                intent_digest: intent.digest(),
            },
            _storage: storage,
            _engine: engine,
            _lock: lock,
        })
    }

    /// Recovery is isolated per group. Bad/missing authority cannot recreate a
    /// replica, and cannot prevent metadata or another prepared group opening.
    pub(crate) fn recover(&mut self, store: &kv9_meta::MetaStore<WalEngine>) -> Result<()> {
        let entries = match fs::read_dir(&self.directory) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(io(e)),
        };
        let creations = committed_creations(store)?;
        for (index, entry) in entries.enumerate() {
            if index >= MAX_LOCAL_GROUPS {
                return Err(invalid("local group directory capacity exceeded"));
            }
            let entry = entry.map_err(io)?;
            let name = entry.file_name();
            let id = name
                .to_str()
                .and_then(|s| s.parse::<u64>().ok())
                .map(RegionId)
                .filter(|id| name == id.0.to_string().as_str())
                .ok_or_else(|| invalid("invalid group directory name"))?;
            let result = (|| {
                if !entry.file_type().map_err(io)?.is_dir() {
                    return Err(invalid("group path is not a directory"));
                }
                // Do not open any Raft/engine file until exact metadata binding.
                if let Some(record) = read_record(&entry.path().join(RECORD))? {
                    if record.intent.region() != id {
                        return Err(invalid("group directory and record identity differ"));
                    }
                }
                let creation = creations
                    .iter()
                    .find(|creation| creation.intent().region() == id)
                    .ok_or_else(|| invalid("group creation is absent from applied metadata"))?;
                self.prepare(creation).map(|_| ())
            })();
            if let Err(e) = result {
                self.groups.insert(id, LocalGroup::Failed(e.to_string()));
            }
        }
        Ok(())
    }

    pub(crate) fn observations(&self) -> Vec<(RegionId, Result<GroupPreparation>)> {
        self.groups
            .iter()
            .map(|(id, group)| {
                (
                    *id,
                    match group {
                        LocalGroup::Ready(p) => Ok(p.observation.clone()),
                        LocalGroup::Failed(cause) => Err(invalid(cause)),
                    },
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
