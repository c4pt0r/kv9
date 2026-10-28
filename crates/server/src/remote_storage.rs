//! Background upload/adoption for the initial single Raft group. This module
//! owns remote I/O; ordered apply owns only descriptor validation and CAS.
use std::path::Path;
use std::sync::{mpsc, Arc};
use std::time::Duration;

use kv9_common::{Error, Result, RootDescriptor, META_REGION_0};
use kv9_engine::checkpoint::{CheckpointManifest, FlushJournal, PendingFlush, RemoteUploader};
use kv9_engine::{MinioConfig, MinioObjectStore, WalEngine};
use kv9_raft::driver::NodeDriver;
use kv9_raft::storage::DiskRaftStorage;
use kv9_raft::Role;
use kv9_region::manifest::{ManifestAttempt, ManifestProposalState, ManifestSeam, SettledManifest};

use crate::checkpoint_retention::{CheckpointExecution, CheckpointLedger, CheckpointOwners};
use crate::node::Node;
mod pending_disk;

pub(crate) struct PreparedRemote {
    directory: std::path::PathBuf,
    pub uploader: Arc<RemoteUploader>,
    journal: FlushJournal,
    interval: Duration,
    cluster: String,
    root: RootDescriptor,
}

/// Validate configuration and pending recovery authority before starting driver
/// or listener threads. An invalid journal never becomes an empty fresh slot.
impl PreparedRemote {
    pub(crate) fn interval(&self) -> Duration {
        self.interval
    }
    #[cfg(test)]
    pub(crate) fn for_test(
        dir: &Path,
        root: &RootDescriptor,
        uploader: Arc<RemoteUploader>,
    ) -> Self {
        let journal = uploader.journal(dir.join("catalog.pending"));
        Self {
            directory: dir.to_path_buf(),
            uploader,
            journal,
            interval: Duration::from_millis(100),
            cluster: root.cluster_id.to_string(),
            root: root.clone(),
        }
    }
}

pub(crate) fn prepare_remote(
    dir: &Path,
    root: &RootDescriptor,
    storage: &DiskRaftStorage,
) -> Result<Option<PreparedRemote>> {
    let cluster = root.cluster_id.to_string();
    let image_budget = Arc::new(kv9_engine::checkpoint_memory::ImageBudget::new(
        image_memory_limit(std::env::var("KV9_CHECKPOINT_IMAGE_BYTES"))?,
    )?);
    let spool_budget = Arc::new(kv9_engine::checkpoint_spool_budget::SpoolBudget::new(
        spool_disk_limit(std::env::var("KV9_CHECKPOINT_SPOOL_BYTES"))?,
    )?);
    let pending_budget = pending_disk::recover(
        dir,
        pending_disk_limit(std::env::var("KV9_CHECKPOINT_PENDING_BYTES"))?,
    )?;
    let journal = FlushJournal::new(dir.join("catalog.pending"))
        .with_image_budget(image_budget.clone())
        .with_pending_budget(pending_budget.clone())
        .with_streaming_objects();
    let pending = journal.load()?;
    let uploader = match std::env::var("KV9_STORAGE") {
        Err(std::env::VarError::NotPresent) if pending.is_none() => return Ok(None),
        Err(std::env::VarError::NotPresent) => {
            return Err(Error::Config(
                "pending flush requires MinIO configuration".into(),
            ))
        }
        Ok(value) if value == "minio" => {
            let rate = object_io_rate(std::env::var("KV9_CHECKPOINT_IO_BYTES_PER_SEC"))?;
            let budget = Arc::new(kv9_engine::object_io::ObjectIoBudget::new(
                rate,
                kv9_engine::checkpoint::MAX_CHECKPOINT_BYTES as u64,
            )?);
            let buffers = Arc::new(kv9_engine::object_memory::BufferBudget::new(
                object_buffer_limit(std::env::var("KV9_OBJECT_BUFFER_BYTES"))?,
            )?);
            Arc::new(
                RemoteUploader::new(Arc::new(MinioObjectStore::connect_with_budgets(
                    MinioConfig::from_env()?,
                    budget,
                    buffers,
                )?))
                .with_image_budget(image_budget)
                .with_pending_budget(pending_budget)
                .with_spool_budget(spool_budget),
            )
        }
        _ => {
            return Err(Error::Config(
                "KV9_STORAGE must be minio when configured".into(),
            ))
        }
    };
    if let Some(pending) = pending {
        let manifest = pending.manifest();
        if manifest.scope.cluster != cluster
            || manifest.scope.region != META_REGION_0.0
            || storage.committed_term(manifest.index)? != manifest.term
        {
            return Err(Error::Engine(
                "pending flush does not belong to this cluster's committed Raft history".into(),
            ));
        }
    }
    let interval = match std::env::var("KV9_FLUSH_INTERVAL_MS") {
        Ok(value) => value
            .parse::<u64>()
            .ok()
            .filter(|n| *n >= 100)
            .ok_or_else(|| {
                Error::Config("KV9_FLUSH_INTERVAL_MS must be an integer >= 100".into())
            })?,
        Err(std::env::VarError::NotPresent) => 5000,
        Err(_) => return Err(Error::Config("invalid KV9_FLUSH_INTERVAL_MS".into())),
    };
    Ok(Some(PreparedRemote {
        directory: dir.to_path_buf(),
        uploader,
        journal,
        interval: Duration::from_millis(interval),
        cluster,
        root: root.clone(),
    }))
}

