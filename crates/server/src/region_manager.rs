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
use kv9_raft::state_machine::checkpoint_recovery::data_group::DataRecoveryDependencies;
use kv9_raft::storage::DiskRaftStorage;
use kv9_raft::{MemStateMachine, RaftPeer};

mod recovery;
pub use recovery::{GroupRecoveryStatus, RecoverySelectionRelation};

const RECORD: &str = "group-record";
const LOCK: &str = "group-lock";
const MAGIC: &[u8; 8] = b"KV9LOC01";
const MAX_RECORD_BYTES: usize = 8 + 1 + 8 + 16 + MAX_INTENT_BYTES + 32;
const MAX_LOCAL_GROUPS: usize = 255;

#[derive(Clone, Copy)]
pub(crate) struct RecoveryDecisions<'a> {
    pub truncations: &'a [kv9_meta::data_groups::truncation::TruncationDecision],
    pub compactions: &'a [kv9_meta::data_groups::compaction::GroupCompaction],
}
impl RecoveryDecisions<'_> {
    /// Validate the actual recovered prefix before constructing any Raft owner.
    pub(crate) fn checked_base(
        self,
        region: RegionId,
        storage: &DiskRaftStorage,
    ) -> Result<Option<kv9_common::AppliedPosition>> {
        let base = storage.compacted_base()?;
        if base.is_some_and(|base| !self.authorizes(region, base)) {
            return Err(invalid(
                "compacted group log lacks a committed truncation decision",
            ));
        }
        Ok(base)
    }

    fn authorizes(self, region: RegionId, base: kv9_common::AppliedPosition) -> bool {
        self.truncations
            .iter()
            .any(|d| d.region() == region && d.floor() == base)
            || self
                .compactions
                .iter()
                .any(|c| c.region() == region && c.floor().index >= base.index)
    }
}

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
    recovery: Option<DataRecoveryDependencies>,
    driver: Option<Arc<NodeDriver<DiskRaftStorage, WalEngine>>>,
    _lock: File,
}

/// Retirement keeps the disk lock until the durable fence, pool owner and
/// previously issued read/capture handles have all drained. Taking the final
/// Arc with try_unwrap runs destruction here, before releasing the lock or
/// deleting files; observing a zero weak count could race with a destructor.
struct RetiringLocal {
    intent: CreationIntent,
    durable: bool,
    driver: Option<Arc<NodeDriver<DiskRaftStorage, WalEngine>>>,
    engine: Option<Arc<WalEngine>>,
    peer: Option<Arc<RaftPeer<DiskRaftStorage>>>,
    _lock: File,
}

enum LocalGroup {
    Ready(Box<PreparedLocal>),
    Recovering(Box<PendingRecovery>),
    Failed(String),
    /// Permanently fenced by committed authority. Pending work retains the
    /// lock; a drained tombstone owns neither payload nor an open lock file.
    Retired {
        draining: Option<Box<RetiringLocal>>,
    },
    /// Fenced AND physically reclaimed: only the record and lock remain.
    Reclaimed {},
}

