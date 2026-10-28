//! Actual metadata/data Raft drivers with different leaders, durable format
//! guards, public activation and internal control RPCs over duplex HTTP/2.
use super::*;
use crate::client::{
    ttl::{ScopedTtlClient, TtlOutcome},
    TransportKind,
};
use crate::raw_ttl::{Operation, Reply};
use crate::ttl_clock::{ClockResult, ClockStatus, TtlClock};
use kv9_common::raw_ttl::{ClockSample, CLOCK_KEY};
use kv9_common::store_lifecycle::StoreGuard;
use kv9_common::{BootstrapGeneration, RootVoter};
use kv9_engine::ColumnFamily;
use kv9_raft::transport::{InProcHub, RaftTransport};
use std::sync::atomic::AtomicUsize;
use tokio::sync::mpsc;

struct Clock;
impl TtlClock for Clock {
    fn sample(&self) -> ClockResult<ClockSample> {
        Ok(ClockSample::with_uncertainty(1000, 10).unwrap())
    }
    fn status(&self) -> ClockStatus {
        panic!("not authority")
    }
}
type Driver = Arc<NodeDriver<DiskRaftStorage, WalEngine>>;
fn pump(drivers: &[Driver]) {
    for _ in 0..30 {
        for d in drivers {
            d.step().unwrap();
        }
    }
}
struct Fixture {
    path: PathBuf,
    _guards: Vec<StoreGuard>,
    range: DataRange,
    backends: Vec<Arc<RuntimeBackend>>,
    meta: Vec<Driver>,
    data: Vec<Driver>,
    discoveries: Vec<Arc<RuntimeDiscovery>>,
    owners: Vec<std::thread::JoinHandle<()>>,
    servers: Vec<tokio::task::JoinHandle<()>>,
    public: Vec<tonic::transport::Channel>,
    hooks: Vec<Arc<DirectoryHook>>,
}
struct DirectoryHook {
    membership_reads: AtomicUsize,
    membership_unknown: AtomicBool,
    membership_refuse: AtomicBool,
    membership_change_endpoint: Mutex<Option<Arc<RuntimeBackend>>>,
    data_fence_reads: AtomicUsize,
    data_fence_unknown: AtomicBool,
    data_fence_refuse: AtomicBool,
    data_fence_change_endpoint: Mutex<Option<Arc<RuntimeBackend>>>,
    retention_data_fence_fault: AtomicUsize,

    bootstrap_reads: AtomicUsize,
    bootstrap_unknown: AtomicBool,
    bootstrap_refuse: AtomicBool,
    bootstrap_change_endpoint: Mutex<Option<Arc<RuntimeBackend>>>,
    backend: Arc<RuntimeBackend>,
    reads: AtomicUsize,
    change_on_second: AtomicBool,
    retention_calls: AtomicUsize,
    retention_commits: AtomicUsize,
    retention_catalog_busy: AtomicUsize,
    retention_fail_after: AtomicUsize,
    retention_refuse_from: AtomicUsize,
    // 1 refuses atomic finish before apply; 2 loses its first committed receipt
    // and then refuses retries. Other ledger operations remain available.
    retention_finish_fault: AtomicUsize,
    retention_read_fault: AtomicUsize,
    publisher_enter_fault: AtomicUsize,
    publisher_finish_fault: AtomicUsize,
    publisher_owner_gate: Mutex<Option<PublisherOwnerGate>>,
    retention_selection_fault: AtomicUsize,
    retention_read_gate_fault: AtomicUsize,
    retention_bootstrap_closure_fault: AtomicUsize,
    retention_image_gate_fault: AtomicUsize,
    retention_image_census_fault: AtomicUsize,
    retention_image_publisher_census_fault: AtomicUsize,
    capture_reads: AtomicUsize,
    capture_refuse: AtomicBool,
    capture_backend: Mutex<Option<Arc<RuntimeBackend>>>,
    capture_change_endpoint: Mutex<Option<Arc<RuntimeBackend>>>,
    selection_reads: AtomicUsize,
    selection_unknown: AtomicBool,
    selection_refuse: AtomicBool,
    selection_change_endpoint: Mutex<Option<Arc<RuntimeBackend>>>,
    history_reads: AtomicUsize,
    history_unknown: AtomicBool,
    history_refuse: AtomicBool,
    history_change_endpoint: Mutex<Option<Arc<RuntimeBackend>>>,
}
struct PublisherOwnerGate {
    owner: kv9_common::retention::OwnerId,
    observed: std::sync::mpsc::Sender<()>,
    resume: std::sync::mpsc::Receiver<()>,
}
impl RegistrationBackend for DirectoryHook {
    fn configuration_timeline(
        &self,
        root: RootDigest,
        region: RegionId,
        creation: RootDigest,
        cursor: Option<kv9_raft::storage::configuration_timeline::TimelineCursor>,
        deadline: Instant,
    ) -> Result<Option<kv9_raft::storage::configuration_timeline::TimelinePage>> {
        self.membership_reads.fetch_add(1, Ordering::SeqCst);
        if self.membership_refuse.load(Ordering::SeqCst) {
            return Err(Error::NotImplemented("injected unsupported DATA fence"));
        }
        if self.membership_unknown.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let floor = self
            .backend
            .configuration_timeline(root, region, creation, cursor, deadline)?;
        if let Some(metadata) = self.membership_change_endpoint.lock().unwrap().take() {
            let txn = metadata.node.meta_raft.store.begin()?;
            let endpoint = node_endpoint(&txn, self.backend.node.id)?.unwrap();
            let cluster = kv9_meta::admission::cluster_id(&txn)?.unwrap();
            drop(txn);
            metadata.change_node_endpoint(
                "data-fence-test",
                crate::api::EndpointChange {
                    cluster,
                    node: endpoint.node,
                    incarnation: endpoint.incarnation,
                    expected_address: endpoint.address,
                    expected_generation: endpoint.generation,
                    new_address: endpoint.address,
                },
            )?;
        }
        Ok(floor)
    }
    fn configuration_origins(
        &self,
        root: RootDigest,
        region: RegionId,
        creation: RootDigest,
        cursor: Option<kv9_raft::storage::configuration_origins::OriginCursor>,
        deadline: Instant,
    ) -> Result<Option<kv9_raft::storage::configuration_origins::OriginPage>> {
        self.membership_reads.fetch_add(1, Ordering::SeqCst);
        if self.membership_refuse.load(Ordering::SeqCst) {
            return Err(Error::NotImplemented("injected unsupported DATA fence"));
        }
        if self.membership_unknown.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let floor = self
            .backend
            .configuration_origins(root, region, creation, cursor, deadline)?;
        if let Some(metadata) = self.membership_change_endpoint.lock().unwrap().take() {
            let txn = metadata.node.meta_raft.store.begin()?;
            let endpoint = node_endpoint(&txn, self.backend.node.id)?.unwrap();
            let cluster = kv9_meta::admission::cluster_id(&txn)?.unwrap();
            drop(txn);
            metadata.change_node_endpoint(
                "data-fence-test",
                crate::api::EndpointChange {
                    cluster,
                    node: endpoint.node,
                    incarnation: endpoint.incarnation,
                    expected_address: endpoint.address,
                    expected_generation: endpoint.generation,
                    new_address: endpoint.address,
                },
            )?;
        }
        Ok(floor)
    }
    fn data_membership(
        &self,
        root: RootDigest,
        region: RegionId,
        creation: RootDigest,
        cursor: Option<kv9_raft::storage::membership_history::MembershipCursor>,
        deadline: Instant,
    ) -> Result<Option<kv9_raft::storage::membership_history::MembershipPage>> {
        self.membership_reads.fetch_add(1, Ordering::SeqCst);
        if self.membership_refuse.load(Ordering::SeqCst) {
            return Err(Error::NotImplemented("injected unsupported DATA fence"));
        }
        if self.membership_unknown.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let floor = self
            .backend
            .data_membership(root, region, creation, cursor, deadline)?;
        if let Some(metadata) = self.membership_change_endpoint.lock().unwrap().take() {
            let txn = metadata.node.meta_raft.store.begin()?;
            let endpoint = node_endpoint(&txn, self.backend.node.id)?.unwrap();
            let cluster = kv9_meta::admission::cluster_id(&txn)?.unwrap();
            drop(txn);
            metadata.change_node_endpoint(
                "data-fence-test",
                crate::api::EndpointChange {
                    cluster,
                    node: endpoint.node,
                    incarnation: endpoint.incarnation,
                    expected_address: endpoint.address,
                    expected_generation: endpoint.generation,
                    new_address: endpoint.address,
                },
            )?;
        }
        Ok(floor)
    }