fn object_buffer_limit(value: std::result::Result<String, std::env::VarError>) -> Result<u64> {
    match value {
        Err(std::env::VarError::NotPresent) => Ok(kv9_engine::object_memory::DEFAULT_BUFFER_BYTES),
        Ok(value) => value
            .parse::<u64>()
            .ok()
            .filter(|n| *n >= 2 * kv9_engine::checkpoint::MAX_CHECKPOINT_BYTES as u64)
            .ok_or_else(|| {
                Error::Config("KV9_OBJECT_BUFFER_BYTES must be an integer >= 100663296".into())
            }),
        Err(_) => Err(Error::Config("invalid KV9_OBJECT_BUFFER_BYTES".into())),
    }
}

fn image_memory_limit(value: std::result::Result<String, std::env::VarError>) -> Result<u64> {
    match value {
        Err(std::env::VarError::NotPresent) => {
            Ok(kv9_engine::checkpoint_memory::DEFAULT_IMAGE_BYTES)
        }
        Ok(value) => value
            .parse::<u64>()
            .ok()
            .filter(|n| *n >= kv9_engine::checkpoint::MAX_CHECKPOINT_BYTES as u64)
            .ok_or_else(|| {
                Error::Config("KV9_CHECKPOINT_IMAGE_BYTES must be an integer >= 50331648".into())
            }),
        Err(_) => Err(Error::Config("invalid KV9_CHECKPOINT_IMAGE_BYTES".into())),
    }
}

fn spool_disk_limit(value: std::result::Result<String, std::env::VarError>) -> Result<u64> {
    match value {
        Err(std::env::VarError::NotPresent) => {
            Ok(kv9_engine::checkpoint_spool_budget::DEFAULT_SPOOL_BYTES)
        }
        Ok(value) => value
            .parse::<u64>()
            .ok()
            .filter(|n| *n >= kv9_engine::checkpoint::MAX_CHECKPOINT_BYTES as u64)
            .ok_or_else(|| {
                Error::Config("KV9_CHECKPOINT_SPOOL_BYTES must be an integer >= 50331648".into())
            }),
        Err(_) => Err(Error::Config("invalid KV9_CHECKPOINT_SPOOL_BYTES".into())),
    }
}

fn pending_disk_limit(value: std::result::Result<String, std::env::VarError>) -> Result<u64> {
    match value {
        Err(std::env::VarError::NotPresent) => {
            Ok(kv9_engine::checkpoint_pending_budget::DEFAULT_PENDING_BYTES)
        }
        Ok(value) => value
            .parse::<u64>()
            .ok()
            .filter(|n| *n >= kv9_engine::checkpoint::MAX_CHECKPOINT_BYTES as u64)
            .ok_or_else(|| {
                Error::Config("KV9_CHECKPOINT_PENDING_BYTES must be an integer >= 50331648".into())
            }),
        Err(_) => Err(Error::Config("invalid KV9_CHECKPOINT_PENDING_BYTES".into())),
    }
}

