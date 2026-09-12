//! Best-effort, fixed-inventory local metrics. Never a source of authority.
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kv9_common::metrics::{NamedLatency, WalIoMetrics, BUCKETS};
use kv9_raft::driver::{ApplyLagObservation, NodeDriver};
use serde::Serialize;

const INTERVAL: Duration = Duration::from_secs(1);
pub(crate) const MAX_EXPORT_BYTES: usize = 512 * 1024;
pub(crate) const METRIC_COUNT: usize = 18 + kv9_raft::driver::DriverMetrics::COUNT;

#[derive(Serialize)]
struct ApplyLag {
    term_before: u64,
    term_after: u64,
    committed_before: u64,
    committed_after: u64,
    driver_applied_term: Option<u64>,
    driver_applied_index: Option<u64>,
    lag_entries: Option<u64>,
}

impl From<ApplyLagObservation> for ApplyLag {
    fn from(value: ApplyLagObservation) -> Self {
        Self {
            term_before: value.term_before,
            term_after: value.term_after,
            committed_before: value.committed_before,
            committed_after: value.committed_after,
            driver_applied_term: value.driver_applied.map(|p| p.term),
            driver_applied_index: value.driver_applied.map(|p| p.index),
            lag_entries: value.lag(),
        }
    }
}

#[derive(Serialize)]
struct Document<'a> {
    schema_version: u32,
    node_id: u64,
    process_id: u32,
    exporter_created_unix_ns: &'a str,
    exporter_uptime_ns: u128,
    captured_unix_ns: String,
    reset: &'static str,
    clock: &'static str,
    duration_unit: &'static str,
    snapshot_consistency: &'static str,
    bucket_count: usize,
    bucket_rule: &'static str,
    export_failures_before_capture: u64,
    export_failures_saturated: bool,
    apply_lag: ApplyLag,
    metrics: Vec<NamedLatency>,
}

struct ExportState {
    next: Instant,
    successes: u64,
    failures: u64,
    failures_saturated: bool,
    last: &'static str,
}

pub(crate) struct MetricsExporter {
    node_id: u64,
    path: PathBuf,
    started: Instant,
    started_unix_ns: String,
    process_identity: ProcessIdentity,
    state: Mutex<ExportState>,
}

/// Bind persisted status to the process that wrote it. PID alone can name a
/// different lifetime, especially PID 1 after a container restarts on its PVC.
/// Capture once; procfs is never read on the request or status-export hot path.
struct ProcessIdentity {
    start_ticks: Option<u64>,
    boot_id: Option<String>,
}

impl ProcessIdentity {
    fn capture() -> Self {
        #[cfg(target_os = "linux")]
        {
            Self {
                start_ticks: read_identity_file("/proc/self/stat")
                    .and_then(|stat| process_start_ticks(&stat, std::process::id())),
                boot_id: read_identity_file("/proc/sys/kernel/random/boot_id")
                    .and_then(|value| process_boot_id(&value)),
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            Self {
                start_ticks: None,
                boot_id: None,
            }
        }
    }

    fn status_lines(&self) -> String {
        format!(
            "process_start_ticks={}\nprocess_boot_id={}\n",
            self.start_ticks
                .map_or_else(|| "unavailable".into(), |value| value.to_string()),
            self.boot_id.as_deref().unwrap_or("unavailable"),
        )
    }
}

#[cfg(target_os = "linux")]
fn read_identity_file(path: &str) -> Option<String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(4097)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 4096 {
        return None;
    }
    String::from_utf8(bytes).ok()
}

#[cfg(any(target_os = "linux", test))]
fn process_start_ticks(stat: &str, expected_pid: u32) -> Option<u64> {
    let (pid, _) = stat.split_once('(')?;
    if pid.trim().parse::<u32>().ok()? != expected_pid {
        return None;
    }
    // Field 2 (comm) can contain spaces, newlines and closing parentheses.
    // Field 22 is 19 positions after field 3 (state), after the final ')'.
    let (_, fields) = stat.rsplit_once(')')?;
    fields.split_whitespace().nth(19)?.parse().ok()
}