struct PendingRecovery {
    creation: CommittedCreation,
    error: Option<String>,
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

/// A locally served group's `(intent, engine, driver)`.
pub(crate) type LocalParts = (CreationIntent, Arc<WalEngine>, Arc<NodeDriver<DiskRaftStorage, WalEngine>>);

/// One data group this node leads: `(intent, driver status, peer progress)`.
pub(crate) type LedGroup = (CreationIntent, kv9_raft::driver::NodeStatus, Vec<kv9_raft::rawnode::PeerProgress>);

pub(crate) struct RegionManager {
    pub(crate) raw_directory: Arc<crate::runtime::range_api::RawDirectory>,
    pub(crate) group_handles: GroupHandles,
    /// The same configured, verified object reader used by metadata recovery.
    /// A data checkpoint without remote access refuses recovery, never falls
    /// back to a partial local WAL after its prefix has been reclaimed.
    pub(crate) uploader: Option<Arc<kv9_engine::checkpoint::RemoteUploader>>,
    pub(crate) recovery_retention: Option<Arc<crate::recovery_retention::RecoveryRetention>>,
    pub(crate) data_read_fences: Option<Arc<kv9_common::data_read_fence::DataReadFences>>,
    recovery_cursor: Option<RegionId>,
    adoption_cursor: Option<RegionId>,
    directory: PathBuf,
    identity: StoreIdentity,
    groups: BTreeMap<RegionId, LocalGroup>,
    pub(crate) adoption_receipts: Arc<std::sync::Mutex<BTreeMap<RegionId, AdoptionReceipt>>>,
    pool: Option<DriverPool<DiskRaftStorage, WalEngine>>,
    /// Bounded shared Ready/tick worker count, fixed at construction from the
    /// validated node configuration. The metadata owner is separate.
    data_workers: usize,
    pub(crate) max_write_log_entries: u64,
    pub(crate) log_budget: Arc<kv9_common::log_budget::LogBudget>,
    pub(crate) replica_pressure: Arc<kv9_raft::replica_pressure::ReplicaPressure>,
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
    RetiredFileSync,
    RetiredRename,
    RetiredDirectorySync,
}

fn invalid(message: &str) -> Error {
    Error::Config(message.into())
}
fn io(error: std::io::Error) -> Error {
    Error::Engine(format!("data group storage: {error}"))
}

/// A successful return includes completion of the final destructor. If a
/// previously issued handle is still alive, retain our owner for a later turn.
fn drop_exclusive<T>(owner: &mut Option<Arc<T>>) -> bool {
    match owner.take().map(Arc::try_unwrap) {
        Some(Err(shared)) => {
            *owner = Some(shared);
            false
        }
        Some(Ok(value)) => {
            drop(value);
            true
        }
        None => true,
    }
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
        Phase::Active | Phase::Reclaimed => [
            PrepareStep::ActiveFileSync,
            PrepareStep::ActiveRename,
            PrepareStep::ActiveDirectorySync,
        ],
        Phase::Retired => [
            PrepareStep::RetiredFileSync,
            PrepareStep::RetiredRename,
            PrepareStep::RetiredDirectorySync,
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
fn delete_group_payload(
    directory: &Path,
    uploader: Option<&kv9_engine::checkpoint::RemoteUploader>,
    log_budget: &Arc<kv9_common::log_budget::LogBudget>,
) -> Result<u64> {
    let mut freed = 0u64;
    for name in [
        "data.wal",
        "data.pending",
        "data.pending.tmp",
        "data.checkpoint-image.tmp",
    ] {
        let path = directory.join(name);
        match fs::metadata(&path) {
            Ok(meta) => {
                freed += meta.len();
                fs::remove_file(&path).map_err(io)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io(e)),
        }
    }
    freed += delete_capture_payload(directory, uploader)?;
    // Installed engines keep their retry slot inside the selected generation.
    // The durable Reclaimed fence and drained group owners apply to these
    // files too. Preserve installer records and stable capture lock inodes.
    for entry in fs::read_dir(directory).map_err(io)? {
        let entry = entry.map_err(io)?;
        if entry.file_name().to_string_lossy().starts_with("install-")
            && entry.file_type().map_err(io)?.is_dir()
        {
            freed += delete_capture_payload(&entry.path(), uploader)?;
        }
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
    if let Some(uploader) = uploader {
        uploader.observe_removed_journal(&directory.join("data.pending"))?;
    }
    crate::log_budget::reclaimed(directory, log_budget)?;
    Ok(freed)
}

fn delete_capture_payload(
    directory: &Path,
    uploader: Option<&kv9_engine::checkpoint::RemoteUploader>,
) -> Result<u64> {
    let mut freed = 0;
    for name in [
        "capture.plan",
        "capture.pending.tmp",
        "capture-replica.plan",
        "capture-replica.pending.tmp",
    ] {
        let path = directory.join(name);
        match fs::symlink_metadata(&path) {
            Ok(meta) => {
                fs::remove_file(&path).map_err(io)?;
                freed += meta.len();
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io(e)),
        }
    }
    kv9_common::fs::sync_ancestors(&kv9_common::fs::OsFileSystem, directory).map_err(io)?;
    if let Some(uploader) = uploader {
        uploader.observe_removed_journal(&directory.join("capture.plan"))?;
        uploader.observe_removed_journal(&directory.join("capture-replica.plan"))?;
    }
    Ok(freed)
}

impl RegionManager {
    pub(crate) fn new(directory: &Path, identity: StoreIdentity, data_workers: usize) -> Self {
        Self {
            raw_directory: Arc::default(),
            group_handles: Arc::default(),
            uploader: None,
            recovery_retention: None,
            data_read_fences: None,
            recovery_cursor: None,
            adoption_cursor: None,
            directory: directory.join("data-groups"),
            identity,
            groups: BTreeMap::new(),
            adoption_receipts: Arc::default(),
            pool: None,
            data_workers: data_workers.clamp(1, 32),
            replica_pressure: Arc::new(kv9_raft::replica_pressure::ReplicaPressure::new(identity)),
            max_write_log_entries: crate::write_pressure::DEFAULT_MAX_RAFT_LOG_ENTRIES,
            log_budget: Arc::new(
                kv9_common::log_budget::LogBudget::new(kv9_common::log_budget::DEFAULT_LOG_BYTES)
                    .expect("positive log byte threshold"),
            ),
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
        // A compacted prefix is legitimate ONLY under committed authority: a
        // kind-105 migration truncation decision whose floor EQUALS the base, OR
        // a kind-110 healthy-group compaction floor that reaches the base. A
        // migration truncation is a single exact floor; compaction, by contrast,
        // advances a SEQUENCE of floors, and a replica that lagged (or was down
        // while the group compacted further) recovers with a base at an EARLIER
        // committed floor than the latest — so the base is legitimate as long as
        // a committed compaction floor reaches at or beyond it (that authority
        // covers discarding every entry at or below the base). `compactions`
        // carries the highest committed floor per region. Anything else refuses.
        let installed_base = RecoveryDecisions {
            truncations,
            compactions,
        }
        .checked_base(region, &storage)?;
        prepared.engine.attach_log_budget(&self.log_budget)?;
        let log_debt = storage.attach_log_budget(&self.log_budget)?;
        let peer = Arc::new(match installed_base {
            None => RaftPeer::with_storage(self.identity.node_id, region, storage)?,
            Some(base) => {
                RaftPeer::with_installed_storage(self.identity.node_id, region, storage, base)?
            }
        });
        peer.set_max_write_log_entries(self.max_write_log_entries);
        peer.set_replica_pressure(self.replica_pressure.clone());
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
        driver.attach_log_debt(log_debt)?;
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

    /// One deferred open per maintenance turn, rotating even after a refusal.
    /// Metadata must already be able to commit Reader admission. No group
    /// driver, routing handle or open-engine observation exists while waiting.
    pub(crate) fn resume_recovery_one(&mut self) {
        let region = self
            .groups
            .iter()
            .filter(|(_, g)| matches!(g, LocalGroup::Recovering(_)))
            .map(|(id, _)| *id)
            .min_by_key(|id| {
                (
                    self.recovery_cursor.is_some_and(|cursor| *id <= cursor),
                    *id,
                )
            });
        let Some(region) = region else {
            return;
        };
        self.recovery_cursor = Some(region);
        let Some(LocalGroup::Recovering(pending)) = self.groups.get(&region) else {
            unreachable!()
        };
        let creation = pending.creation.clone();
        match self.open_local(creation.intent(), &mut |_, _| Ok(())) {
            Ok(prepared) => {
                self.groups
                    .insert(region, LocalGroup::Ready(Box::new(prepared)));
            }
            Err(error @ Error::MetaNotReady(_)) => {
                self.groups.insert(
                    region,
                    LocalGroup::Recovering(Box::new(PendingRecovery {
                        creation,
                        error: Some(error.to_string()),
                    })),
                );
            }
            Err(error) => {
                self.groups
                    .insert(region, LocalGroup::Failed(error.to_string()));
            }
        }
    }

    /// Completed/terminal groups do not consume adoption turns; a missing or
    /// temporarily unreadable image cannot starve another destination group.
    pub(crate) fn next_adoption(
        &mut self,
        candidates: impl Iterator<Item = kv9_meta::data_groups::migration::CommittedMigration>,
    ) -> Option<kv9_meta::data_groups::migration::CommittedMigration> {
        let next = candidates
            .filter(|m| match self.groups.get(&m.intent().region()) {
                Some(LocalGroup::Ready(p)) => p.driver.is_none(),
                Some(LocalGroup::Retired { .. }) | Some(LocalGroup::Reclaimed { .. }) => false,
                _ => true,
            })
            .min_by_key(|m| {
                let id = m.intent().region();
                (
                    self.adoption_cursor.is_some_and(|cursor| id <= cursor),
                    id,
                    m.intent().operation(),
                )
            });
        if let Some(next) = &next {
            self.adoption_cursor = Some(next.intent().region());
        }
        next
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
        decisions: RecoveryDecisions<'_>,
    ) -> Result<()> {
        let region = migration.intent().region();
        if migration.intent().root() != self.identity.root_digest
            || migration.intent().destination().node != self.identity.node_id
            || migration.intent().destination().incarnation != self.identity.store_incarnation
        {
            return Err(invalid(
                "migration does not authorize this destination store",
            ));
        }
        match self.groups.get(&region) {
            Some(LocalGroup::Ready(prepared)) if prepared.driver.is_some() => return Ok(()),
            Some(LocalGroup::Retired { .. }) | Some(LocalGroup::Reclaimed { .. }) => return Ok(()),
            _ => {}
        }
        if !self.groups.contains_key(&region) && self.occupied_groups() >= MAX_LOCAL_GROUPS {
            return Err(invalid("local group preparation capacity reached"));
        }
        let image_owner = crate::migration_retention::migration_owner_ids(
            migration.intent().root(),
            migration.intent().operation(),
            &migration.intent().destination().incarnation,
        )?
        .1;
        let reader = self
            .recovery_retention
            .as_ref()
            .map(|r| r.reader(uploader, Some(image_owner)));
        let adopted = kv9_raft::snapshot_install::adopt_for_runtime_with_reader(
            guard,
            self.identity,
            region,
            reader.as_ref().map_or(
                uploader as &dyn kv9_engine::checkpoint::CheckpointRestore,
                |r| r as &dyn kv9_engine::checkpoint::CheckpointRestore,
            ),
        )?;
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
        let mut storage = DiskRaftStorage::recover(&adopted.raft_directory)?;
        let base = storage
            .compacted_base()?
            .ok_or_else(|| invalid("adopted protocol base missing"))?;
        if base != adopted.cut && !decisions.authorizes(region, base) {
            return Err(invalid(
                "adopted log advanced without a committed compaction decision",
            ));
        }
        let (engine, _, recovery) = adopted.open_engine_with_retained_reader(
            &mut storage, uploader,
            reader.as_ref().map_or(uploader as &dyn kv9_raft::state_machine::checkpoint_recovery::data_group::DataCheckpointReader,
                |r| r as &dyn kv9_raft::state_machine::checkpoint_recovery::data_group::DataCheckpointReader),
        )?;
        engine.attach_log_budget(&self.log_budget)?;
        let log_debt = storage.attach_log_budget(&self.log_budget)?;
        engine.set_data_sync_defer(data_sync_defer_bytes());
        let engine = Arc::new(engine);
        let mut state = MemStateMachine::with_engine(engine.clone())?;
        state.set_data_group(adopted.range.root, region, adopted.range.creation)?;
        let peer = Arc::new(RaftPeer::with_installed_storage(
            self.identity.node_id,
            region,
            storage,
            base,
        )?);
        peer.set_max_write_log_entries(self.max_write_log_entries);
        peer.set_replica_pressure(self.replica_pressure.clone());
        let driver =
            NodeDriver::with_installed_base(peer, transport.register_group(region)?, state, base)?;
        driver.attach_log_debt(log_debt)?;
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
                self.raw_directory.clock(),
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
                recovery: Some(recovery),
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
    /// record survives restarts. Storage is kept, never deleted. Repeated
    /// calls drain outstanding owners before releasing the group lock.
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

    pub(crate) fn retire_locally(&mut self, region: RegionId) -> Result<()> {
        self.retire_observed(region, &mut |_, _| Ok(()))
    }

    fn retire_observed(
        &mut self,
        region: RegionId,
        observe: &mut impl FnMut(PrepareStep, bool) -> Result<()>,
    ) -> Result<()> {
        let Some(entry) = self.groups.get(&region) else {
            return Ok(()); // nothing local to fence
        };
        match entry {
            LocalGroup::Recovering(pending) => {
                let intent = pending.creation.intent().clone();
                // Fence retries before any fallible retirement publication.
                self.groups.insert(
                    region,
                    LocalGroup::Failed("retirement requires recovery".into()),
                );
                let directory = self.directory.join(region.0.to_string());
                let lock = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(directory.join(LOCK))
                    .map_err(io)?;
                lock.try_lock()
                    .map_err(|e| invalid(&format!("group is already owned: {e}")))?;
                let record = read_record(&directory.join(RECORD))?
                    .ok_or_else(|| invalid("retiring record missing"))?;
                if record.intent != intent
                    || record.node != self.identity.node_id
                    || record.incarnation != self.identity.store_incarnation
                    || record.phase != Phase::Active
                {
                    return Err(invalid("deferred recovery retirement binding differs"));
                }
                publish(
                    &directory,
                    &Record {
                        phase: Phase::Retired,
                        ..record
                    },
                    observe,
                )?;
                self.groups
                    .insert(region, LocalGroup::Retired { draining: None });
                return Ok(());
            }
            LocalGroup::Retired { .. } => return self.finish_retirement(region, observe),
            LocalGroup::Reclaimed { .. } => return Ok(()),
            LocalGroup::Failed(_) => return Ok(()), // quarantined already
            LocalGroup::Ready(_) => {}
        }
        let Some(LocalGroup::Ready(prepared)) = self.groups.remove(&region) else {
            unreachable!()
        };
        self.raw_directory.remove(region);
        self.group_handles
            .lock()
            .expect("group handles poisoned")
            .remove(&region);
        self.adoption_receipts
            .lock()
            .expect("receipts poisoned")
            .remove(&region);
        if let Some(driver) = prepared.driver.as_ref() {
            driver.stop();
        }
        let retiring = RetiringLocal {
            intent: prepared.intent,
            durable: false,
            engine: Some(prepared.engine),
            peer: prepared.driver.as_ref().map(|d| d.peer().clone()),
            driver: prepared.driver,
            _lock: prepared._lock,
        };
        // An unstarted preparation has no worker and can close its storage
        // directly. Failed publication must retain this pending state and
        // its lock so reconciliation can retry without reopening an old voter.
        drop(prepared.storage);
        self.groups.insert(
            region,
            LocalGroup::Retired {
                draining: Some(Box::new(retiring)),
            },
        );
        self.finish_retirement(region, observe)
    }

    fn finish_retirement(
        &mut self,
        region: RegionId,
        observe: &mut impl FnMut(PrepareStep, bool) -> Result<()>,
    ) -> Result<()> {
        let Some(LocalGroup::Retired { draining }) = self.groups.get_mut(&region) else {
            return Ok(());
        };
        let Some(pending) = draining else {
            return Ok(());
        };
        if !pending.durable {
            publish(
                &self.directory.join(region.0.to_string()),
                &Record {
                    phase: Phase::Retired,
                    node: self.identity.node_id,
                    incarnation: self.identity.store_incarnation,
                    intent: pending.intent.clone(),
                },
                observe,
            )?;
            pending.durable = true;
        }
        if let Some(driver) = &pending.driver {
            let pool = self
                .pool
                .as_ref()
                .ok_or_else(|| invalid("retiring driver has no pool"))?;
            if !pool.try_unregister(driver)? {
                return Ok(());
            }
        }
        // Include the peer because captures can outlive the driver itself.
        if !drop_exclusive(&mut pending.driver)
            || !drop_exclusive(&mut pending.peer)
            || !drop_exclusive(&mut pending.engine)
        {
            return Ok(());
        }
        *draining = None; // closes the lock; the durable tombstone fences reuse
        Ok(())
    }

    /// Physically delete a RETIRED local group's payload — the engine WAL,
    /// its segment directory and the raft log — under the caller's
    /// re-verified committed authority. The durable record survives as the
    /// permanent fence and is REWRITTEN to `Reclaimed` BEFORE any file is
    /// deleted, so a crash mid-deletion resumes idempotently at discovery.
    /// Returns payload bytes freed (0 if already done or owners still drain).
    pub(crate) fn reclaim_retired(&mut self, region: RegionId) -> Result<u64> {
        self.finish_retirement(region, &mut |_, _| Ok(()))?;
        match self.groups.get(&region) {
            None => return Ok(0), // nothing local to reclaim
            Some(LocalGroup::Reclaimed { .. }) => return Ok(0),
            Some(LocalGroup::Retired { draining: Some(_) }) => return Ok(0),
            Some(LocalGroup::Retired { draining: None }) => {}
            Some(LocalGroup::Ready(_)) => {
                return Err(invalid("reclamation requires the retired fence"));
            }
            Some(LocalGroup::Failed(_)) | Some(LocalGroup::Recovering(_)) => {
                return Err(invalid("a quarantined group is never reclaimed"));
            }
        }
        let directory = self.directory.join(region.0.to_string());
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(directory.join(LOCK))
            .map_err(io)?;
        lock.try_lock()
            .map_err(|e| invalid(&format!("retired group is already owned: {e}")))?;
        let record = read_record(&directory.join(RECORD))?
            .ok_or_else(|| invalid("retired group record is missing"))?;
        if !matches!(record.phase, Phase::Retired | Phase::Reclaimed)
            || record.intent.region() != region
        {
            return Err(invalid("retired group record differs"));
        }
        // Durable fence first: the Reclaimed record commits the deletion
        // before one byte disappears.
        if record.phase == Phase::Retired {
            publish(
                &directory,
                &Record {
                    phase: Phase::Reclaimed,
                    ..record
                },
                &mut |_, _| Ok(()),
            )?;
        }
        let freed = delete_group_payload(&directory, self.uploader.as_deref(), &self.log_budget)?;
        self.groups.insert(region, LocalGroup::Reclaimed {});
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
                Some(LocalGroup::Failed(_)) | Some(LocalGroup::Recovering(_)) => continue,
                Some(LocalGroup::Retired { .. }) | Some(LocalGroup::Reclaimed { .. }) => continue,
                Some(LocalGroup::Ready(p)) if p.driver.is_some() => continue,
                _ => {}
            }
            // Capacity refusal is temporary, not evidence of corrupt storage.
            // Keep this committed request eligible after retirement drains.
            if !self.groups.contains_key(&region) && self.occupied_groups() >= MAX_LOCAL_GROUPS {
                break;
            }
            if let Err(error) = self.activate(creation, transport, tick) {
                self.groups
                    .insert(region, LocalGroup::Failed(error.to_string()));
            }
            break;
        }
    }

    /// Every data group this node currently LEADS: its creation intent, the
    /// driver's status and the leader's view of each peer. The repair verdict
    /// (`runtime/replica_repair.rs`) reads this.
    pub(crate) fn led_groups(&self) -> Vec<LedGroup> {
        self.groups
            .values()
            .filter_map(|group| match group {
                LocalGroup::Ready(p) => p.driver.as_ref().map(|d| (p, d)),
                _ => None,
            })
            .filter_map(|(p, d)| {
                let status = d.status();
                if status.role != kv9_raft::Role::Leader || status.fatal.is_some() {
                    return None;
                }
                Some((p.intent.clone(), status, d.peer_progress()))
            })
            .collect()
    }

    /// A locally served data group's parts — creation intent, engine, driver —
    /// whether or not the catalog binds it to a keyspace (a split child is served
    /// before it is routable). The split driver reads this.
    pub(crate) fn local_parts(&self, region: RegionId) -> Option<LocalParts> {
        match self.groups.get(&region) {
            Some(LocalGroup::Ready(p)) => p
                .driver
                .as_ref()
                .filter(|d| d.status().fatal.is_none())
                .map(|d| (p.intent.clone(), p.engine.clone(), d.clone())),
            _ => None,
        }
    }

    pub(crate) fn status_json(&self) -> String {
        let observations: Vec<_> = self.groups.iter().map(|(region, group)| {
            match group {
                LocalGroup::Recovering(p) => serde_json::json!({
                    "region": region.0, "state": "recovering", "error": p.error,
                }),
                LocalGroup::Failed(error) => serde_json::json!({
                    "region": region.0, "state": "failed", "error": error,
                }),
                LocalGroup::Retired { draining } => serde_json::json!({
                    "region": region.0, "state": "retired", "draining": draining.is_some(),
                }),
                LocalGroup::Reclaimed { .. } => serde_json::json!({
                    "region": region.0, "state": "reclaimed",
                }),
                LocalGroup::Ready(p) => match p.driver.as_ref().map(|d| d.status()) {
                    None => serde_json::json!({"region": region.0, "state": "prepared", "recovery_at_open": recovery::at_open_json(p.recovery.as_ref())}),
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
                        // Retained committed-log payload bytes — the signal a
                        // byte-based auto-compaction trigger observes (null if
                        // the peer read fails). O(retained), bounded by
                        // compaction once active.
                        let retained_bytes = p
                            .driver
                            .as_ref()
                            .and_then(|d| d.peer().retained_log_bytes().ok());
                        // On-disk raft.log size (append-only, never shrinks):
                        // `log_file_bytes - retained_log_bytes` is the
                        // compacted-away waste still occupying disk.
                        let log_file_bytes = p.driver.as_ref().map(|d| d.peer().log_file_bytes());
                        serde_json::json!({
                            "region": region.0, "state": if s.fatal.is_some() { "failed" } else { "active" },
                            "role": format!("{:?}", s.role), "term": s.term,
                            "voters": s.voters, "learners": s.learners,
                            "leader": s.leader_id.map(|n| n.0), "committed": s.raft_committed,
                            "log_first_index": s.log_first_index,
                            "log_last_index": s.log_last_index,
                            "max_write_log_entries": s.max_write_log_entries,
                            "retained_log_entries": s.log_last_index.saturating_sub(s.log_first_index.saturating_sub(1)),
                            "engine_applied": s.applied_index,
                            "confirmed_floor": confirmed,
                            "retained_log_bytes": retained_bytes,
                            "log_file_bytes": log_file_bytes,
                            "driver_applied": s.driver_applied.map(|p| serde_json::json!({"term": p.term, "index": p.index})),
                            "error": s.fatal,
                            "recovery_at_open": recovery::at_open_json(p.recovery.as_ref()),
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
                            self.raw_directory.clock(),
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
            Some(LocalGroup::Recovering(_)) => {
                Err(Error::MetaNotReady("data group recovery pending".into()))
            }
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

    /// Stop consensus/capture publication while retaining every group lock.
    /// The runtime drains background jobs before calling final shutdown.
    pub(crate) fn stop_owners(&mut self) {
        self.raw_directory.clear();
        self.group_handles
            .lock()
            .expect("group handles poisoned")
            .clear();
        self.adoption_receipts
            .lock()
            .expect("receipts poisoned")
            .clear();
        // Join before releasing any group lock or the parent's StoreGuard.
        self.pool.take();
    }

    pub(crate) fn shutdown(&mut self) {
        self.stop_owners();
        self.groups.clear();
    }

    fn occupied_groups(&self) -> usize {
        self.groups
            .values()
            .filter(|group| {
                !matches!(
                    group,
                    LocalGroup::Retired { draining: None } | LocalGroup::Reclaimed { .. }
                )
            })
            .count()
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
                LocalGroup::Recovering(_) => {
                    Err(Error::MetaNotReady("data group recovery pending".into()))
                }
                LocalGroup::Retired { .. } => Err(invalid("data group is retired")),
                LocalGroup::Reclaimed { .. } => Err(invalid("data group is reclaimed")),
                LocalGroup::Failed(cause) => Err(invalid(&format!(
                    "group requires recovery after failure: {cause}"
                ))),
            };
        }
        if self.occupied_groups() >= MAX_LOCAL_GROUPS {
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
                // Terminal records remain authoritative even in a new manager
                // that has not run discovery. Never reopen their payload.
                if matches!(record.phase, Phase::Retired | Phase::Reclaimed) {
                    return Err(invalid("data group is permanently retired"));
                }
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
        if let Some(floors) = &self.data_read_fences {
            let selected = WalEngine::checkpoint_reference(directory.join("data.wal"))?;
            kv9_raft::state_machine::checkpoint_recovery::data_group::check_local_read_floor(
                floors,
                kv9_raft::state_machine::checkpoint_recovery::data_group::DataCheckpointIdentity {
                    cluster: self.identity.cluster_id,
                    root: intent.root(),
                    region: intent.region(),
                    creation: intent.digest(),
                },
                selected.as_ref(),
            )?;
        }
        let mut storage = operation(PrepareStep::RaftOpen, observe, || {
            if record.phase != Phase::IntentDurable {
                DiskRaftStorage::recover(&directory.join("raft"))
            } else {
                let voters: Vec<_> = intent.replicas().iter().map(|r| r.node.0).collect();
                DiskRaftStorage::open(&directory.join("raft"), &voters).map(|s| s.0)
            }
        })?;
        let (engine, recovery) = operation(PrepareStep::EngineOpen, observe, || {
            let voters: Vec<_> = intent.replicas().iter().map(|r| r.node.0).collect();
            if record.phase == Phase::Active {
                storage.validate_fixed_group(&voters)?;
            } else {
                storage.validate_unstarted_group(&voters)?;
            }
            let path = directory.join("data.wal");
            if record.phase != Phase::Active
                && self
                    .uploader
                    .as_ref()
                    .map_or_else(
                        || {
                            kv9_engine::checkpoint::FlushJournal::new(
                                path.with_extension("pending"),
                            )
                        },
                        |u| u.journal(path.with_extension("pending")),
                    )
                    .load()?
                    .is_some()
            {
                return Err(invalid("unstarted group has a pending checkpoint"));
            }
            if record.phase != Phase::IntentDurable
                && (!path.is_file() || !path.with_extension("segments").is_dir())
            {
                return Err(invalid("prepared group is missing its engine WAL topology"));
            }
            let (engine, recovery) = if record.phase == Phase::Active {
                let reader = self
                    .recovery_retention
                    .as_ref()
                    .zip(self.uploader.as_ref())
                    .map(|(retention, uploader)| retention.reader(uploader, None));
                let dependency = reader.as_ref().map(|r| r as &dyn kv9_raft::state_machine::checkpoint_recovery::data_group::DataCheckpointReader)
                    .or_else(|| self.uploader.as_deref().map(|u| u as &dyn kv9_raft::state_machine::checkpoint_recovery::data_group::DataCheckpointReader));
                let (engine, _, recovery) = kv9_raft::state_machine::checkpoint_recovery::data_group::open_with_retained_reader(
                    &mut storage,
                    &path,
                    self.uploader.as_deref(),
                    dependency,
                    kv9_raft::state_machine::checkpoint_recovery::data_group::DataCheckpointIdentity {
                        cluster: self.identity.cluster_id,
                        root: intent.root(),
                        region: intent.region(),
                        creation: intent.digest(),
                    },
                )?;
                (engine, Some(recovery))
            } else {
                (
                    WalEngine::open_with_replay_observer(
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
                    )?
                    .0,
                    None,
                )
            };
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
            Ok((Arc::new(engine), recovery))
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
            recovery,
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
        // Historical fences do not consume active execution capacity. The
        // preparation path separately bounds every live/quarantined group.
        for entry in entries {
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
                let record = read_record(&entry.path().join(RECORD))?;
                if let Some(record) = &record {
                    if record.intent.region() != id {
                        return Err(invalid("group directory and record identity differ"));
                    }
                    if record.phase == Phase::Retired || record.phase == Phase::Reclaimed {
                        if record.node != self.identity.node_id
                            || record.incarnation != self.identity.store_incarnation
                            || record.intent.root() != self.identity.root_digest
                        {
                            return Err(invalid("retired record names a different store"));
                        }
                        // Lock only while recovering the fence/deletion. No
                        // payload is opened and no FD is kept per tombstone.
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
                            delete_group_payload(
                                &entry.path(),
                                self.uploader.as_deref(),
                                &self.log_budget,
                            )?;
                            self.groups.insert(id, LocalGroup::Reclaimed {});
                        } else {
                            self.groups
                                .insert(id, LocalGroup::Retired { draining: None });
                        }
                        return Ok(());
                    }
                }
                let creation = creations
                    .iter()
                    .find(|creation| creation.intent().region() == id)
                    .ok_or_else(|| invalid("group creation is absent from applied metadata"))?;
                if self.recovery_retention.is_some()
                    && record.as_ref().is_some_and(|r| r.phase == Phase::Active)
                {
                    let record = record.as_ref().unwrap();
                    if record.intent != *creation.intent()
                        || record.node != self.identity.node_id
                        || record.incarnation != self.identity.store_incarnation
                    {
                        return Err(invalid(
                            "active recovery record differs from creation or store",
                        ));
                    }
                    if self.occupied_groups() >= MAX_LOCAL_GROUPS {
                        return Err(invalid("local group preparation capacity reached"));
                    }
                    self.groups.insert(
                        id,
                        LocalGroup::Recovering(Box::new(PendingRecovery {
                            creation: creation.clone(),
                            error: None,
                        })),
                    );
                    return Ok(());
                }
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
                        LocalGroup::Recovering(_) => {
                            Err(Error::MetaNotReady("data group recovery pending".into()))
                        }
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
