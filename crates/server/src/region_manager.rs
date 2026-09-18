//! Durable local preparation for dynamically allocated data groups.
//!
//! A committed metadata intent authorizes preparation on its exact store
//! incarnation. StorageReady is not permission to vote, serve or publish a
//! range. Active is durable before any voting; public range publication remains
//! separate. Data groups use the dedicated, non-fallback group RPC.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use kv9_common::{Error, RegionId, Result, RootDigest, StoreIdentity, StoreIncarnation};
use kv9_engine::{DurableAppliedPosition, Engine, ReplicatedEngine, WalEngine};
use kv9_meta::data_groups::{
    committed_creations, CommittedCreation, CreationIntent, MAX_INTENT_BYTES,
};
use kv9_raft::driver::{DriverPool, NodeDriver, NodeStatus};
use kv9_raft::grpc::GrpcTransport;
use kv9_raft::storage::DiskRaftStorage;
use kv9_raft::{MemStateMachine, RaftPeer};

const RECORD: &str = "group-record";
const LOCK: &str = "group-lock";
const MAGIC: &[u8; 8] = b"KV9LOC01";
const MAX_RECORD_BYTES: usize = 8 + 1 + 8 + 16 + MAX_INTENT_BYTES + 32;
const MAX_LOCAL_GROUPS: usize = 255;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    IntentDurable = 0,
    StorageReady = 1,
    Active = 2,
    /// The committed removal named this exact store; the local replica is
    /// permanently fenced. Storage stays durable; nothing is deleted.
    Retired = 3,
    /// The retired payload (engine WAL/segments and raft log) has been
    /// physically deleted under re-verified committed authority. The record
    /// itself IS the surviving fence and is never deleted.
    Reclaimed = 4,
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
            2 => Phase::Active,
            3 => Phase::Retired,
            4 => Phase::Reclaimed,
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
    range_proposal: Option<kv9_raft::ProposedAt>,
    observation: GroupPreparation,
    phase: Phase,
    intent: CreationIntent,
    storage: Option<DiskRaftStorage>,
    engine: Arc<WalEngine>,
    driver: Option<Arc<NodeDriver<DiskRaftStorage, WalEngine>>>,
    _lock: File,
}

enum LocalGroup {
    Ready(Box<PreparedLocal>),
    Failed(String),
    /// Permanently fenced by a committed removal; the lock keeps any other
    /// opener out and the durable storage untouched.
    Retired {
        _lock: File,
    },
    /// Fenced AND physically reclaimed: only the record and lock remain.
    Reclaimed {
        _lock: File,
    },
}

/// The enclosing NodeRuntime owns the exclusive parent store lock. Each child
/// also has its own lock; a second preparation cannot open its logs concurrently.
/// The destination's durable adoption facts, replayed as the canonical
/// KV9EVD01 evidence receipt. Emitting this grants no quiesce or release
/// capability; only the committed catalog row does.
#[derive(Clone)]
pub(crate) struct AdoptionReceipt {
    pub(crate) receipt: Vec<u8>,
    pub(crate) image_digest: kv9_common::RootDigest,
    pub(crate) cut: kv9_common::AppliedPosition,
}

/// Engine + driver handles for EVERY locally started group — bound or not.
/// Shared with the RPC backend (like the raw directory), because unbound
/// split children have no raw-directory entry yet.
pub(crate) type GroupHandles = Arc<
    std::sync::Mutex<
        BTreeMap<RegionId, (Arc<WalEngine>, Arc<NodeDriver<DiskRaftStorage, WalEngine>>)>,
    >,
>;

pub(crate) struct RegionManager {
    pub(crate) raw_directory: Arc<crate::runtime::range_api::RawDirectory>,
    pub(crate) group_handles: GroupHandles,
    directory: PathBuf,
    identity: StoreIdentity,
    groups: BTreeMap<RegionId, LocalGroup>,
    pub(crate) adoption_receipts: Arc<std::sync::Mutex<BTreeMap<RegionId, AdoptionReceipt>>>,
    pool: Option<DriverPool<DiskRaftStorage, WalEngine>>,
    /// Bounded shared Ready/tick worker count, fixed at construction from the
    /// validated node configuration. The metadata owner is separate.
    data_workers: usize,
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
    ActiveFileSync,
    ActiveRename,
    ActiveDirectorySync,
}