#[cfg(any(target_os = "linux", test))]
fn process_boot_id(value: &str) -> Option<String> {
    let value = value.trim();
    if value.len() != 36
        || !value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
    {
        return None;
    }
    Some(value.to_ascii_lowercase())
}

fn unix_ns() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or_else(|_| "unavailable".into(), |d| d.as_nanos().to_string())
}

impl MetricsExporter {
    pub(crate) fn new(data_dir: &Path, node_id: u64) -> Self {
        Self {
            node_id,
            path: data_dir.join("metrics.json"),
            started: Instant::now(),
            started_unix_ns: unix_ns(),
            process_identity: ProcessIdentity::capture(),
            state: Mutex::new(ExportState {
                next: Instant::now(),
                successes: 0,
                failures: 0,
                failures_saturated: false,
                last: "not_attempted",
            }),
        }
    }

    /// The callback and filesystem calls run outside the exporter lock. This
    /// method returns no error to its caller, including on schema/I/O failure.
    pub(crate) fn export(
        &self,
        force: bool,
        capture: impl FnOnce() -> (Vec<NamedLatency>, ApplyLagObservation),
    ) {
        let (failures, saturated) = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let now = Instant::now();
            if !force && now < state.next {
                return;
            }
            state.next = now + INTERVAL;
            (state.failures, state.failures_saturated)
        };
        let (metrics, lag) = capture();
        let document = Document {
            schema_version: 2,
            node_id: self.node_id,
            process_id: std::process::id(),
            exporter_created_unix_ns: &self.started_unix_ns,
            exporter_uptime_ns: self.started.elapsed().as_nanos(),
            captured_unix_ns: unix_ns(),
            reset: "node_component_construction",
            clock: "monotonic_instant",
            duration_unit: "nanoseconds",
            snapshot_consistency: "coherent_per_metric_independent_between_metrics",
            bucket_count: BUCKETS,
            bucket_rule: "0=[0,0]; i>0=[2^(i-1),2^i-1]",
            export_failures_before_capture: failures,
            export_failures_saturated: saturated,
            apply_lag: lag.into(),
            metrics,
        };
        let outcome = self.write_document(&document);
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.last = outcome;
        if outcome == "success" {
            state.successes = state.successes.saturating_add(1);
        } else {
            let (count, overflow) = state.failures.overflowing_add(1);
            state.failures = if overflow { u64::MAX } else { count };
            state.failures_saturated |= overflow;
        }
    }

    fn write_document(&self, document: &Document<'_>) -> &'static str {
        if document.metrics.len() != METRIC_COUNT {
            return "inventory_error";
        }
        let Ok(bytes) = serde_json::to_vec(document) else {
            return "encode_error";
        };
        if bytes.len() > MAX_EXPORT_BYTES {
            return "size_limit";
        }
        let tmp = self.path.with_extension("tmp");
        if std::fs::write(&tmp, bytes).is_err() {
            return "write_error";
        }
        if std::fs::rename(&tmp, &self.path).is_err() {
            return "rename_error";
        }
        // Diagnostic only: no fsync, checkpoint, authority or receipt depends
        // on this file surviving a crash. Rename makes each read one document.
        "success"
    }

    pub(crate) fn status_lines(&self) -> String {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        format!("{}metrics_schema_version=2\nmetrics_export_successes={}\nmetrics_export_failures={}\nmetrics_export_failures_saturated={}\nmetrics_export_last={}\n",
            self.process_identity.status_lines(), state.successes, state.failures, state.failures_saturated, state.last)
    }
}