fn object_io_rate(value: std::result::Result<String, std::env::VarError>) -> Result<u64> {
    match value {
        Err(std::env::VarError::NotPresent) => Ok(64 * 1024 * 1024),
        Ok(value) => value
            .parse::<u64>()
            .ok()
            .filter(|rate| *rate >= 16 * 1024 * 1024)
            .ok_or_else(|| {
                Error::Config(
                    "KV9_CHECKPOINT_IO_BYTES_PER_SEC must be an integer >= 16777216".into(),
                )
            }),
        Err(_) => Err(Error::Config(
            "invalid KV9_CHECKPOINT_IO_BYTES_PER_SEC".into(),
        )),
    }
}

struct CheckpointWorker {
    execution: Option<CheckpointExecution>,
    retire_after: Option<kv9_common::retention::OwnerId>,
    node: Arc<Node<WalEngine>>,
    driver: Arc<NodeDriver<DiskRaftStorage, WalEngine>>,
    seam: ManifestSeam,
    remote: PreparedRemote,
    last_revision: u64,
    in_flight_revision: Option<u64>,
    adopted: u64,
    reported_selection: Option<kv9_common::retention::OwnerToken>,
    retention: Arc<dyn CheckpointLedger>,
}
impl CheckpointWorker {
    fn new(
        node: Arc<Node<WalEngine>>,
        driver: Arc<NodeDriver<DiskRaftStorage, WalEngine>>,
        remote: PreparedRemote,
        retention: Arc<dyn CheckpointLedger>,
    ) -> Result<Self> {
        let seam = ManifestSeam::mint(driver.mint_seam_handle()?);
        Ok(Self {
            execution: None,
            retire_after: None,
            node,
            driver,
            seam,
            remote,
            last_revision: 0,
            in_flight_revision: None,
            adopted: 0,
            reported_selection: None,
            retention,
        })
    }
    fn tick(&mut self, stopped: &mpsc::Receiver<()>) -> Result<()> {
        let execution = self.close_execution();
        let retirement = crate::bootstrap_retention::resume_retirement(
            &self.remote.directory,
            &self.remote.root,
            self.retention.as_ref(),
        );
        // These independent reports each get a turn; both must settle before
        // another candidate can be created. A skipped/unchanged manifest must
        // not strand a persisted bootstrap fence indefinitely.
        let selection = self.report_selection();
        execution.and(retirement).and(selection)?;
        let uploader = self.remote.uploader.clone();
        let executed = uploader
            .with_local_execution_report(|uploader| self.tick_in_execution(stopped, uploader));
        let result = match executed {
            Ok((result, report)) => {
                let observed = if let Some(execution) = &mut self.execution {
                    execution.observe_drain(report)
                } else if report.identity().is_some() {
                    Err(Error::Engine("checkpoint drain has no executor".into()))
                } else {
                    Ok(())
                };
                observed.and(result)
            }
            Err(error) => Err(error),
        };
        self.close_execution().and(result)
    }
    fn report_selection(&mut self) -> Result<()> {
        let Some(manifest) = self.node.meta_raft.store.engine().selected_checkpoint()? else {
            if self.reported_selection.is_some() {
                return Err(Error::Engine(
                    "metadata selection disappeared after report".into(),
                ));
            }
            return Ok(());
        };
        let identity =
            crate::bootstrap_retention::local_identity(&self.remote.root, self.retention.as_ref())?;
        let Some(grant) = crate::bootstrap_retention::selected_grant(
            &self.remote.directory,
            &self.remote.root,
            identity,
            &manifest,
        )?
        else {
            // Legacy selected stores are enrolled by the existing adoption path.
            return Ok(());
        };
        if self.reported_selection == Some(grant.pending) {
            return Ok(());
        }
        let bytes = kv9_meta::retention::encode_request(
            self.remote.root.digest(),
            &kv9_meta::retention::LedgerRequest::RecordCheckpointSelection {
                pending: grant.pending,
                source: identity.node_id,
                store: identity.store_incarnation,
            },
        )?;
        self.retention.apply(bytes)?;
        self.reported_selection = Some(grant.pending);
        Ok(())
    }
    fn close_execution(&mut self) -> Result<()> {
        if let Some(execution) = &self.execution {
            execution.after_local_drain(self.retention.as_ref())?;
            self.execution = None;
        }
        Ok(())
    }
    fn tick_in_execution(
        &mut self,
        stopped: &mpsc::Receiver<()>,
        uploader: &RemoteUploader,
    ) -> Result<()> {
        let engine = self.node.meta_raft.store.engine().clone();
        if self.seam.in_flight(META_REGION_0.0) {
            let state = self
                .seam
                .converge(META_REGION_0.0, Duration::from_secs(1))
                .map_err(seam_error)?;
            return self.settle(state, stopped);
        }
        // A restarted process (or an ambiguous stage fsync) must recover the
        // exact previous identity BEFORE it can mint another attempt.
        if let Some(pending) = self.remote.journal.load()? {
            return self.resume_pending(pending, stopped, uploader);
        }
        // Every replica adopts only a generation that ordered apply installed
        // locally. A staged pending file is never an adoption/reclaim authority.
        let pair = self.driver.manifest_pair(META_REGION_0.0)?;
        if pair.generation > self.adopted {
            if let Some((term, index, bytes)) =
                self.driver.manifest_at(META_REGION_0.0, pair.generation)?
            {
                if CheckpointManifest::is_checkpoint(&bytes) {
                    let manifest = CheckpointManifest::decode(&bytes)?;
                    if manifest.scope.cluster != self.remote.cluster
                        || manifest.scope.region != META_REGION_0.0
                        || manifest.term != term
                        || manifest.index != index
                        || manifest.encode()? != bytes
                    {
                        return Err(Error::Engine("checkpoint history or scope mismatch".into()));
                    }
                    let selected = engine.selected_checkpoint()?;
                    // Re-publication of an older image cannot move this selector
                    // backwards. Do not mint/retire a grant and then retry that
                    // terminal generation forever after a process restart.
                    if selected
                        .as_ref()
                        .is_none_or(|old| old.index <= manifest.index)
                    {
                        crate::bootstrap_retention::before_selection(
                            &self.remote.directory,
                            &self.remote.root,
                            self.retention.as_ref(),
                            selected.as_ref(),
                            &manifest,
                            pair.generation - 1,
                            |predecessor| {
                                let (term, index, bytes) = self
                                    .driver
                                    .manifest_at(META_REGION_0.0, predecessor + 1)?
                                    .ok_or_else(|| {
                                        Error::Engine("bootstrap grant history missing".into())
                                    })?;
                                let m = CheckpointManifest::decode(&bytes)?;
                                if m.term != term || m.index != index || m.encode()? != bytes {
                                    return Err(Error::Engine(
                                        "bootstrap grant history differs".into(),
                                    ));
                                }
                                Ok(m)
                            },
                        )?;
                        if engine.checkpoint_applied(&manifest)? {
                            eprintln!(
                                "node {} checkpoint generation={} through={} adopted",
                                self.node.id.0, pair.generation, manifest.index
                            );
                        }
                        // A declined selection (for example an unpositioned legacy
                        // WAL) remains an admitted candidate. Do not terminally fence
                        // the same attempt that a later retry may still select.
                        if engine.selected_checkpoint()?.as_ref() == Some(&manifest) {
                            let retirement = crate::bootstrap_retention::after_selection(
                                &self.remote.directory,
                                &self.remote.root,
                                self.retention.as_ref(),
                                engine.selected_checkpoint()?.as_ref(),
                            );
                            // Either receipt can be lost; give both reports a
                            // turn without repeating object reads on the retry.
                            let selection = self.report_selection();
                            retirement.and(selection)?;
                        }
                    }
                }
            }
            self.adopted = pair.generation;
        }
        if self.driver.status().role != Role::Leader {
            return Ok(());
        }
        let revision = engine.data_revision();
        if revision == self.last_revision {
            return Ok(());
        }
        let frozen = engine.freeze_with_scope(|view| {
            Ok(
                kv9_meta::checkpoint::inspect_initial_checkpoint_base(view, &self.remote.root)?
                    .flush_scope(),
            )
        })?;
        let frozen_revision = frozen.data_revision();
        let mut planned = uploader.plan_spooled(frozen, &mut self.remote.journal)?;
        // Validate the complete prospective owner before writing a durable plan.
        CheckpointOwners::new(&self.remote.root, planned.manifest(), pair.generation)?;
        self.in_flight_revision = Some(frozen_revision);
        self.remote
            .journal
            .stage_spooled(&mut planned, pair.generation)?;
        drop(planned);
        self.pause_for_test("planned", stopped)?;
        let pending = self
            .remote
            .journal
            .load()?
            .ok_or_else(|| Error::Engine("staged checkpoint plan disappeared".into()))?;
        self.resume_pending(pending, stopped, uploader)
    }