fn invalid(message: &str) -> Error {
    Error::Config(message.into())
}
fn io(error: std::io::Error) -> Error {
    Error::Engine(format!("data group storage: {error}"))
}

/// An engine position matches the retained Raft history, or sits at/below a
/// durable compacted base — where the history is legitimately unavailable
/// and the REC_COMPACTION record (gated on a committed truncation decision
/// at startup) vouches for the whole prefix. The base position itself must
/// match terms exactly.
fn position_in_history(storage: &DiskRaftStorage, at: kv9_common::AppliedPosition) -> Result<bool> {
    if let Some(base) = storage.compacted_base()? {
        if at.index < base.index {
            return Ok(true);
        }
        if at.index == base.index {
            return Ok(at.term == base.term);
        }
    }
    Ok(storage.committed_term(at.index)? == at.term)
}

/// KV9_DATA_SYNC_DEFER_BYTES: deferred apply-sync threshold for DATA-GROUP
/// engines (0 = strict per-apply sync, the default). Metadata catalogs are
/// NEVER deferred. Legal because acknowledged writes rest on the synced
/// raft log and deterministic replay; the compaction path forces a sync
/// barrier before discarding any replay source.
fn data_sync_defer_bytes() -> u64 {
    std::env::var("KV9_DATA_SYNC_DEFER_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
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
        Phase::Active | Phase::Retired | Phase::Reclaimed => [
            PrepareStep::ActiveFileSync,
            PrepareStep::ActiveRename,
            PrepareStep::ActiveDirectorySync,
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

/// Delete a group directory's PAYLOAD only — engine WAL, segment directory,
/// raft log — leaving the record (the fence) and the lock file untouched.
/// Idempotent: absent paths are simply already gone. Returns bytes removed.
fn delete_group_payload(directory: &Path) -> Result<u64> {
    let mut freed = 0u64;
    let wal = directory.join("data.wal");
    if let Ok(meta) = fs::metadata(&wal) {
        freed += meta.len();
        fs::remove_file(&wal).map_err(io)?;
    }
    for name in ["data.segments", "raft"] {
        let path = directory.join(name);
        if !path.exists() {
            continue;
        }
        let mut stack = vec![path.clone()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).map_err(io)?.flatten() {
                let child = entry.path();
                if child.is_dir() {
                    stack.push(child);
                } else if let Ok(meta) = entry.metadata() {
                    freed += meta.len();
                }
            }
        }
        fs::remove_dir_all(&path).map_err(io)?;
    }
    kv9_common::fs::sync_ancestors(&kv9_common::fs::OsFileSystem, directory).map_err(io)?;
    Ok(freed)
}

impl RegionManager {
    pub(crate) fn new(directory: &Path, identity: StoreIdentity, data_workers: usize) -> Self {
        Self {
            raw_directory: Arc::default(),
            group_handles: Arc::default(),
            directory: directory.join("data-groups"),
            identity,
            groups: BTreeMap::new(),
            adoption_receipts: Arc::default(),
            pool: None,
            data_workers: data_workers.clamp(1, 32),
        }
    }

    pub(crate) fn prepare(&mut self, creation: &CommittedCreation) -> Result<GroupPreparation> {
        self.prepare_observed(creation, &mut |_, _| Ok(()))
    }

    pub(crate) fn activate(
        &mut self,
        creation: &CommittedCreation,
        transport: &Arc<GrpcTransport>,
        tick: Duration,
    ) -> Result<()> {
        self.prepare(creation)?;
        self.start_group(
            creation.intent().region(),
            transport,
            tick,
            &[],
            &[],
            &mut |_, _| Ok(()),
        )
    }

    fn start_group(
        &mut self,
        region: RegionId,
        transport: &Arc<GrpcTransport>,
        tick: Duration,
        truncations: &[kv9_meta::data_groups::truncation::TruncationDecision],
        compactions: &[kv9_meta::data_groups::compaction::GroupCompaction],
        observe: &mut impl FnMut(PrepareStep, bool) -> Result<()>,
    ) -> Result<()> {
        match self.groups.get(&region) {
            Some(LocalGroup::Ready(p)) if p.driver.is_some() => {
                return match p.driver.as_ref().unwrap().status().fatal {
                    Some(cause) => Err(Error::Raft(cause)),
                    None => Ok(()),
                };
            }
            Some(LocalGroup::Ready(_)) => {}
            _ => return Err(invalid("group is not prepared for activation")),
        }
        // Fixed data workers are separate from the metadata owner. Thread
        // allocation failure precedes publication or acquisition of a voter.
        if self.pool.is_none() {
            self.pool = Some(DriverPool::new(self.data_workers, tick)?);
        }
        let LocalGroup::Ready(mut prepared) = self.groups.remove(&region).unwrap() else {
            unreachable!()
        };
        self.groups.insert(
            region,
            LocalGroup::Failed("activation did not complete; recovery required".into()),
        );
        let directory = self.directory.join(region.0.to_string());
        if prepared.phase == Phase::StorageReady {
            publish(
                &directory,
                &Record {
                    phase: Phase::Active,
                    node: self.identity.node_id,
                    incarnation: self.identity.store_incarnation,
                    intent: prepared.intent.clone(),
                },
                observe,
            )?;
            prepared.phase = Phase::Active;
        }
        // No peer, inbox or worker can vote/send before durable Active.
        let storage = prepared
            .storage
            .take()
            .ok_or_else(|| invalid("group storage already has a voter owner"))?;
        // A compacted prefix is legitimate ONLY under committed authority whose
        // floor equals the durable base exactly: either a kind-105 migration
        // truncation decision OR a kind-110 healthy-group compaction floor
        // (follower-side compaction persists a compacted base on every voter,
        // not just the leader, so recovery must accept the compaction floor
        // that authorized it). Anything else keeps the refusal.
        let installed_base = match storage.compacted_base()? {
            None => None,
            Some(base) => {
                let by_truncation = truncations
                    .iter()
                    .any(|d| d.region() == region && d.floor() == base);
                let by_compaction = compactions
                    .iter()
                    .any(|c| c.region() == region && c.floor() == base);
                if !by_truncation && !by_compaction {
                    return Err(invalid(
                        "compacted group log lacks a committed truncation decision",
                    ));
                }
                Some(base)
            }
        };
        let peer = Arc::new(match installed_base {
            None => RaftPeer::with_storage(self.identity.node_id, region, storage)?,
            Some(base) => {
                RaftPeer::with_installed_storage(self.identity.node_id, region, storage, base)?
            }
        });
        let mut state = MemStateMachine::with_engine(prepared.engine.clone())?;
        state.set_data_group(prepared.intent.root(), region, prepared.intent.digest())?;
        let driver = match installed_base {
            None => NodeDriver::new(peer, transport.register_group(region)?, state)?,
            Some(base) => NodeDriver::with_installed_base(
                peer,
                transport.register_group(region)?,
                state,
                base,
            )?,
        };
        self.pool.as_ref().unwrap().register(driver.clone())?;
        self.group_handles
            .lock()
            .expect("group handles poisoned")
            .insert(region, (prepared.engine.clone(), driver.clone()));
        prepared.driver = Some(driver);
        self.groups.insert(region, LocalGroup::Ready(prepared));
        Ok(())
    }

    /// Resume only already durable Active groups, after the enclosing runtime
    /// has recovered membership/endpoint authority. A bad group stays isolated.
    pub(crate) fn resume_active(
        &mut self,
        transport: &Arc<GrpcTransport>,
        tick: Duration,
        truncations: &[kv9_meta::data_groups::truncation::TruncationDecision],
        compactions: &[kv9_meta::data_groups::compaction::GroupCompaction],
    ) {
        let regions: Vec<_> = self
            .groups
            .iter()
            .filter_map(|(id, group)| match group {
                LocalGroup::Ready(p) if p.phase == Phase::Active && p.driver.is_none() => Some(*id),
                _ => None,
            })
            .collect();
        for region in regions {
            if let Err(error) = self.start_group(
                region,
                transport,
                tick,
                truncations,
                compactions,
                &mut |_, _| Ok(()),
            ) {
                self.groups
                    .insert(region, LocalGroup::Failed(error.to_string()));
            }
        }
    }

    pub(crate) fn status(&self, region: RegionId) -> Result<NodeStatus> {
        Ok(self.driver(region)?.status())
    }

    /// Adopt a VERIFIED installed generation as this node's runtime replica
    /// of the migration group, and start its driver from the installed base.
    /// One-way: after the durable adoption marker, the joint installer is
    /// closed for this group. The peer starts as whatever the image
    /// configuration says this node is (a learner today); it votes and
    /// serves nothing beyond what that configuration and the read barrier
    /// already enforce. Idempotent across restarts via re-adoption.
    pub(crate) fn adopt_installed(
        &mut self,
        guard: &kv9_common::store_lifecycle::StoreGuard,
        migration: &kv9_meta::data_groups::migration::CommittedMigration,
        transport: &Arc<GrpcTransport>,
        tick: Duration,
        uploader: &kv9_engine::checkpoint::RemoteUploader,
    ) -> Result<()> {
        let region = migration.intent().region();
        match self.groups.get(&region) {
            Some(LocalGroup::Ready(prepared)) if prepared.driver.is_some() => return Ok(()),
            Some(LocalGroup::Retired { .. }) | Some(LocalGroup::Reclaimed { .. }) => return Ok(()),
            _ => {}
        }
        let adopted =
            kv9_raft::snapshot_install::adopt_for_runtime(guard, self.identity, region, uploader)?;
        let creation = migration.creation().intent();
        if adopted.range.creation != creation.digest() || adopted.range.root != creation.root() {
            return Err(invalid(
                "adopted range does not describe the committed creation",
            ));
        }
        let receipt = kv9_meta::data_groups::evidence::InstallEvidence::receipt(
            migration.intent(),
            adopted.generation,
            adopted.image_digest,
            adopted.subject,
            adopted.cut,
        )?;
        let storage = DiskRaftStorage::recover(&adopted.raft_directory)?;
        let (engine, _) =
            WalEngine::open_with_uploader(adopted.engine_wal.clone(), Some(uploader))?;
        engine.set_data_sync_defer(data_sync_defer_bytes());
        let engine = Arc::new(engine);
        let mut state = MemStateMachine::with_engine(engine.clone())?;
        state.set_data_group(adopted.range.root, region, adopted.range.creation)?;
        let peer = Arc::new(RaftPeer::with_installed_storage(
            self.identity.node_id,
            region,
            storage,
            adopted.cut,
        )?);
        let driver = NodeDriver::with_installed_base(
            peer,
            transport.register_group(region)?,
            state,
            adopted.cut,
        )?;
        if self.pool.is_none() {
            self.pool = Some(DriverPool::new(self.data_workers, tick)?);
        }
        self.pool
            .as_ref()
            .expect("pool created above")
            .register(driver.clone())?;
        self.group_handles
            .lock()
            .expect("group handles poisoned")
            .insert(region, (engine.clone(), driver.clone()));
        self.raw_directory
            .insert(crate::runtime::range_api::RawGroup::new(
                adopted.range.clone(),
                engine.clone(),
                driver.clone(),
            ));
        self.groups.insert(
            region,
            LocalGroup::Ready(Box::new(PreparedLocal {
                range_proposal: None,
                observation: GroupPreparation {
                    task: creation.task(),
                    region,
                    intent_digest: creation.digest(),
                },
                phase: Phase::Active,
                intent: creation.clone(),
                storage: None,
                engine,
                driver: Some(driver),
                _lock: adopted.group_lock,
            })),
        );
        self.adoption_receipts
            .lock()
            .expect("receipts poisoned")
            .insert(
                region,
                AdoptionReceipt {
                    receipt,
                    image_digest: adopted.image_digest,
                    cut: adopted.cut,
                },
            );
        Ok(())
    }

    /// Permanently fence this store's replica of a group the committed
    /// removal decision named — this exact node AND incarnation. The driver
    /// stops, the group leaves the raw directory, and a durable Retired
    /// record survives restarts. Storage is kept, never deleted; the group
    /// lock stays held so no other opener can adopt the files. Idempotent.
    pub(crate) fn retire_removed(
        &mut self,
        decision: &kv9_meta::data_groups::removal::RemovalDecision,
    ) -> Result<()> {
        let region = decision.region();
        if decision.source().node != self.identity.node_id
            || decision.source().incarnation != self.identity.store_incarnation
        {
            return Err(invalid("removal decision names a different store"));
        }
        self.retire_locally(region)
    }

    /// A PUBLISHED split retires the sealed parent everywhere it is hosted:
    /// the committed directory (sealed parent binding with covering
    /// children) is the authority, and every replica fences its local copy
    /// through the same durable Retired machinery. Idempotent.
    pub(crate) fn retire_published_parent(&mut self, region: RegionId) -> Result<()> {
        self.retire_locally(region)
    }

    fn retire_locally(&mut self, region: RegionId) -> Result<()> {
        let Some(entry) = self.groups.get(&region) else {
            return Ok(()); // nothing local to fence
        };
        match entry {
            LocalGroup::Retired { .. } | LocalGroup::Reclaimed { .. } => return Ok(()),
            LocalGroup::Failed(_) => return Ok(()), // quarantined already
            LocalGroup::Ready(_) => {}
        }
        let Some(LocalGroup::Ready(prepared)) = self.groups.remove(&region) else {
            unreachable!()
        };
        let intent = prepared.intent.clone();
        if let Some(driver) = prepared.driver.as_ref() {
            driver.stop();
        }
        self.raw_directory.remove(region);
        self.group_handles
            .lock()
            .expect("group handles poisoned")
            .remove(&region);
        let directory = self.directory.join(region.0.to_string());
        publish(
            &directory,
            &Record {
                phase: Phase::Retired,
                node: self.identity.node_id,
                incarnation: self.identity.store_incarnation,
                intent: intent.clone(),
            },
            &mut |_, _| Ok(()),
        )?;
        self.groups.insert(
            region,
            LocalGroup::Retired {
                _lock: prepared._lock,
            },
        );
        Ok(())
    }

    /// Physically delete a RETIRED local group's payload — the engine WAL,
    /// its segment directory and the raft log — under the caller's
    /// re-verified committed authority. The durable record survives as the
    /// permanent fence and is REWRITTEN to `Reclaimed` BEFORE any file is
    /// deleted, so a crash mid-deletion resumes idempotently at discovery.
    /// Returns the payload bytes freed (0 on an idempotent confirmation).
    pub(crate) fn reclaim_retired(&mut self, region: RegionId) -> Result<u64> {
        match self.groups.get(&region) {
            None => return Ok(0), // nothing local to reclaim
            Some(LocalGroup::Reclaimed { .. }) => return Ok(0),
            Some(LocalGroup::Retired { .. }) => {}
            Some(LocalGroup::Ready(_)) => {
                return Err(invalid("reclamation requires the retired fence"));
            }
            Some(LocalGroup::Failed(_)) => {
                return Err(invalid("a quarantined group is never reclaimed"));
            }
        }
        let directory = self.directory.join(region.0.to_string());
        let record = read_record(&directory.join(RECORD))?
            .ok_or_else(|| invalid("retired group record is missing"))?;
        if record.phase != Phase::Retired || record.intent.region() != region {
            return Err(invalid("retired group record differs"));
        }
        // Durable fence first: the Reclaimed record commits the deletion
        // before one byte disappears.
        publish(
            &directory,
            &Record {
                phase: Phase::Reclaimed,
                ..record
            },
            &mut |_, _| Ok(()),
        )?;
        let freed = delete_group_payload(&directory)?;
        let Some(LocalGroup::Retired { _lock }) = self.groups.remove(&region) else {
            unreachable!()
        };
        self.groups.insert(region, LocalGroup::Reclaimed { _lock });
        Ok(freed)
    }

    /// At most one new local activation per reconciliation turn. Terminal
    /// failures stay quarantined until a new manager performs disk recovery.
    pub(crate) fn reconcile_activation(
        &mut self,
        requests: &[kv9_meta::data_groups::activation::CommittedActivation],
        transport: &Arc<GrpcTransport>,
        tick: Duration,
    ) {
        for request in requests {
            let creation = request.creation();
            let intent = creation.intent();
            if intent.root() != self.identity.root_digest
                || !intent.replicas().iter().any(|r| {
                    r.node == self.identity.node_id
                        && r.incarnation == self.identity.store_incarnation
                })
            {
                continue;
            }
            let region = intent.region();
            match self.groups.get(&region) {
                Some(LocalGroup::Failed(_)) => continue,
                Some(LocalGroup::Retired { .. }) | Some(LocalGroup::Reclaimed { .. }) => continue,
                Some(LocalGroup::Ready(p)) if p.driver.is_some() => continue,
                _ => {}
            }
            if let Err(error) = self.activate(creation, transport, tick) {
                self.groups
                    .insert(region, LocalGroup::Failed(error.to_string()));
            }
            break;
        }
    }

    pub(crate) fn status_json(&self) -> String {
        let observations: Vec<_> = self.groups.iter().map(|(region, group)| {
            match group {
                LocalGroup::Failed(error) => serde_json::json!({
                    "region": region.0, "state": "failed", "error": error,
                }),
                LocalGroup::Retired { .. } => serde_json::json!({
                    "region": region.0, "state": "retired",
                }),
                LocalGroup::Reclaimed { .. } => serde_json::json!({
                    "region": region.0, "state": "reclaimed",
                }),
                LocalGroup::Ready(p) => match p.driver.as_ref().map(|d| d.status()) {
                    None => serde_json::json!({"region": region.0, "state": "prepared"}),
                    Some(s) => {
                        // Observability for follower-side compaction: the
                        // group-replicated confirmed floor this replica has
                        // applied (None until the leader publishes it).
                        let confirmed = p
                            .engine
                            .get(kv9_engine::ColumnFamily::Default, kv9_common::data_range::COMPACTION_CONFIRMED_KEY)
                            .ok()
                            .flatten()
                            .filter(|b| b.len() == 16)
                            .map(|b| {
                                serde_json::json!({
                                    "term": u64::from_be_bytes(b[0..8].try_into().unwrap()),
                                    "index": u64::from_be_bytes(b[8..16].try_into().unwrap()),
                                })
                            });
                        serde_json::json!({
                            "region": region.0, "state": if s.fatal.is_some() { "failed" } else { "active" },
                            "role": format!("{:?}", s.role), "term": s.term,
                            "leader": s.leader_id.map(|n| n.0), "committed": s.raft_committed,
                            "log_first_index": s.log_first_index,
                            "engine_applied": s.applied_index,
                            "confirmed_floor": confirmed,
                            "driver_applied": s.driver_applied.map(|p| serde_json::json!({"term": p.term, "index": p.index})),
                            "error": s.fatal,
                        })
                    }
                },
            }
        }).collect();
        serde_json::to_string(&observations).expect("serialize group observations")
    }

    /// Publish serving handles only after the range record is applied in the
    /// target group. Propose at most one initialization per turn and term.
    pub(crate) fn reconcile_ranges(
        &mut self,
        bindings: &[kv9_meta::data_groups::ranges::CommittedRange],
    ) -> Result<()> {
        for binding in bindings {
            let range = binding.range();
            let Some(LocalGroup::Ready(prepared)) = self.groups.get_mut(&range.region) else {
                continue;
            };
            let Some(driver) = &prepared.driver else {
                continue;
            };
            if prepared.intent != *binding.creation().intent() {
                return Err(invalid("range and local creation differ"));
            }
            let status = driver.status();
            if status.fatal.is_some() {
                continue;
            }
            let current = prepared
                .engine
                .get(
                    kv9_engine::ColumnFamily::Default,
                    kv9_common::data_range::RANGE_KEY,
                )?
                .map(|b| kv9_common::data_range::DataRange::decode(&b))
                .transpose()?;
            if let Some(current) = current {
                if current == *range {
                    self.raw_directory
                        .insert(crate::runtime::range_api::RawGroup::new(
                            range.clone(),
                            prepared.engine.clone(),
                            driver.clone(),
                        ));
                } else if !current.may_follow(range) {
                    return Err(invalid(
                        "applied range differs from committed namespace binding",
                    ));
                }
                continue;
            }
            if status.role == kv9_raft::Role::Leader
                && prepared
                    .range_proposal
                    .is_none_or(|p| p.term != status.term)
            {
                match driver.propose(&kv9_raft::Command::DataRange {
                    expected: None,
                    next: range.clone(),
                }) {
                    Ok(at) => prepared.range_proposal = Some(at),
                    Err(Error::NotLeader { .. }) => {}
                    Err(error) => return Err(error),
                }
                break;
            }
        }
        Ok(())
    }

    fn driver(&self, region: RegionId) -> Result<&NodeDriver<DiskRaftStorage, WalEngine>> {
        match self.groups.get(&region) {
            Some(LocalGroup::Ready(p)) => p
                .driver
                .as_deref()
                .ok_or_else(|| invalid("data group is not active")),
            Some(LocalGroup::Failed(cause)) => Err(invalid(cause)),
            Some(LocalGroup::Retired { .. }) => Err(invalid("data group is retired")),
            Some(LocalGroup::Reclaimed { .. }) => Err(invalid("data group is reclaimed")),
            None => Err(invalid("unknown data group")),
        }
    }

    // Test harness access cannot escape through a public runtime API. D02
    // uses separate private RawGroup handles with ordered range/epoch fences.
    #[cfg(test)]
    pub(crate) fn driver_for_tests(
        &self,
        region: RegionId,
    ) -> Result<&NodeDriver<DiskRaftStorage, WalEngine>> {
        self.driver(region)
    }

    pub(crate) fn shutdown(&mut self) {
        self.raw_directory.clear();
        // Join before releasing any group lock or the parent's StoreGuard.
        self.pool.take();
        self.groups.clear();
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
                LocalGroup::Retired { .. } => Err(invalid("data group is retired")),
                LocalGroup::Reclaimed { .. } => Err(invalid("data group is reclaimed")),
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
            if record.phase != Phase::IntentDurable {
                DiskRaftStorage::recover(&directory.join("raft"))
            } else {
                let voters: Vec<_> = intent.replicas().iter().map(|r| r.node.0).collect();
                DiskRaftStorage::open(&directory.join("raft"), &voters).map(|s| s.0)
            }
        })?;
        let engine = operation(PrepareStep::EngineOpen, observe, || {
            let voters: Vec<_> = intent.replicas().iter().map(|r| r.node.0).collect();
            if record.phase == Phase::Active {
                storage.validate_fixed_group(&voters)?;
            } else {
                storage.validate_unstarted_group(&voters)?;
            }
            let path = directory.join("data.wal");
            if record.phase != Phase::IntentDurable
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
                    if record.phase == Phase::Active {
                        match at {
                            Some(at) if position_in_history(&storage, at)? => Ok(()),
                            None if batch.is_empty() => Ok(()),
                            _ => Err(invalid(
                                "active group engine lacks matching committed Raft history",
                            )),
                        }
                    } else if batch.is_empty() && at.is_none() {
                        Ok(())
                    } else {
                        Err(invalid(
                            "unstarted group contains user data or applied history",
                        ))
                    }
                },
            )?;
            match engine.applied_position()? {
                DurableAppliedPosition::AppliedNothing => {}
                DurableAppliedPosition::AppliedThrough(at)
                    if record.phase == Phase::Active && position_in_history(&storage, at)? => {}
                _ => return Err(invalid("group engine has an unauthorized applied position")),
            }
            engine.enable_segmentation()?;
            // AFTER segmentation: the policy lives on the Segmented backing;
            // setting it before the switch lands on the discarded Legacy one.
            engine.set_data_sync_defer(data_sync_defer_bytes());
            Ok(Arc::new(engine))
        })?;
        if record.phase == Phase::IntentDurable {
            publish(
                &directory,
                &Record {
                    phase: Phase::StorageReady,
                    ..record.clone()
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
            phase: if record.phase == Phase::Active {
                Phase::Active
            } else {
                Phase::StorageReady
            },
            intent: intent.clone(),
            storage: Some(storage),
            engine,
            driver: None,
            range_proposal: None,
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
                    if record.phase == Phase::Retired || record.phase == Phase::Reclaimed {
                        // Permanently fenced: hold the lock, open nothing.
                        let lock = OpenOptions::new()
                            .read(true)
                            .write(true)
                            .create(true)
                            .truncate(false)
                            .open(entry.path().join("group-lock"))
                            .map_err(io)?;
                        lock.try_lock().map_err(|e| {
                            invalid(&format!("retired group is already owned: {e}"))
                        })?;
                        if record.phase == Phase::Reclaimed {
                            // The durable Reclaimed record committed the
                            // deletion; a crash mid-deletion left payload
                            // behind — finishing it here is the resume.
                            delete_group_payload(&entry.path())?;
                            self.groups
                                .insert(id, LocalGroup::Reclaimed { _lock: lock });
                        } else {
                            self.groups.insert(id, LocalGroup::Retired { _lock: lock });
                        }
                        return Ok(());
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

    /// The locally RETIRED (not yet reclaimed) regions — reclamation
    /// candidates for the caller to authorize against committed state.
    pub(crate) fn retired_regions(&self) -> Vec<RegionId> {
        self.groups
            .iter()
            .filter_map(|(id, group)| matches!(group, LocalGroup::Retired { .. }).then_some(*id))
            .collect()
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
                        LocalGroup::Retired { .. } => Err(invalid("data group is retired")),
                        LocalGroup::Reclaimed { .. } => Err(invalid("data group is reclaimed")),
                    },
                )
            })
            .collect()
    }
}

impl Drop for RegionManager {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests;