pub(crate) fn capture<S, E>(
    admission: &crate::admission::PublicAdmission,
    driver: &NodeDriver<S, E>,
    raft: &WalIoMetrics,
    engine: &WalIoMetrics,
) -> (Vec<NamedLatency>, ApplyLagObservation)
where
    S: kv9_raft::rawnode::PersistentRaftStorage,
    E: kv9_raft::ApplyStore + 'static,
{
    let mut metrics = admission.latency_snapshots();
    metrics.extend(driver.metrics().snapshots());
    for (name, latency) in [
        ("raft_wal_record_write", &raft.write),
        ("raft_wal_record_sync", &raft.sync),
        ("raft_wal_recovery_sync", &raft.recovery_sync),
        ("raft_wal_namespace_publish", &raft.namespace_publish),
        ("engine_wal_record_write", &engine.write),
        ("engine_wal_record_sync", &engine.sync),
        ("engine_wal_recovery_sync", &engine.recovery_sync),
        ("engine_wal_namespace_publish", &engine.namespace_publish),
    ] {
        metrics.push(NamedLatency::new(name, latency));
    }
    (metrics, driver.apply_lag_observation())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kv9_common::metrics::{BucketBounds, Outcome};
    use kv9_common::{NodeId, RegionId};
    use kv9_raft::{MemStateMachine, RaftGroup, RaftPeer};
    use std::sync::Arc;

    #[test]
    fn process_identity_distinguishes_reused_pid_and_rejects_missing_evidence() {
        fn stat(start: &str) -> String {
            format!(
                "1 (kv9 ) worker\nname) S {} {start} 999 888\n",
                vec!["0"; 18].join(" ")
            )
        }
        let old = process_start_ticks(&stat("132065456"), 1).unwrap();
        let new = process_start_ticks(&stat("132069188"), 1).unwrap();
        assert_eq!(old, 132065456);
        assert_eq!(new, 132069188);
        let boot = "00112233-4455-6677-8899-aabbccddeeff";
        let old_status = ProcessIdentity {
            start_ticks: Some(old),
            boot_id: Some(boot.into()),
        }
        .status_lines();
        let new_status = ProcessIdentity {
            start_ticks: Some(new),
            boot_id: Some(boot.into()),
        }
        .status_lines();
        assert_ne!(
            old_status, new_status,
            "reused PID lost its process-start discriminator"
        );
        assert!(old_status.contains("process_start_ticks=132065456\n"));
        assert!(new_status.contains("process_start_ticks=132069188\n"));
        assert_eq!(process_start_ticks(&stat("132069188"), 2), None);
        assert_eq!(process_start_ticks("1 (kv9) S 0", 1), None);
        assert_eq!(process_start_ticks(&stat("unavailable"), 1), None);
        assert_eq!(process_start_ticks(&stat("18446744073709551616"), 1), None);
        assert_eq!(process_boot_id(&format!("{boot}\n")), Some(boot.into()));
        assert_eq!(process_boot_id(""), None);
        assert_eq!(process_boot_id(&format!("{boot}\nforged=true")), None);
        assert_eq!(
            process_boot_id("00112233_4455-6677-8899-aabbccddeeff"),
            None
        );
        assert_eq!(
            ProcessIdentity {
                start_ticks: None,
                boot_id: None
            }
            .status_lines(),
            "process_start_ticks=unavailable\nprocess_boot_id=unavailable\n"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn status_identity_is_captured_once_and_matches_the_running_process() {
        let exporter = MetricsExporter::new(Path::new("unused-identity-test"), 1);
        let before = exporter.status_lines();
        let observed = ProcessIdentity::capture();
        assert!(observed.start_ticks.is_some(), "test needs Linux procfs");
        assert!(observed.boot_id.is_some(), "test needs Linux boot identity");
        assert!(before.starts_with(&observed.status_lines()));
        assert_eq!(before, exporter.status_lines());
    }

    fn fixture() -> (Arc<crate::admission::PublicAdmission>, Arc<NodeDriver>) {
        let hub = kv9_raft::transport::InProcHub::new();
        let peer = Arc::new(RaftPeer::new(NodeId(1), RegionId(1), &[NodeId(1)]).unwrap());
        let driver = NodeDriver::new(
            peer,
            Arc::new(hub.endpoint(NodeId(1))),
            MemStateMachine::new(),
        )
        .unwrap();
        (
            crate::admission::PublicAdmission::new(Default::default()).unwrap(),
            driver,
        )
    }

    #[test]
    fn export_inventory_is_fixed_and_worst_case_document_fits_the_cap() {
        let (admission, driver) = fixture();
        let io = WalIoMetrics::default();
        let (mut metrics, lag) = capture(&admission, &driver, &io, &io);
        assert_eq!(metrics.len(), METRIC_COUNT);
        let names: std::collections::BTreeSet<_> = metrics.iter().map(|m| m.name).collect();
        assert_eq!(names.len(), METRIC_COUNT);
        assert!(names.contains("public_transaction_backend"));
        assert!(names.contains("raft_application_wait"));
        assert!(names.contains("raft_pump_service"));
        assert!(names.contains("raft_pump_idle_wait"));
        assert!(names.contains("raft_pump_iteration_spacing"));
        assert!(names.contains("engine_wal_record_sync"));
        for metric in &mut metrics {
            for h in &mut metric.latency.outcomes {
                assert_eq!(h.buckets.len(), 65);
                h.buckets.fill(u64::MAX);
                h.count = u64::MAX;
                h.sum_ns = u64::MAX;
                h.min_ns = Some(u64::MAX);
                h.max_ns = Some(u64::MAX);
                let bounds = Some(BucketBounds {
                    lower_ns: u64::MAX,
                    upper_ns: u64::MAX,
                });
                h.p50 = bounds;
                h.p95 = bounds;
                h.p99 = bounds;
            }
        }
        let dir = std::env::temp_dir().join(format!(
            "kv9-metrics-bound-{}-{}",
            std::process::id(),
            unix_ns()
        ));
        std::fs::create_dir(&dir).unwrap();
        let exporter = MetricsExporter::new(&dir, u64::MAX);
        exporter.export(true, || (metrics, lag));
        let bytes = std::fs::read(dir.join("metrics.json")).unwrap();
        assert!(
            bytes.len() < MAX_EXPORT_BYTES - 4096,
            "reserve room for maximum-width envelope fields"
        );
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["schema_version"], 2);
        assert_eq!(value["metrics"].as_array().unwrap().len(), METRIC_COUNT);
        assert!(value["apply_lag"]["lag_entries"].is_null());
        exporter.export(false, || panic!("rate-limited export must not collect"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_metrics_publication_does_not_prevent_driver_progress() {
        let (admission, driver) = fixture();
        let io = WalIoMetrics::default();
        let dir = std::env::temp_dir().join(format!(
            "kv9-metrics-failure-{}-{}",
            std::process::id(),
            unix_ns()
        ));
        // A directory at the target makes atomic rename fail, independently of
        // running as root or filesystem permission policy.
        std::fs::create_dir_all(dir.join("metrics.json")).unwrap();
        let exporter = MetricsExporter::new(&dir, 1);
        exporter.export(true, || capture(&admission, &driver, &io, &io));
        assert!(exporter
            .status_lines()
            .contains("metrics_export_last=rename_error\n"));
        driver.peer().campaign().unwrap();
        driver.tick_and_step().unwrap();
        let at = driver
            .propose(&kv9_raft::Command::Put {
                cf: 0,
                key: b"k".to_vec(),
                value: b"v".to_vec(),
            })
            .unwrap();
        driver.tick_and_step().unwrap();
        assert!(matches!(
            driver.wait_applied(at, Duration::ZERO).unwrap(),
            kv9_raft::driver::ApplyWaitOutcome::Applied(_)
        ));
        assert_eq!(
            driver.get(kv9_engine::ColumnFamily::Default, b"k").unwrap(),
            Some(b"v".to_vec())
        );
        std::fs::remove_dir(dir.join("metrics.json")).unwrap();
        exporter.export(true, || capture(&admission, &driver, &io, &io));
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("metrics.json")).unwrap()).unwrap();
        assert_eq!(value["export_failures_before_capture"], 1);
        assert_eq!(value["apply_lag"]["lag_entries"], 0);
        assert_eq!(
            driver.metrics().application_wait.snapshot().outcomes[Outcome::Success as usize].count,
            1
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