    fn resume_pending(
        &mut self,
        pending: PendingFlush,
        stopped: &mpsc::Receiver<()>,
        uploader: &RemoteUploader,
    ) -> Result<()> {
        let cut = pending.manifest().index;
        let generation = pending.expected_generation();
        let owners = CheckpointOwners::new(&self.remote.root, pending.manifest(), generation)?;
        owners.before_io(self.retention.as_ref())?;
        if let Some(settled) = self
            .seam
            .inspect_checkpoint(pending.manifest(), generation)
            .map_err(seam_error)?
        {
            return self.settle(ManifestProposalState::Settled(settled), stopped);
        }
        eprintln!("node {} checkpoint through={} owners pending={:?} version={:?} confirmed before remote I/O",
            self.node.id.0, cut, owners.pending_id(), owners.version_id());
        self.pause_for_test("owned", stopped)?;
        self.execution = Some(owners.execution(self.retention.as_ref())?);
        self.execution
            .as_mut()
            .unwrap()
            .before_io_scoped(self.retention.as_ref(), uploader)?;
        let (prepared, generation) = pending.recover(uploader)?;
        self.remote.journal.record_uploaded(&prepared, generation)?;
        self.pause_for_test("prepared", stopped)?;
        let attempt = ManifestAttempt::from_prepared(prepared, generation)?;
        eprintln!(
            "node {} checkpoint pending generation={} through={} resuming",
            self.node.id.0, generation, cut
        );
        self.pause_for_test("recovering", stopped)?;
        let state = self
            .seam
            .resume_in_flight(attempt, Duration::from_secs(1))
            .map_err(seam_error)?;
        self.settle(state, stopped)
    }