    fn data_read_fence(
        &self,
        root: RootDigest,
        region: RegionId,
        creation: RootDigest,
        deadline: Instant,
    ) -> Result<Option<kv9_common::data_read_fence::ReadFloor>> {
        self.data_fence_reads.fetch_add(1, Ordering::SeqCst);
        if self.data_fence_refuse.load(Ordering::SeqCst) {
            return Err(Error::NotImplemented("injected unsupported DATA fence"));
        }
        if self.data_fence_unknown.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let floor = self
            .backend
            .data_read_fence(root, region, creation, deadline)?;
        if let Some(metadata) = self.data_fence_change_endpoint.lock().unwrap().take() {
            let txn = metadata.node.meta_raft.store.begin()?;
            let endpoint = node_endpoint(&txn, self.backend.node.id)?.unwrap();
            let cluster = kv9_meta::admission::cluster_id(&txn)?.unwrap();
            drop(txn);
            metadata.change_node_endpoint(
                "data-fence-test",
                crate::api::EndpointChange {
                    cluster,
                    node: endpoint.node,
                    incarnation: endpoint.incarnation,
                    expected_address: endpoint.address,
                    expected_generation: endpoint.generation,
                    new_address: endpoint.address,
                },
            )?;
        }
        Ok(floor)
    }

