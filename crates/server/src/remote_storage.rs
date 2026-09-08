//! Background upload/adoption for the initial single Raft group. This module
//! owns remote I/O; ordered apply owns only descriptor validation and CAS.
use std::path::Path;
use std::sync::{mpsc, Arc};
use std::time::Duration;

use kv9_common::{Error, Result, META_REGION_0};
use kv9_engine::checkpoint::{CheckpointManifest, FlushJournal, FlushScope, RemoteUploader};
use kv9_engine::{MinioConfig, MinioObjectStore, WalEngine};
use kv9_meta::codec::{memcmp_uint, ColumnValue};
use kv9_meta::schema::{ColumnId, REGIONS_DESC};
use kv9_raft::driver::NodeDriver;
use kv9_raft::storage::DiskRaftStorage;
use kv9_raft::Role;
use kv9_region::manifest::{ManifestAttempt, ManifestProposalState, ManifestSeam, SettledManifest};

use crate::node::Node;

pub(crate) struct PreparedRemote {
    pub uploader: Arc<RemoteUploader>,
    journal: FlushJournal,
    interval: Duration,
    cluster: String,
}

/// Validate configuration and pending recovery authority before starting driver
/// or listener threads. An invalid journal never becomes an empty fresh slot.
pub(crate) fn prepare_remote(
    dir: &Path,
    cluster: String,
    storage: &DiskRaftStorage,
) -> Result<Option<PreparedRemote>> {
    let journal = FlushJournal::new(dir.join("catalog.pending"));
    let pending = journal.load()?;
    let uploader = match std::env::var("KV9_STORAGE") {
        Err(std::env::VarError::NotPresent) if pending.is_none() => return Ok(None),
        Err(std::env::VarError::NotPresent) => {
            return Err(Error::Config(
                "pending flush requires MinIO configuration".into(),
            ))
        }
        Ok(value) if value == "minio" => Arc::new(RemoteUploader::new(Arc::new(
            MinioObjectStore::connect(MinioConfig::from_env()?)?,
        ))),
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
        uploader,
        journal,
        interval: Duration::from_millis(interval),
        cluster,
    }))
}

struct CheckpointWorker {
    node: Arc<Node<WalEngine>>,
    driver: Arc<NodeDriver<DiskRaftStorage, WalEngine>>,
    seam: ManifestSeam,
    remote: PreparedRemote,
    last_revision: u64,
    in_flight_revision: Option<u64>,
    adopted: u64,
}
impl CheckpointWorker {
    fn tick(&mut self, stopped: &mpsc::Receiver<()>) -> Result<()> {
        let engine = self.node.meta_raft.store.engine();
        // Every replica adopts only a generation that ordered apply installed
        // locally. A staged pending file is never an adoption/reclaim authority.
        let pair = self.driver.manifest_pair(META_REGION_0.0)?;
        if pair.generation > self.adopted {
            if let Some((_, _, bytes)) =
                self.driver.manifest_at(META_REGION_0.0, pair.generation)?
            {
                if CheckpointManifest::is_checkpoint(&bytes) {
                    let manifest = CheckpointManifest::decode(&bytes)?;
                    if manifest.scope.cluster != self.remote.cluster {
                        return Err(Error::Engine("checkpoint cluster mismatch".into()));
                    }
                    if engine.checkpoint_applied(&manifest)? {
                        eprintln!(
                            "node {} checkpoint generation={} through={} adopted",
                            self.node.id.0, pair.generation, manifest.index
                        );
                    }
                }
            }
            self.adopted = pair.generation;
        }
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
            let cut = pending.manifest().index;
            let (prepared, generation) = pending.recover(&self.remote.uploader)?;
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
            return self.settle(state, stopped);
        }
        if self.driver.status().role != Role::Leader {
            return Ok(());
        }
        let revision = engine.data_revision();
        if revision == self.last_revision {
            return Ok(());
        }
        let txn = self.node.meta_raft.store.begin()?;
        let Some(region) = txn.get(&REGIONS_DESC, &[memcmp_uint(META_REGION_0.0)])? else {
            return Ok(());
        };
        let uint = |id| match region.value.get(ColumnId(id)) {
            Some(ColumnValue::Uint(n)) => Ok(*n),
            _ => Err(Error::Engine("metadata region epoch is invalid".into())),
        };
        let frozen = engine.freeze(FlushScope {
            cluster: self.remote.cluster.clone(),
            region: META_REGION_0.0,
            conf_ver: uint(5)?,
            version: uint(6)?,
        })?;
        drop(txn);
        let prepared = self.remote.uploader.upload(frozen)?;
        self.remote.journal.stage(&prepared, pair.generation)?;
        self.pause_for_test("prepared", stopped)?;
        self.in_flight_revision = Some(revision);
        let attempt = ManifestAttempt::from_prepared(prepared, pair.generation)?;
        let state = self
            .seam
            .propose_manifest_change(attempt, Duration::from_secs(1))
            .map_err(seam_error)?;
        self.settle(state, stopped)
    }

    fn settle(&mut self, state: ManifestProposalState, stopped: &mpsc::Receiver<()>) -> Result<()> {
        if let ManifestProposalState::Settled(settled) = state {
            if matches!(
                settled,
                SettledManifest::MyChangeApplied { .. }
                    | SettledManifest::AlreadyApplied { .. }
                    | SettledManifest::EffectSettled
            ) {
                self.pause_for_test("applied", stopped)?;
            }
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
impl RemoteStorage {
    pub fn start(
        node: Arc<Node<WalEngine>>,
        driver: Arc<NodeDriver<DiskRaftStorage, WalEngine>>,
        remote: PreparedRemote,
    ) -> Result<Self> {
        let interval = remote.interval;
        let seam = ManifestSeam::mint(driver.mint_seam_handle()?);
        let mut worker = CheckpointWorker {
            node,
            driver,
            seam,
            remote,
            last_revision: 0,
            in_flight_revision: None,
            adopted: 0,
        };
        let (stop, stopped) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("kv9-checkpoint".into())
            .spawn(move || {
                while matches!(
                    stopped.recv_timeout(interval),
                    Err(mpsc::RecvTimeoutError::Timeout)
                ) {
                    if let Err(e) = worker.tick(&stopped) {
                        eprintln!("node {} checkpoint deferred: {e}", worker.node.id.0);
                    }
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