    fn settle(&mut self, state: ManifestProposalState, stopped: &mpsc::Receiver<()>) -> Result<()> {
        if let ManifestProposalState::Settled(settled) = state {
            if matches!(settled, SettledManifest::WindowRefused { .. }) {
                let pending = self
                    .remote
                    .journal
                    .load()?
                    .ok_or_else(|| Error::Engine("closed checkpoint journal missing".into()))?;
                let owners = CheckpointOwners::new(
                    &self.remote.root,
                    pending.manifest(),
                    pending.expected_generation(),
                )?;
                owners.close_admission(self.retention.as_ref())?;
                self.retire_after =
                    owners.retire_unpublished_page(self.retention.as_ref(), self.retire_after)?;
                if self.retire_after.is_some() {
                    return Ok(());
                }
            }
            if matches!(
                settled,
                SettledManifest::MyChangeApplied { .. }
                    | SettledManifest::AlreadyApplied { .. }
                    | SettledManifest::EffectSettled
            ) {
                self.pause_for_test("applied", stopped)?;
                let pending = self.remote.journal.load()?.ok_or_else(|| {
                    Error::Engine("settled checkpoint journal disappeared".into())
                })?;
                CheckpointOwners::new(
                    &self.remote.root,
                    pending.manifest(),
                    pending.expected_generation(),
                )?
                .after_positive_settlement(self.retention.as_ref())?;
            }
            // Typed negative settlement clears local scheduling only. Its durable
            // owner stays pinned until a later certified abort/history protocol;
            // a local refusal or absent object never releases global ownership.
            // If clearing fails, the on-disk slot remains recoverable. Do not
            // start another attempt just because the in-memory seam cleared.
            self.remote.journal.clear_settled()?;
            eprintln!(
                "node {} checkpoint pending settled {:?}",
                self.node.id.0, settled
            );
            if matches!(
                settled,
                SettledManifest::MyChangeApplied { .. }
                    | SettledManifest::AlreadyApplied { .. }
                    | SettledManifest::EffectSettled
            ) {
                if let Some(revision) = self.in_flight_revision {
                    self.last_revision = revision;
                }
            }
            self.in_flight_revision = None;
        }
        Ok(())
    }