    fn export_migration_capture(
        &self,
        root: RootDigest,
        region: RegionId,
        operation: [u8; 16],
        digest: RootDigest,
        deadline: Instant,
    ) -> Result<kv9_raft::grpc::migration_payload::PayloadExport> {
        let backend = self
            .capture_backend
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| self.backend.clone());
        backend.export_migration_capture(root, region, operation, digest, deadline)
    }
    fn replicate_migration_capture(
        &self,
        root: RootDigest,
        region: RegionId,
        operation: [u8; 16],
        digest: RootDigest,
        source: StoreIdentity,
        deadline: Instant,
    ) -> Result<()> {
        let backend = self
            .capture_backend
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| self.backend.clone());
        backend.replicate_migration_capture(root, region, operation, digest, source, deadline)
    }
    fn migration_capture(
        &self,
        root: RootDigest,
        region: RegionId,
        operation: [u8; 16],
        deadline: Instant,
    ) -> Result<Option<[u8; 32]>> {
        self.capture_reads.fetch_add(1, Ordering::SeqCst);
        if self.capture_refuse.load(Ordering::SeqCst) {
            return Err(Error::NotImplemented(
                "injected capture observation failure",
            ));
        }
        let backend = self
            .capture_backend
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| self.backend.clone());
        let observed = backend.migration_capture(root, region, operation, deadline)?;
        let change = self.capture_change_endpoint.lock().unwrap().take();
        if let Some(metadata) = change {
            let txn = metadata.node.meta_raft.store.begin()?;
            let endpoint = node_endpoint(&txn, self.backend.node.id)?.unwrap();
            let cluster = kv9_meta::admission::cluster_id(&txn)?.unwrap();
            drop(txn);
            metadata.change_node_endpoint(
                "capture-test",
                crate::api::EndpointChange {
                    cluster,
                    node: endpoint.node,
                    incarnation: endpoint.incarnation,
                    expected_address: endpoint.address,
                    expected_generation: endpoint.generation,
                    new_address: endpoint.address,
                },
            )?;
        }
        Ok(observed)
    }

    fn bootstrap_read_fence(
        &self,
        root: RootDigest,
        predecessor: u64,
        deadline: Instant,
    ) -> Result<Option<[u8; 32]>> {
        self.bootstrap_reads.fetch_add(1, Ordering::SeqCst);
        if self.bootstrap_refuse.load(Ordering::SeqCst) {
            return Err(Error::NotImplemented(
                "injected unsupported bootstrap fence",
            ));
        }
        if self.bootstrap_unknown.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let observed = self
            .backend
            .bootstrap_read_fence(root, predecessor, deadline)?;
        if let Some(metadata) = self.bootstrap_change_endpoint.lock().unwrap().take() {
            let txn = metadata.node.meta_raft.store.begin()?;
            let endpoint = node_endpoint(&txn, self.backend.node.id)?.unwrap();
            let cluster = kv9_meta::admission::cluster_id(&txn)?.unwrap();
            drop(txn);
            metadata.change_node_endpoint(
                "bootstrap-fence-test",
                crate::api::EndpointChange {
                    cluster,
                    node: endpoint.node,
                    incarnation: endpoint.incarnation,
                    expected_address: endpoint.address,
                    expected_generation: endpoint.generation,
                    new_address: endpoint.address,
                },
            )?;
        }
        Ok(observed)
    }

    fn checkpoint_selection(
        &self,
        root: RootDigest,
        region: RegionId,
        predecessor: u64,
        deadline: Instant,
    ) -> Result<Option<[u8; 32]>> {
        self.selection_reads.fetch_add(1, Ordering::SeqCst);
        if self.selection_refuse.load(Ordering::SeqCst) {
            return Err(Error::NotImplemented(
                "injected unsupported checkpoint selection",
            ));
        }
        if self.selection_unknown.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let observed = self
            .backend
            .checkpoint_selection(root, region, predecessor, deadline)?;
        if let Some(metadata) = self.selection_change_endpoint.lock().unwrap().take() {
            let txn = metadata.node.meta_raft.store.begin()?;
            let endpoint = node_endpoint(&txn, self.backend.node.id)?.unwrap();
            let cluster = kv9_meta::admission::cluster_id(&txn)?.unwrap();
            drop(txn);
            metadata.change_node_endpoint(
                "selection-test",
                crate::api::EndpointChange {
                    cluster,
                    node: endpoint.node,
                    incarnation: endpoint.incarnation,
                    expected_address: endpoint.address,
                    expected_generation: endpoint.generation,
                    new_address: endpoint.address,
                },
            )?;
        }
        Ok(observed)
    }
    fn checkpoint_history(
        &self,
        root: RootDigest,
        region: RegionId,
        predecessor: u64,
        deadline: Instant,
    ) -> Result<Option<[u8; 32]>> {
        self.history_reads.fetch_add(1, Ordering::SeqCst);
        if self.history_refuse.load(Ordering::SeqCst) {
            return Err(Error::NotImplemented(
                "injected unsupported checkpoint history",
            ));
        }
        if self.history_unknown.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let result = self
            .backend
            .checkpoint_history(root, region, predecessor, deadline)?;
        let change = self.history_change_endpoint.lock().unwrap().take();
        if let Some(metadata) = change {
            let txn = metadata.node.meta_raft.store.begin()?;
            let endpoint = node_endpoint(&txn, self.backend.node.id)?.unwrap();
            let cluster = kv9_meta::admission::cluster_id(&txn)?.unwrap();
            drop(txn);
            metadata.change_node_endpoint(
                "history-test",
                crate::api::EndpointChange {
                    cluster,
                    node: endpoint.node,
                    incarnation: endpoint.incarnation,
                    expected_address: endpoint.address,
                    expected_generation: endpoint.generation,
                    new_address: endpoint.address,
                },
            )?;
        }
        Ok(result)
    }
    fn checkpoint_retention(
        &self,
        root: RootDigest,
        query: &kv9_raft::grpc::checkpoint_retention::LedgerQuery,
        deadline: Instant,
    ) -> Result<kv9_raft::grpc::checkpoint_retention::LedgerReply> {
        use kv9_raft::grpc::checkpoint_retention::{LedgerQuery, LedgerReply};
        let finish = match query {
            LedgerQuery::Apply(bytes) => matches!(
                kv9_meta::retention::decode_request(bytes, root),
                Ok(kv9_meta::retention::LedgerRequest::FinishCheckpointExecution { .. })
            ),
            _ => false,
        };
        let read_finish = match query {
            LedgerQuery::Apply(bytes) => matches!(
                kv9_meta::retention::decode_request(bytes, root),
                Ok(
                    kv9_meta::retention::LedgerRequest::FinishCheckpointRead { .. }
                        | kv9_meta::retention::LedgerRequest::FinishImageRead { .. }
                )
            ),
            _ => false,
        };
        let data_fence_record = matches!(query, LedgerQuery::Apply(bytes)
            if matches!(kv9_meta::retention::decode_request(bytes, root), Ok(kv9_meta::retention::LedgerRequest::RecordDataReadFence { .. })));
        if data_fence_record && self.retention_data_fence_fault.load(Ordering::SeqCst) == 1 {
            return Err(Error::MetaNotReady(
                "injected DATA fence record refusal".into(),
            ));
        }
        let selection_record = match query {
            LedgerQuery::Apply(bytes) => matches!(
                kv9_meta::retention::decode_request(bytes, root),
                Ok(kv9_meta::retention::LedgerRequest::RecordCheckpointSelection { .. })
            ),
            _ => false,
        };
        let read_gate = matches!(query, LedgerQuery::Apply(bytes)
            if matches!(kv9_meta::retention::decode_request(bytes, root),
                Ok(kv9_meta::retention::LedgerRequest::AdvanceMetadataReadGate { .. })));
        if read_gate && self.retention_read_gate_fault.load(Ordering::SeqCst) == 1 {
            return Err(Error::MetaNotReady(
                "injected metadata read gate refusal".into(),
            ));
        }
        let publisher_enter = matches!(query, LedgerQuery::Apply(bytes)
            if matches!(kv9_meta::retention::decode_request(bytes, root),
                Ok(kv9_meta::retention::LedgerRequest::EnterMigrationPublisher { .. })));
        let bootstrap_closure = matches!(query, LedgerQuery::Apply(bytes)
            if matches!(kv9_meta::retention::decode_request(bytes, root),
                Ok(kv9_meta::retention::LedgerRequest::AdvanceBootstrapClosure { .. })));
        if bootstrap_closure
            && self
                .retention_bootstrap_closure_fault
                .load(Ordering::SeqCst)
                == 1
        {
            return Err(Error::MetaNotReady(
                "injected bootstrap closure refusal".into(),
            ));
        }
        let publisher_finish = matches!(query, LedgerQuery::Apply(bytes)
            if matches!(kv9_meta::retention::decode_request(bytes, root),
                Ok(kv9_meta::retention::LedgerRequest::FinishMigrationPublisher { .. })));
        let image_gate = matches!(query, LedgerQuery::Apply(bytes)
            if matches!(kv9_meta::retention::decode_request(bytes, root),
                Ok(kv9_meta::retention::LedgerRequest::CloseMetadataImageReads(_))));
        let image_census = matches!(query, LedgerQuery::Apply(bytes)
            if matches!(kv9_meta::retention::decode_request(bytes, root),
                Ok(kv9_meta::retention::LedgerRequest::AdvanceImageReaderClosure { .. })));
        let publisher_census = matches!(query, LedgerQuery::Apply(bytes)
            if matches!(kv9_meta::retention::decode_request(bytes, root),
                Ok(kv9_meta::retention::LedgerRequest::AdvanceImagePublisherClosure { .. })));
        if publisher_census
            && self
                .retention_image_publisher_census_fault
                .load(Ordering::SeqCst)
                == 1
        {
            return Err(Error::MetaNotReady(
                "injected image Publisher census refusal".into(),
            ));
        }
        if image_census && self.retention_image_census_fault.load(Ordering::SeqCst) == 1 {
            return Err(Error::MetaNotReady(
                "injected image Reader census refusal".into(),
            ));
        }
        if image_gate && self.retention_image_gate_fault.load(Ordering::SeqCst) == 1 {
            return Err(Error::MetaNotReady(
                "injected metadata image gate refusal".into(),
            ));
        }
        if (publisher_enter && self.publisher_enter_fault.load(Ordering::SeqCst) == 1)
            || (publisher_finish && self.publisher_finish_fault.load(Ordering::SeqCst) == 1)
        {
            return Err(Error::MetaNotReady(
                "injected publisher mutation refusal".into(),
            ));
        }
        if matches!(query, LedgerQuery::Apply(_)) {
            let call = self.retention_calls.fetch_add(1, Ordering::SeqCst) + 1;
            let refuse = self.retention_refuse_from.load(Ordering::SeqCst);
            if refuse != 0 && call >= refuse {
                return Err(Error::MetaNotReady(
                    "injected retention admission refusal".into(),
                ));
            }
        }
        if finish && self.retention_finish_fault.load(Ordering::SeqCst) == 1 {
            return Err(Error::MetaNotReady("injected atomic finish refusal".into()));
        }
        if read_finish && self.retention_read_fault.load(Ordering::SeqCst) == 1 {
            return Err(Error::MetaNotReady("injected read finish refusal".into()));
        }
        if selection_record && self.retention_selection_fault.load(Ordering::SeqCst) == 1 {
            return Err(Error::MetaNotReady(
                "injected selection record refusal".into(),
            ));
        }
        let result = self.backend.checkpoint_retention(root, query, deadline).inspect_err(|error| {
            if matches!(error, Error::MetaNotReady(message) if message == "checkpoint retention catalog busy") {
                self.retention_catalog_busy.fetch_add(1, Ordering::SeqCst);
            }
            if !matches!(error, Error::NotLeader { .. }) {
                // Preserve the backend cause before the RPC deliberately folds
                // it into an unknown outcome. Never print command payloads.
                let tag = match query { LedgerQuery::Apply(bytes) => bytes.get(40).copied(), _ => None };
                eprintln!("retention backend failure node={} query={:?} tag={tag:?} remaining_ms={} error={error:?}",
                    self.backend.node.id.0, std::mem::discriminant(query), deadline.saturating_duration_since(Instant::now()).as_millis());
            }
        })?;
        let gate = {
            let mut gate = self.publisher_owner_gate.lock().unwrap();
            if gate
                .as_ref()
                .is_some_and(|g| matches!(query, LedgerQuery::Owner(owner) if *owner == g.owner))
            {
                gate.take()
            } else {
                None
            }
        };
        if let Some(gate) = gate {
            gate.observed.send(()).unwrap();
            gate.resume.recv_timeout(Duration::from_secs(10)).unwrap();
        }
        if matches!(result, LedgerReply::Applied { .. }) {
            for (matches, fault) in [
                (image_gate, &self.retention_image_gate_fault),
                (image_census, &self.retention_image_census_fault),
                (
                    publisher_census,
                    &self.retention_image_publisher_census_fault,
                ),
                (bootstrap_closure, &self.retention_bootstrap_closure_fault),
                (publisher_enter, &self.publisher_enter_fault),
                (publisher_finish, &self.publisher_finish_fault),
            ] {
                if matches
                    && fault
                        .compare_exchange(2, 1, Ordering::SeqCst, Ordering::SeqCst)
                        .is_ok()
                {
                    return Err(Error::MetaNotReady(
                        "injected lost retention mutation receipt".into(),
                    ));
                }
            }
            let call = self.retention_commits.fetch_add(1, Ordering::SeqCst) + 1;
            if read_gate
                && self
                    .retention_read_gate_fault
                    .compare_exchange(2, 1, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
            {
                return Err(Error::MetaNotReady(
                    "injected lost metadata read gate receipt".into(),
                ));
            }
            if data_fence_record
                && self
                    .retention_data_fence_fault
                    .compare_exchange(2, 1, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
            {
                return Err(Error::MetaNotReady(
                    "injected lost DATA fence receipt".into(),
                ));
            }
            if selection_record
                && self
                    .retention_selection_fault
                    .compare_exchange(2, 1, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
            {
                return Err(Error::MetaNotReady(
                    "injected lost selection record receipt".into(),
                ));
            }
            if read_finish
                && self
                    .retention_read_fault
                    .compare_exchange(2, 1, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
            {
                return Err(Error::MetaNotReady(
                    "injected lost read finish receipt".into(),
                ));
            }
            if finish
                && self
                    .retention_finish_fault
                    .compare_exchange(2, 1, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
            {
                return Err(Error::MetaNotReady(
                    "injected lost atomic finish receipt".into(),
                ));
            }
            if call == self.retention_fail_after.load(Ordering::SeqCst) {
                return Err(Error::MetaNotReady(
                    "injected lost retention receipt after real apply".into(),
                ));
            }
        }
        Ok(result)
    }

    fn raw_ttl_directory(
        &self,
        binding: &DataRange,
        ids: &[NodeId],
        deadline: Instant,
    ) -> Result<RawTtlDirectory> {
        let n = self.reads.fetch_add(1, Ordering::SeqCst) + 1;
        if n == 2 && self.change_on_second.load(Ordering::SeqCst) {
            let current = self.backend.node.meta_raft.store.begin().unwrap();
            let e = node_endpoint(&current, NodeId(3)).unwrap().unwrap();
            let cluster = kv9_meta::admission::cluster_id(&current).unwrap().unwrap();
            drop(current);
            self.backend
                .change_node_endpoint(
                    "test",
                    crate::api::EndpointChange {
                        cluster,
                        node: e.node,
                        incarnation: e.incarnation,
                        expected_address: e.address,
                        expected_generation: e.generation,
                        new_address: "127.0.0.1:22303".parse().unwrap(),
                    },
                )
                .unwrap();
        }
        self.backend.raw_ttl_directory(binding, ids, deadline)
    }
    fn register(
        &self,
        node: NodeId,
        addr: &str,
        cluster: ClusterId,
        ticket: &[u8],
        incarnation: StoreIncarnation,
    ) -> std::result::Result<RegistrationReceipt, RegistrationError> {
        self.backend
            .register(node, addr, cluster, ticket, incarnation)
    }
}
impl Fixture {
    async fn new() -> Self {
        Self::with_log_budgets(None).await
    }
    async fn with_log_budgets(
        budgets: Option<Vec<Arc<kv9_common::log_budget::LogBudget>>>,
    ) -> Self {
        let path = std::env::temp_dir().join(format!(
            "kv9-ttl-admin-{}",
            StoreIncarnation::mint().unwrap()
        ));
        let mut guards = Vec::new();
        let mut voters = Vec::new();
        for id in 1..=3 {
            let mut guard = StoreGuard::lock(&path.join(format!("n{id}"))).unwrap();
            let record = guard.prepare(NodeId(id)).unwrap();
            voters.push(RootVoter {
                node_id: NodeId(id),
                addr: format!("127.0.0.1:{}", 22000 + id).parse().unwrap(),
                store_incarnation: record.incarnation,
            });
            guards.push(guard);
        }
        let root = RootDescriptor::new(
            ClusterId::mint().unwrap(),
            BootstrapGeneration::mint().unwrap(),
            voters,
            b"ttl-admin-test",
        )
        .unwrap();
        let mut contexts = Vec::new();
        let mut discoveries = Vec::new();
        for (i, g) in guards.iter_mut().enumerate() {
            let node = NodeId(i as u64 + 1);
            let identity = StoreIdentity::for_voter(&root, node).unwrap();
            g.bind(&root, &identity).unwrap();
            g.activate(&identity).unwrap();
            let fence = g.raw_ttl_format_fence(&root, &identity).unwrap();
            contexts.push(Arc::new(
                ActivationContext::new(root.clone(), fence.clone(), "cluster-test".into())
                    .unwrap()
                    .with_drain_outbox(&path.join(format!("n{}", node.0)))
                    .unwrap(),
            ));
            let discovery = Arc::new(
                RuntimeDiscovery::new(
                    node,
                    true,
                    0,
                    RootWireIdentity {
                        bootstrap_generation: root.bootstrap_generation,
                        root_digest: root.digest(),
                    },
                )
                .with_raw_ttl_format(fence)
                .unwrap(),
            );
            discovery.set_cluster_id(root.cluster_id);
            discovery.authorize_raft();
            discoveries.push(discovery);
        }
        let hub = InProcHub::new();
        let mut meta = Vec::new();
        let mut nodes = Vec::new();
        for id in 1..=3 {
            let engine = Arc::new(
                WalEngine::open(path.join(format!("n{id}/meta.wal")))
                    .unwrap()
                    .0,
            );
            let storage = DiskRaftStorage::open(&path.join(format!("n{id}/meta-raft")), &[1, 2, 3])
                .unwrap()
                .0;
            let debt = budgets.as_ref().map(|budgets| {
                let budget = &budgets[id as usize - 1];
                engine.attach_log_budget(budget).unwrap();
                storage.attach_log_budget(budget).unwrap()
            });
            let peer =
                Arc::new(RaftPeer::with_storage(NodeId(id), META_REGION_0, storage).unwrap());
            nodes.push(Arc::new(
                Node::with_raft_and_engine(
                    NodeId(id),
                    Config::default(),
                    peer.clone(),
                    engine.clone(),
                )
                .unwrap(),
            ));
            let mut state = MemStateMachine::with_engine(engine).unwrap();
            state.set_fence_adjudicator(Arc::new(CatalogFenceAdjudicator::new(
                nodes.last().unwrap().clone(),
            )));
            meta.push(
                NodeDriver::new(
                    peer,
                    Arc::new(hub.endpoint(NodeId(id))) as Arc<dyn RaftTransport>,
                    state,
                )
                .unwrap(),
            );
            if let Some(debt) = debt {
                meta.last().unwrap().attach_log_debt(debt).unwrap();
            }
        }
        meta[1].peer().campaign().unwrap();
        pump(&meta);
        assert_eq!(meta[1].status().role, Role::Leader);
        let initial = nodes[1]
            .build_initial_metadata_command_for_root(&[NodeId(1), NodeId(2), NodeId(3)], &root)
            .unwrap();
        meta[1].propose(&initial).unwrap();
        pump(&meta);
        let mut txn = nodes[1].meta_raft.store.begin().unwrap();
        let creation = kv9_meta::data_groups::plan_empty_group(
            &mut txn,
            [9; 16],
            &[NodeId(1), NodeId(2), NodeId(3)],
        )
        .unwrap();
        let (range, _) = kv9_meta::data_groups::ranges::plan_keyspace(
            &mut txn,
            root.digest(),
            creation.task(),
            "ttl",
            TenantId::DEFAULT,
        )
        .unwrap();
        meta[1]
            .propose(&Command::from_batch(&txn.into_batch()))
            .unwrap();
        pump(&meta);
        for node in &nodes {
            use kv9_meta::BootstrapEvent::*;
            let mut state = node.meta.lock().unwrap();
            state
                .bootstrap
                .on_event(FoundInitialized {
                    cluster_id: root.cluster_id,
                })
                .unwrap();
            state.bootstrap.on_event(Registered).unwrap();
        }
        let data_hub = InProcHub::new();
        let mut data = Vec::new();
        let mut engines = Vec::new();
        for id in 1..=3 {
            let engine = Arc::new(
                WalEngine::open(path.join(format!("n{id}/data.wal")))
                    .unwrap()
                    .0,
            );
            engine.enable_segmentation().unwrap();
            let storage = DiskRaftStorage::open(&path.join(format!("n{id}/data-raft")), &[1, 2, 3])
                .unwrap()
                .0;
            let debt = budgets.as_ref().map(|budgets| {
                let budget = &budgets[id as usize - 1];
                engine.attach_log_budget(budget).unwrap();
                storage.attach_log_budget(budget).unwrap()
            });
            let peer = Arc::new(RaftPeer::with_storage(NodeId(id), range.region, storage).unwrap());
            let mut sm = MemStateMachine::with_engine(engine.clone()).unwrap();
            sm.set_data_group(range.root, range.region, range.creation)
                .unwrap();
            data.push(
                NodeDriver::new(
                    peer,
                    Arc::new(data_hub.endpoint(NodeId(id))) as Arc<dyn RaftTransport>,
                    sm,
                )
                .unwrap(),
            );
            if let Some(debt) = debt {
                data.last().unwrap().attach_log_debt(debt).unwrap();
            }
            engines.push(engine);
        }
        data[0].peer().campaign().unwrap();
        pump(&data);
        assert_eq!(data[0].status().role, Role::Leader);
        data[0]
            .propose(&Command::DataRange {
                expected: None,
                next: range.clone(),
            })
            .unwrap();
        pump(&data);
        let mut backends = Vec::new();
        let mut owners = Vec::new();
        for i in 0..3 {
            let directory = Arc::new(range_api::RawDirectory::with_clock(Some(Arc::new(Clock))));
            directory.insert(range_api::RawGroup::new(
                range.clone(),
                engines[i].clone(),
                data[i].clone(),
                directory.clock(),
            ));
            backends.push(Arc::new(RuntimeBackend {
                maintenance: Arc::default(),
                ttl_activation: Some(contexts[i].clone()),
                raw_directory: directory,
                group_handles: Arc::default(),
                adoption_receipts: Arc::default(),
                node: nodes[i].clone(),
                driver: meta[i].clone(),
                transport: GrpcTransport::new(
                    NodeId(i as u64 + 1),
                    Some("cluster-test".into()),
                    tokio::runtime::Handle::current(),
                    root.digest(),
                ),
                endpoint_ready: Arc::new(AtomicBool::new(true)),
                initial_voters: root.voters.iter().map(|v| (v.node_id, v.addr)).collect(),
                uploader: None,
            }));
            owners.push(meta[i].spawn(Duration::from_millis(5)).unwrap());
            owners.push(data[i].spawn(Duration::from_millis(5)).unwrap());
        }
        let mut servers = Vec::new();
        let mut public = Vec::new();
        let mut hooks = Vec::new();
        for i in 0..3 {
            let auth = Arc::new(ClusterAuthenticator {
                expected_token: Arc::from("cluster-test"),
                voters: Arc::new([NodeId(1), NodeId(2), NodeId(3)].into_iter().collect()),
                node: nodes[i].clone(),
                driver: meta[i].clone(),
                catchup: Arc::default(),
            });
            let hook = Arc::new(DirectoryHook {
                membership_reads: AtomicUsize::new(0),
                membership_unknown: AtomicBool::new(false),
                membership_refuse: AtomicBool::new(false),
                membership_change_endpoint: Mutex::new(None),
                data_fence_reads: AtomicUsize::new(0),
                data_fence_unknown: AtomicBool::new(false),
                data_fence_refuse: AtomicBool::new(false),
                data_fence_change_endpoint: Mutex::new(None),
                retention_data_fence_fault: AtomicUsize::new(0),
                bootstrap_reads: AtomicUsize::new(0),
                bootstrap_unknown: AtomicBool::new(false),
                bootstrap_refuse: AtomicBool::new(false),
                bootstrap_change_endpoint: Mutex::default(),
                backend: backends[i].clone(),
                reads: AtomicUsize::new(0),
                change_on_second: AtomicBool::new(false),
                retention_calls: AtomicUsize::new(0),
                retention_commits: AtomicUsize::new(0),
                retention_catalog_busy: AtomicUsize::new(0),
                retention_fail_after: AtomicUsize::new(0),
                retention_refuse_from: AtomicUsize::new(0),
                retention_finish_fault: AtomicUsize::new(0),
                retention_read_fault: AtomicUsize::new(0),
                publisher_enter_fault: AtomicUsize::new(0),
                publisher_finish_fault: AtomicUsize::new(0),
                publisher_owner_gate: Mutex::default(),
                retention_selection_fault: AtomicUsize::new(0),
                retention_read_gate_fault: AtomicUsize::new(0),
                retention_bootstrap_closure_fault: AtomicUsize::new(0),
                retention_image_gate_fault: AtomicUsize::new(0),
                retention_image_census_fault: AtomicUsize::new(0),
                retention_image_publisher_census_fault: AtomicUsize::new(0),
                capture_reads: AtomicUsize::new(0),
                capture_refuse: AtomicBool::new(false),
                capture_backend: Mutex::default(),
                capture_change_endpoint: Mutex::default(),
                selection_reads: AtomicUsize::new(0),
                selection_unknown: AtomicBool::new(false),
                selection_refuse: AtomicBool::new(false),
                selection_change_endpoint: Mutex::default(),
                history_reads: AtomicUsize::new(0),
                history_unknown: AtomicBool::new(false),
                history_refuse: AtomicBool::new(false),
                history_change_endpoint: Mutex::default(),
            });
            hooks.push(hook.clone());
            let raft = RaftGrpcService::new(
                NodeId(i as u64 + 1),
                kv9_raft::work::RaftInbox::default(),
                discoveries[i].clone(),
            )
            .with_registration(hook);
            let raft = match &budgets {
                Some(budgets) => raft.with_log_budget(budgets[i].clone()),
                None => raft,
            };
            let raft = Kv9RaftServer::with_interceptor(raft, AuthInterceptor::new(auth));
            let api = Kv9Grpc::new(backends[i].clone());
            let public_service = api
                .clone()
                .authenticated_service(crate::point_test_support::authenticator());
            let point_stream = crate::point_stream::service(
                api,
                crate::point_test_support::authenticator(),
                tokio_util::sync::CancellationToken::new(),
            );
            let (incoming, receive) = mpsc::channel(8);
            servers.push(tokio::spawn(async move {
                tonic::transport::Server::builder()
                    .add_service(public_service)
                    .add_service(point_stream)
                    .add_service(raft)
                    .serve_with_incoming(tokio_stream::wrappers::ReceiverStream::new(receive))
                    .await
                    .unwrap();
            }));
            let channel = tonic::transport::Endpoint::from_static("http://memory.test")
                .connect_with_connector(tower::service_fn(move |_| {
                    let incoming = incoming.clone();
                    async move {
                        let (c, s) = tokio::io::duplex(65536);
                        incoming
                            .send(Ok::<_, std::io::Error>(s))
                            .await
                            .map_err(|_| std::io::Error::from(std::io::ErrorKind::BrokenPipe))?;
                        Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(c))
                    }
                }))
                .await
                .unwrap();
            for context in &contexts {
                context
                    .channels
                    .lock()
                    .unwrap()
                    .insert(root.voters[i].addr, channel.clone());
            }
            public.push(channel);
        }
        Self {
            path,
            _guards: guards,
            range,
            backends,
            meta,
            data,
            discoveries,
            owners,
            servers,
            public,
            hooks,
        }
    }
    fn client(&self, index: usize) -> ScopedTtlClient {
        ScopedTtlClient::with_test_transport(
            self.range.clone(),
            TransportKind::TonicUnary,
            self.public[index].clone(),
            5000,
        )
    }
    fn enabled(&self) -> bool {
        self.backends[0]
            .raw_directory
            .by_region(self.range.region)
            .unwrap()
            .capture_parts()
            .1
            .get(ColumnFamily::Default, CLOCK_KEY)
            .unwrap()
            .is_some()
    }
    async fn shutdown_keep(self) -> PathBuf {
        self.shutdown_inner(true).await
    }
    async fn shutdown_inner(mut self, wait_for_store_owners: bool) -> PathBuf {
        let floors: Vec<_> = self
            .backends
            .iter()
            .filter_map(|b| {
                let context = b.ttl_context().unwrap();
                // Close test-only duplex cycles even for a store whose guard was
                // explicitly handed to a separate offline owner by the test.
                context.channels.lock().unwrap().clear();
                let identity = context.local.identity();
                self._guards
                    .iter()
                    .any(|g| {
                        g.record().is_some_and(|r| {
                            r.node_id == identity.node_id
                                && r.incarnation == identity.store_incarnation
                        })
                    })
                    .then(|| Arc::downgrade(&context.data_read_fences))
            })
            .collect();
        for s in self.servers.drain(..) {
            s.abort();
            let _ = s.await;
        }
        for d in self.meta.iter().chain(self.data.iter()) {
            d.stop();
        }
        for o in self.owners.drain(..) {
            o.join().unwrap();
        }
        let path = self.path.clone();
        drop(self);
        if wait_for_store_owners {
            tokio::time::timeout(Duration::from_secs(5), async {
                while floors.iter().any(|f| f.strong_count() != 0) {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .expect("fixture still has live store owners after shutdown");
        }
        path
    }
    async fn close(self) {
        std::fs::remove_dir_all(self.shutdown_inner(false).await).unwrap();
    }
}

mod replica_pressure;
impl Drop for Fixture {
    fn drop(&mut self) {
        for s in &self.servers {
            s.abort();
        }
        for d in self.meta.iter().chain(self.data.iter()) {
            d.stop();
        }
        for o in self.owners.drain(..) {
            let _ = o.join();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn public_activation_uses_remote_metadata_leader_and_every_real_store_fence() {
    let f = Fixture::new().await;
    assert_eq!(f.meta[1].status().role, Role::Leader);
    assert_eq!(f.data[0].status().role, Role::Leader);
    assert!(!f.enabled());
    // Even with a writable data quorum, the third store's authority is required.
    f.discoveries[2].raft_receive.store(false, Ordering::SeqCst);
    let before = f.data[0].driver_applied();
    let failed = f.client(0).activate().await;
    assert!(
        !matches!(failed.outcome, TtlOutcome::Success { .. }),
        "{failed:?}"
    );
    assert!(!f.enabled());
    assert_eq!(f.data[0].driver_applied(), before);
    f.discoveries[2].authorize_raft();
    let result = f.client(0).activate().await;
    let TtlOutcome::Success {
        reply: Reply::Applied { term, index },
    } = result.outcome
    else {
        panic!("{result:?}")
    };
    assert!(term > 0 && index > before.unwrap().index);
    assert!(f.enabled());
    assert_eq!(result.attempts.len(), 1);
    assert!(
        f.hooks[1].reads.load(Ordering::SeqCst) >= 3,
        "both directory snapshots must use the remote metadata leader"
    );
    let result = f.client(0).activate().await;
    let TtlOutcome::Success {
        reply: Reply::Applied {
            index: confirmation,
            ..
        },
    } = result.outcome
    else {
        panic!("{result:?}")
    };
    assert!(
        confirmation > index,
        "repeat activation is a NEW confirmation, never an original receipt"
    );
    let c = f.client(0);
    assert!(matches!(
        c.call(Operation::Put {
            pairs: vec![(b"k".to_vec(), b"v".to_vec())],
            ttl_secs: Some(0)
        })
        .await
        .outcome,
        TtlOutcome::Success { .. }
    ));
    assert_eq!(
        c.call(Operation::Get {
            keys: vec![b"k".to_vec()]
        })
        .await
        .outcome,
        TtlOutcome::Success {
            reply: Reply::Values { values: vec![None] }
        }
    );
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn endpoint_generation_change_during_collection_prevents_enable() {
    let f = Fixture::new().await;
    f.hooks[1].change_on_second.store(true, Ordering::SeqCst);
    let before = f.data[0].driver_applied();
    let result = f.client(0).activate().await;
    assert!(
        !matches!(result.outcome, TtlOutcome::Success { .. }),
        "{result:?}"
    );
    assert!(!f.enabled());
    assert_eq!(f.data[0].driver_applied(), before);
    assert_eq!(f.hooks[1].reads.load(Ordering::SeqCst), 2);
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn public_activation_redirects_from_metadata_leader_to_data_leader() {
    let f = Fixture::new().await;
    let client = ScopedTtlClient::with_test_channels(
        f.range.clone(),
        TransportKind::TonicStream,
        vec![(2, f.public[1].clone()), (1, f.public[0].clone())],
        5000,
    );
    let before = f.data[0].driver_applied().unwrap();
    let result = client.activate().await;
    assert!(
        matches!(result.outcome, TtlOutcome::Success { .. }),
        "{result:?}"
    );
    assert_eq!(
        result
            .attempts
            .iter()
            .map(|a| a.node_id)
            .collect::<Vec<_>>(),
        vec![2, 1]
    );
    assert_eq!(
        result.attempts[0].failure,
        Some(crate::client::ttl::Failure::Rpc {
            reason: crate::client::Reason::NotLeader { leader: Some(1) },
        })
    );
    assert_eq!(f.data[0].driver_applied().unwrap().index, before.index + 1);
    assert!(f.enabled());
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn republishing_the_same_live_owner_does_not_invalidate_collection() {
    let f = Fixture::new().await;
    let group = f.backends[0]
        .raw_directory
        .by_region(f.range.region)
        .unwrap();
    let (driver, engine, binding) = group.capture_parts();
    let directory = f.backends[0].raw_directory.clone();
    let driver = driver.clone();
    let engine = engine.clone();
    let binding = binding.clone();
    let replace = std::thread::spawn(move || {
        for _ in 0..80 {
            directory.insert(range_api::RawGroup::new(
                binding.clone(),
                engine.clone(),
                driver.clone(),
                directory.clock(),
            ));
            std::thread::sleep(Duration::from_millis(1));
        }
    });
    let result = f.client(0).activate().await;
    replace.join().unwrap();
    assert!(
        matches!(result.outcome, TtlOutcome::Success { .. }),
        "{result:?}"
    );
    assert!(f.enabled());
    let replacement = f.backends[0]
        .raw_directory
        .by_region(f.range.region)
        .unwrap();
    assert!(!Arc::ptr_eq(&replacement, &group));
    assert!(replacement.same_owner(&group));
    drop(group);
    drop(replacement);
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn routed_ttl_discovers_real_metadata_and_uses_the_activated_three_replica_group() {
    use crate::client::routed::{
        ttl::{Outcome, Phase, RoutedTtlClient},
        RoutedConfig,
    };
    let f = Fixture::new().await;
    assert!(matches!(
        f.client(0).activate().await.outcome,
        TtlOutcome::Success { .. }
    ));
    for transport in [TransportKind::TonicUnary, TransportKind::TonicStream] {
        let peers = f.backends[0]
            .initial_voters
            .iter()
            .map(|(n, a)| crate::client::Peer {
                node_id: n.0,
                address: *a,
            })
            .collect::<Vec<_>>();
        let c = RoutedTtlClient::new_with_transport(
            RoutedConfig {
                version: 1,
                root_digest: *f.range.root.as_bytes(),
                tenant_id: f.range.tenant.0,
                keyspace_id: f.range.keyspace.0,
                seeds: peers[..2].to_vec(),
                max_in_flight: 4,
                max_attempts: 12,
                deadline_ms: 5000,
                probe_timeout_ms: 500,
                retry_backoff_ms: 1,
                cache_capacity: 4,
            },
            "secret",
            transport,
        )
        .unwrap();
        for (i, p) in peers.iter().enumerate() {
            c.inject_test_channel(p.address, f.public[i].clone());
        }
        let report = c
            .call(Operation::Put {
                pairs: vec![(b"route".to_vec(), b"v".to_vec())],
                ttl_secs: Some(60),
            })
            .await;
        assert!(
            matches!(
                report.outcome,
                Outcome::Success {
                    reply: Reply::Applied { .. }
                }
            ),
            "{report:?}"
        );
        assert_eq!(
            report
                .attempts
                .iter()
                .filter(|a| a.phase == Phase::Lookup)
                .map(|a| a.node_id)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(
            report
                .attempts
                .iter()
                .filter(|a| a.phase == Phase::Data)
                .map(|a| a.node_id)
                .collect::<Vec<_>>(),
            vec![1]
        );
        assert_eq!(
            c.call(Operation::Get {
                keys: vec![b"route".to_vec(), b"route".to_vec()]
            })
            .await
            .outcome,
            Outcome::Success {
                reply: Reply::Values {
                    values: vec![Some(b"v".to_vec()), Some(b"v".to_vec())]
                }
            }
        );
        assert!(matches!(
            c.call(Operation::Put {
                pairs: vec![(b"route".to_vec(), b"v2".to_vec())],
                ttl_secs: Some(0)
            })
            .await
            .outcome,
            Outcome::Success { .. }
        ));
        let page = c
            .call(Operation::Scan {
                start: vec![],
                end: vec![],
                limit: 16,
            })
            .await;
        assert!(
            matches!(page.outcome, Outcome::Success { reply: Reply::Page { ref rows, next: None, examined: 1 } } if rows.is_empty()),
            "{page:?}"
        );
        assert!(page.continuation.is_none());
        let deleted = c
            .call(Operation::DeleteRange {
                start: vec![],
                end: vec![],
                limit: 16,
            })
            .await;
        let Outcome::Success {
            reply:
                Reply::Deleted {
                    processed_keys: 1,
                    applied: Some(at),
                    next: None,
                },
        } = deleted.outcome
        else {
            panic!("{deleted:?}")
        };
        tokio::time::timeout(Duration::from_secs(5), async {
            while f
                .data
                .iter()
                .any(|d| d.driver_applied().is_none_or(|p| p.index < at.index))
            {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        let physical = kv9_common::codec::encode_key(
            kv9_common::codec::KeyMode::Raw,
            f.range.keyspace,
            b"route",
        )
        .unwrap();
        for backend in &f.backends {
            let group = backend.raw_directory.by_region(f.range.region).unwrap();
            for cf in [ColumnFamily::Default, ColumnFamily::Write] {
                assert!(group
                    .capture_parts()
                    .1
                    .get(cf, &physical)
                    .unwrap()
                    .is_none());
            }
        }
    }
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn background_maintenance_cleans_all_three_replicas_after_real_format_activation() {
    use crate::runtime::ttl_maintenance::Maintenance;
    let f = Fixture::new().await;
    assert!(matches!(
        f.client(0).activate().await.outcome,
        TtlOutcome::Success { .. }
    ));
    for (i, transport) in [TransportKind::TonicUnary, TransportKind::TonicStream]
        .into_iter()
        .enumerate()
    {
        let client = ScopedTtlClient::with_test_transport(
            f.range.clone(),
            transport,
            f.public[0].clone(),
            5000,
        );
        assert!(matches!(
            client
                .call(Operation::Put {
                    pairs: vec![(vec![b'a' + i as u8], b"expired".to_vec())],
                    ttl_secs: Some(0),
                })
                .await
                .outcome,
            TtlOutcome::Success { .. }
        ));
    }
    let tasks = f
        .backends
        .iter()
        .map(|backend| Maintenance::start(backend.clone(), &tokio::runtime::Handle::current()))
        .collect::<Vec<_>>();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let cleared = f.backends.iter().all(|backend| {
                let group = backend.raw_directory.by_region(f.range.region).unwrap();
                [b'a', b'b'].into_iter().all(|key| {
                    let key = kv9_common::codec::encode_key(
                        kv9_common::codec::KeyMode::Raw,
                        f.range.keyspace,
                        &[key],
                    )
                    .unwrap();
                    [ColumnFamily::Default, ColumnFamily::Write]
                        .into_iter()
                        .all(|cf| group.capture_parts().1.get(cf, &key).unwrap().is_none())
                })
            });
            if cleared
                && tasks[0].snapshot().candidates_applied == 2
                && tasks[1..].iter().all(|t| t.snapshot().skipped > 0)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(tasks[0].snapshot().failures, 0);
    for task in &tasks {
        task.cancel();
    }
    for task in tasks {
        task.drain().await;
    }
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn route_lookup_does_not_borrow_a_leader_from_another_group_in_the_keyspace() {
    let f = Fixture::new().await;
    let backend = f.backends[1].clone(); // metadata leader
    let group = backend.raw_directory.by_region(f.range.region).unwrap();
    let (driver, engine, _) = group.capture_parts();
    let mut other_binding = f.range.clone();
    other_binding.region.0 += 1;
    // Inject an unrelated directory wrapper to exercise lookup selection. This
    // is not a fixture for creating or transferring the other group itself.
    backend.raw_directory.remove(f.range.region);
    backend.raw_directory.insert(range_api::RawGroup::new(
        other_binding,
        engine.clone(),
        driver.clone(),
        None,
    ));
    let range = f.range.clone();
    let observation = tokio::task::spawn_blocking(move || {
        backend.lookup_raw_route(range.root, range.tenant, range.keyspace, b"a")
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(observation.range, Some(f.range.clone()));
    assert_eq!(observation.data_leader, None);
    drop(group);
    f.close().await;
}

mod checkpoint_retention;
mod checkpoint_sweep;
mod executor_owners;
mod split_population;

mod data_checkpoint;

mod migration_capture;
mod ttl_split;