    #[cfg(not(feature = "checkpoint-testing"))]
    fn pause_for_test(&self, _phase: &str, _stopped: &mpsc::Receiver<()>) -> Result<()> {
        Ok(())
    }

    /// Deterministic crash cut; absent from production builds. The actual
    /// journal has already been fsynced before the prepared gate is reached.
    #[cfg(feature = "checkpoint-testing")]
    fn pause_for_test(&self, phase: &str, stopped: &mpsc::Receiver<()>) -> Result<()> {
        let Some(dir) = std::env::var_os("KV9_TESTING_FLUSH_PAUSE_DIR") else {
            return Ok(());
        };
        let gate = std::path::PathBuf::from(dir).join(format!("{}-{phase}", self.node.id.0));
        if !gate.exists() {
            return Ok(());
        }
        std::fs::write(gate.with_extension("arrived"), b"paused")
            .map_err(|e| Error::Engine(e.to_string()))?;
        while gate.exists() {
            match stopped.recv_timeout(Duration::from_millis(5)) {
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                _ => return Err(Error::Engine("checkpoint test gate stopped".into())),
            }
        }
        Ok(())
    }
}
fn seam_error(error: kv9_region::manifest::ManifestSeamError) -> Error {
    Error::Engine(error.to_string())
}

pub(crate) struct RemoteStorage {
    stop: Option<mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
#[cfg(test)]
pub(crate) fn checkpoint_passes_for_test(
    node: Arc<Node<WalEngine>>,
    driver: Arc<NodeDriver<DiskRaftStorage, WalEngine>>,
    remote: PreparedRemote,
    retention: Arc<dyn CheckpointLedger>,
) -> Result<impl FnMut() -> Result<u64> + Send> {
    let mut worker = CheckpointWorker::new(node, driver, remote, retention)?;
    Ok(move || {
        let (_keep, stopped) = mpsc::channel();
        // Lose adoption/report caches while retaining the one seam owned by
        // this live Raft node. A real process reopen also replaces that owner.
        worker.adopted = 0;
        worker.reported_selection = None;
        worker.tick(&stopped)?;
        Ok(worker.adopted)
    })
}
impl RemoteStorage {
    pub fn start(
        node: Arc<Node<WalEngine>>,
        driver: Arc<NodeDriver<DiskRaftStorage, WalEngine>>,
        remote: PreparedRemote,
        budget: Arc<crate::maintenance::Budget>,
        retention: Arc<dyn CheckpointLedger>,
    ) -> Result<Self> {
        let interval = remote.interval;
        let mut worker = CheckpointWorker::new(node, driver, remote, retention)?;
        let (stop, stopped) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("kv9-checkpoint".into())
            .spawn(move || {
                let mut lane = budget.lane(crate::maintenance::Class::MetadataCheckpoint);
                let mut due = std::time::Instant::now() + interval;
                while matches!(
                    stopped.recv_timeout(Duration::from_millis(50)),
                    Err(mpsc::RecvTimeoutError::Timeout)
                ) {
                    if std::time::Instant::now() < due {
                        lane.withdraw();
                        continue;
                    }
                    let Some(_permit) = lane.try_acquire() else {
                        continue;
                    };
                    if let Err(e) = worker.tick(&stopped) {
                        eprintln!("node {} checkpoint deferred: {e}", worker.node.id.0);
                    }
                    due = std::time::Instant::now() + interval;
                }
            })
            .map_err(|e| Error::Engine(format!("spawn checkpoint worker: {e}")))?;
        Ok(Self {
            stop: Some(stop),
            thread: Some(thread),
        })
    }
}
impl Drop for RemoteStorage {
    fn drop(&mut self) {
        self.stop.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pending_disk_configuration_is_finite_and_allows_explicit_larger_limits() {
        assert_eq!(
            pending_disk_limit(Err(std::env::VarError::NotPresent)).unwrap(),
            1024 * 1024 * 1024
        );
        assert_eq!(
            pending_disk_limit(Ok("50331648".into())).unwrap(),
            48 * 1024 * 1024
        );
        assert_eq!(
            pending_disk_limit(Ok(u64::MAX.to_string())).unwrap(),
            u64::MAX
        );
        for value in [
            "",
            "0",
            "-1",
            "50331647",
            " 50331648",
            "18446744073709551616",
        ] {
            assert!(pending_disk_limit(Ok(value.into())).is_err());
        }
    }
    #[test]
    fn spool_configuration_is_bounded_and_validated_without_remote_storage() {
        assert_eq!(
            spool_disk_limit(Err(std::env::VarError::NotPresent)).unwrap(),
            512 * 1024 * 1024
        );
        assert_eq!(
            spool_disk_limit(Ok("50331648".into())).unwrap(),
            48 * 1024 * 1024
        );
        for value in [
            "",
            "0",
            "-1",
            "50331647",
            " 50331648",
            "18446744073709551616",
        ] {
            assert!(spool_disk_limit(Ok(value.into())).is_err());
        }
        assert_eq!(
            spool_disk_limit(Ok(u64::MAX.to_string())).unwrap(),
            u64::MAX
        );
    }
    #[test]
    fn object_buffer_configuration_admits_a_maximum_put_and_comparison_read() {
        assert_eq!(
            object_buffer_limit(Err(std::env::VarError::NotPresent)).unwrap(),
            96 * 1024 * 1024
        );
        assert_eq!(
            object_buffer_limit(Ok("100663296".into())).unwrap(),
            96 * 1024 * 1024
        );
        for value in [
            "",
            "0",
            "-1",
            "100663295",
            " 100663296",
            "18446744073709551616",
        ] {
            assert!(object_buffer_limit(Ok(value.into())).is_err());
        }
        assert_eq!(
            object_buffer_limit(Ok(u64::MAX.to_string())).unwrap(),
            u64::MAX
        );
    }
    #[test]
    fn image_memory_configuration_can_admit_one_maximum_image() {
        assert_eq!(
            image_memory_limit(Err(std::env::VarError::NotPresent)).unwrap(),
            96 * 1024 * 1024
        );
        assert_eq!(
            image_memory_limit(Ok("50331648".into())).unwrap(),
            48 * 1024 * 1024
        );
        for value in [
            "",
            "0",
            "-1",
            "50331647",
            " 50331648",
            "18446744073709551616",
        ] {
            assert!(image_memory_limit(Ok(value.into())).is_err());
        }
        assert_eq!(
            image_memory_limit(Ok(u64::MAX.to_string())).unwrap(),
            u64::MAX
        );
    }
    #[test]
    fn object_rate_configuration_preserves_finite_full_image_admission() {
        assert_eq!(
            object_io_rate(Err(std::env::VarError::NotPresent)).unwrap(),
            64 * 1024 * 1024
        );
        assert_eq!(
            object_io_rate(Ok("16777216".into())).unwrap(),
            16 * 1024 * 1024
        );
        for value in [
            "",
            "0",
            "-1",
            "16777215",
            " 16777216",
            "18446744073709551616",
        ] {
            assert!(object_io_rate(Ok(value.into())).is_err());
        }
        assert_eq!(object_io_rate(Ok(u64::MAX.to_string())).unwrap(), u64::MAX);
    }
}
