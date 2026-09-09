use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::task::JoinSet;

use super::recorder::PHASES;
use super::{Generator, HistorySummary, Metrics, MetricsSnapshot, Mode, Recorder, WorkloadConfig};
use crate::client::{CallReport, Outcome, PersistentRawClient, RawOperation, Reason, Value};

const MAX_REPORT_BYTES: u64 = 2_097_152;
const MAX_BINARY_BYTES: u64 = 536_870_912;
const SETUP_BUDGET: Duration = Duration::from_secs(300);
const VERIFY_BUDGET: Duration = Duration::from_secs(60);

pub struct RunOptions {
    pub output: PathBuf,
    pub build_manifest: PathBuf,
    pub stop_file: Option<PathBuf>,
    pub phase_file: Option<PathBuf>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildManifest {
    pub version: u32,
    pub revision: String,
    pub dirty: bool,
    pub source_tree_sha256: String,
    pub binary_sha256: String,
    pub profile: String,
    pub rustc: String,
}

#[derive(Clone, Copy, Serialize)]
pub struct Span {
    pub start_ns: u64,
    pub end_ns: u64,
}

#[derive(Clone, Copy, Serialize)]
pub struct StopEvent {
    pub reason: &'static str,
    pub monotonic_ns: u64,
}

#[derive(Default, Serialize)]
pub struct Stages {
    pub initialization: Option<Span>,
    pub warmup: Option<Span>,
    pub measurement: Option<Span>,
    pub drain: Option<Span>,
    pub verification: Option<Span>,
}

#[derive(Clone, Serialize)]
pub struct Resources {
    pub user_ticks: Option<u64>,
    pub system_ticks: Option<u64>,
    pub rss_bytes: Option<u64>,
    pub peak_rss_bytes: Option<u64>,
}

#[derive(Serialize)]
pub struct RunReport {
    pub version: u32,
    pub complete: bool,
    pub failure: Option<&'static str>,
    pub configuration: WorkloadConfig,
    pub config_sha256: String,
    pub build: BuildManifest,
    pub build_sha256: String,
    pub history_sha256: Option<String>,
    pub process_id: u32,
    pub wall_anchor_unix_ns: u64,
    pub wall_anchor_monotonic_ns: u64,
    pub elapsed_ns: u64,
    pub runtime_threads: usize,
    pub workload_model: &'static str,
    pub independently_checked: bool,
    pub stop: Option<StopEvent>,
    pub stages: Stages,
    pub measured_issued: u64,
    pub measured_completed: u64,
    pub measured_successful: u64,
    pub cohort_elapsed_ns: u64,
    pub cohort_terminal_ops_per_second: Option<f64>,
    pub cohort_success_ops_per_second: Option<f64>,
    pub resources_before: Resources,
    pub resources_after: Resources,
    pub history: HistorySummary,
    pub metrics: MetricsSnapshot,
}

fn ns(epoch: Instant) -> u64 {
    epoch.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
}

fn bounded_read(path: &Path, limit: u64) -> Result<Vec<u8>, &'static str> {
    let mut data = Vec::new();
    File::open(path)
        .map_err(|_| "cannot open workload input")?
        .take(limit + 1)
        .read_to_end(&mut data)
        .map_err(|_| "cannot read workload input")?;
    if data.len() as u64 > limit {
        return Err("workload input exceeds its bound");
    }
    Ok(data)
}

fn digest(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

fn file_digest(path: &Path, limit: u64) -> Result<String, &'static str> {
    let mut file = File::open(path).map_err(|_| "cannot open workload artifact for hashing")?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65_536];
    let mut count = 0;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| "cannot hash workload artifact")?;
        if read == 0 {
            break;
        }
        count += read as u64;
        if count > limit {
            return Err("workload artifact exceeds its hash bound");
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), &'static str> {
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .and_then(|mut file| file.write_all(bytes))
        .map_err(|_| "cannot create workload artifact")
}

fn atomic_json(path: &Path, value: &impl Serialize) -> Result<(), &'static str> {
    let bytes = serde_json::to_vec(value).map_err(|_| "cannot encode workload report")?;
    if bytes.len() as u64 > MAX_REPORT_BYTES {
        return Err("workload report exceeds its bound");
    }
    let temporary = path.with_extension("tmp");
    // The output directory was created exclusively by this run. These fixed
    // temporary/final paths cannot replace another run's artifacts.
    let mut file = File::create(&temporary).map_err(|_| "cannot create workload report")?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "cannot persist workload report")?;
    fs::rename(temporary, path).map_err(|_| "cannot publish workload report")
}

fn resources() -> Resources {
    let stat = bounded_read(Path::new("/proc/self/stat"), 4096)
        .ok()
        .and_then(|v| String::from_utf8(v).ok());
    let times = stat
        .as_ref()
        .and_then(|text| text.rsplit_once(')'))
        .map(|(_, fields)| fields.split_whitespace().collect::<Vec<_>>());
    let field = |index| {
        times
            .as_ref()
            .and_then(|fields| fields.get(index))
            .and_then(|s: &&str| s.parse().ok())
    };
    let status = bounded_read(Path::new("/proc/self/status"), 16_384)
        .ok()
        .and_then(|v| String::from_utf8(v).ok());
    let memory = |name: &str| {
        status
            .as_ref()
            .and_then(|text| text.lines().find(|line| line.starts_with(name)))
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|s| s.parse::<u64>().ok())
            .and_then(|kb| kb.checked_mul(1024))
    };
    Resources {
        user_ticks: field(11),
        system_ticks: field(12),
        rss_bytes: memory("VmRSS:"),
        peak_rss_bytes: memory("VmHWM:"),
    }
}

struct Shared {
    config: WorkloadConfig,
    client: PersistentRawClient,
    recorder: Recorder,
    generator: Generator,
    metrics: Metrics,
    stop: Mutex<Option<StopEvent>>,
    failure: Mutex<Option<&'static str>>,
    nonce: AtomicU64,
}

impl Shared {
    fn stop(&self, reason: &'static str) {
        self.stop
            .lock()
            .expect("workload stop lock poisoned")
            .get_or_insert(StopEvent {
                reason,
                monotonic_ns: ns(self.recorder.epoch()),
            });
    }
    fn stopped(&self) -> bool {
        self.stop
            .lock()
            .expect("workload stop lock poisoned")
            .is_some()
    }
    fn fail(&self, reason: &'static str) {
        self.failure
            .lock()
            .expect("workload failure lock poisoned")
            .get_or_insert(reason);
        self.stop("failure");
    }
    async fn call(
        &self,
        worker: usize,
        phase: &str,
        operation: RawOperation,
    ) -> Result<CallReport, &'static str> {
        let ticket = self.recorder.begin(worker, phase, &operation)?;
        let report = self.client.call(operation).await;
        let recorded = self.recorder.complete(ticket, &report);
        let measured = self.metrics.record(phase, &report);
        recorded?;
        measured?;
        if matches!(
            &report.outcome,
            Outcome::UnknownWrite {
                reason: Reason::Protocol
            } | Outcome::ReadFailure {
                reason: Reason::Protocol
            }
        ) {
            return Err("workload observed a protocol violation");
        }
        Ok(report)
    }
    fn traffic(&self) -> RawOperation {
        self.generator
            .operation(self.nonce.fetch_add(1, Ordering::Relaxed))
    }
}

fn success(report: &CallReport) -> Result<&Value, &'static str> {
    match &report.outcome {
        Outcome::Success { value } => Ok(value),
        _ => Err("required workload operation was not acknowledged"),
    }
}

fn interrupted(options: &RunOptions) -> bool {
    options.stop_file.as_ref().is_some_and(|path| path.exists())
}

async fn initialize(
    shared: &Shared,
    options: &RunOptions,
    deadline: Instant,
) -> Result<(), &'static str> {
    // Bounded DISTINCT reads can rotate past an unavailable initial endpoint.
    // Every probe has its own invocation/terminal; unknown writes are never retried.
    for probe in 0..shared.config.client.peers.len() {
        let observed = shared
            .call(
                0,
                "initialization",
                RawOperation::Get {
                    key: shared.generator.sentinel(),
                },
            )
            .await?;
        match observed.outcome {
            Outcome::Success {
                value: Value::Get { value: None },
            } => break,
            Outcome::Success { .. } => return Err("workload sentinel already exists"),
            _ if probe + 1 < shared.config.client.peers.len() && Instant::now() < deadline => {
                continue
            }
            _ => return Err("no configured endpoint established the fresh sentinel read"),
        }
    }
    for index in 0..=shared.config.keys {
        if interrupted(options) || Instant::now() >= deadline {
            return Err("setup issuance stopped before initialization completed");
        }
        let key = shared.generator.key(index)?;
        // The sentinel was checked by the probe above; all other keys must also
        // be absent before their first acknowledged initialization write.
        if index != shared.config.keys {
            let report = shared
                .call(0, "initialization", RawOperation::Get { key: key.clone() })
                .await?;
            if !matches!(success(&report)?, Value::Get { value: None }) {
                return Err("workload dataset is not fresh");
            }
        }
        let report = shared
            .call(
                0,
                "initialization",
                RawOperation::Put {
                    key,
                    value: shared.generator.value(index as u64),
                },
            )
            .await?;
        success(&report)?;
    }
    Ok(())
}

async fn verify(shared: &Shared, deadline: Instant) -> Result<(), &'static str> {
    // Sentinel first, then a final establishing read of every mutable key. The
    // complete correctness checker decides the allowed state of mutable keys.
    for index in std::iter::once(shared.config.keys).chain(0..shared.config.keys) {
        let mut final_read = None;
        for _ in 0..shared.config.client.peers.len() {
            if Instant::now() >= deadline {
                return Err("verification issuance budget exhausted");
            }
            let report = shared
                .call(
                    0,
                    "verify",
                    RawOperation::Get {
                        key: shared.generator.key(index)?,
                    },
                )
                .await?;
            let acknowledged = matches!(report.outcome, Outcome::Success { .. });
            final_read = Some(report);
            if acknowledged {
                break;
            }
        }
        let report = final_read.ok_or("verification has no configured endpoint")?;
        let value = success(&report)?;
        if index == shared.config.keys
            && *value
                != (Value::Get {
                    value: Some(shared.generator.value(index as u64)),
                })
        {
            return Err("acknowledged immutable sentinel was lost");
        }
    }
    Ok(())
}

fn phase(path: Option<&Path>) -> Result<&'static str, &'static str> {
    let Some(path) = path else {
        return Ok("measure");
    };
    let bytes = bounded_read(path, 64)?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| "invalid workload phase encoding")?
        .trim();
    PHASES
        .iter()
        .copied()
        .find(|known| *known == text && (*known == "measure" || PHASES[4..].contains(known)))
        .ok_or("invalid measurement phase")
}

async fn measure(
    shared: Arc<Shared>,
    options: &RunOptions,
    stages: &mut Stages,
) -> Result<u64, &'static str> {
    let epoch = shared.recorder.epoch();
    let start_ns = ns(epoch);
    let deadline = Instant::now() + Duration::from_millis(shared.config.measure_ms);
    let remaining = shared
        .config
        .max_operations
        .checked_sub(shared.recorder.progress()?.issued + shared.config.verification_operations())
        .filter(|remaining| *remaining > 0)
        .ok_or("operation budget leaves no measured work and final reads")?;
    let allocated = Arc::new(AtomicU64::new(0));
    let mut tasks = JoinSet::new();
    for worker in 0..shared.config.workers {
        let shared = shared.clone();
        let allocated = allocated.clone();
        let phase_file = options.phase_file.clone();
        let stop_file = options.stop_file.clone();
        tasks.spawn(async move {
            loop {
                if shared.stopped() {
                    break;
                }
                if Instant::now() >= deadline {
                    shared.stop("duration");
                    break;
                }
                if stop_file.as_ref().is_some_and(|path| path.exists()) {
                    shared.stop("stop_file");
                    break;
                }
                let current_phase = match phase(phase_file.as_deref()) {
                    Ok(phase) => phase,
                    Err(error) => {
                        shared.fail(error);
                        break;
                    }
                };
                if allocated
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                        (value < remaining).then_some(value + 1)
                    })
                    .is_err()
                {
                    shared.stop("operation_limit");
                    break;
                }
                if let Err(error) = shared.call(worker, current_phase, shared.traffic()).await {
                    shared.fail(error);
                    break;
                }
                if shared.config.interval_ms > 0 {
                    tokio::time::sleep(Duration::from_millis(shared.config.interval_ms)).await;
                }
            }
        });
    }
    // Do not return early or drop JoinSet on a worker/error/stop condition.
    // Every surviving worker completes its issued call before its next stop check.
    while !tasks.is_empty() {
        tokio::select! {
            completed = tasks.join_next() => { if completed.is_some_and(|result| result.is_err()) { shared.fail("workload worker did not finish normally"); } }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                if Instant::now() >= deadline { shared.stop("duration"); }
                if interrupted(options) { shared.stop("stop_file"); }
                match shared.recorder.progress() {
                    Ok(progress) => {
                        if let Err(error) = atomic_json(&options.output.join("progress.json"), &serde_json::json!({
                            "version": 1, "stage": if shared.stopped() { "drain" } else { "measure" },
                            "monotonic_ns": ns(epoch), "progress": progress,
                        })) { shared.fail(error); }
                    }
                    Err(error) => shared.fail(error),
                }
            }
        }
    }
    let end_ns = ns(epoch);
    let stopped = *shared
        .stop
        .lock()
        .map_err(|_| "workload stop lock poisoned")?;
    let close = stopped
        .map_or(end_ns, |event| event.monotonic_ns)
        .max(start_ns);
    stages.measurement = Some(Span {
        start_ns,
        end_ns: close,
    });
    stages.drain = Some(Span {
        start_ns: close,
        end_ns,
    });
    Ok(allocated.load(Ordering::Relaxed))
}

/// Compose a bounded run. Failure reports are retained; complete=false is not a
/// passing experiment. The caller exits nonzero and the independent verifier
/// checks the exact artifacts before any correctness/performance claim.
pub async fn run(
    config: WorkloadConfig,
    token: &str,
    options: RunOptions,
) -> Result<RunReport, &'static str> {
    config.validate()?;
    let build_bytes = bounded_read(&options.build_manifest, 65_536)?;
    let build: BuildManifest =
        serde_json::from_slice(&build_bytes).map_err(|_| "invalid build manifest")?;
    let hexadecimal = |s: &str, len| {
        s.len() == len
            && s.bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    };
    if build.version != 1
        || !hexadecimal(&build.revision, 40)
        || !hexadecimal(&build.source_tree_sha256, 64)
        || !hexadecimal(&build.binary_sha256, 64)
        || !["debug", "release"].contains(&build.profile.as_str())
        || build.rustc.len() > 4096
    {
        return Err("build manifest fields are invalid");
    }
    let executable = std::env::current_exe().map_err(|_| "cannot identify workload executable")?;
    if file_digest(&executable, MAX_BINARY_BYTES)? != build.binary_sha256 {
        return Err("build manifest does not match this executable");
    }
    let client = PersistentRawClient::new(config.client.clone(), token)?;
    let generator = Generator::new(config.clone())?;
    fs::create_dir(&options.output).map_err(|_| "workload output directory must be new")?;
    let config_bytes =
        serde_json::to_vec_pretty(&config).map_err(|_| "cannot encode workload configuration")?;
    write_new(&options.output.join("config.json"), &config_bytes)?;
    write_new(&options.output.join("build.json"), &build_bytes)?;
    let history_path = options.output.join("history.jsonl");
    let recorder = Recorder::new(
        config.clone(),
        (config.mode == Mode::Correctness).then_some(history_path.as_path()),
    )?;
    let epoch = recorder.epoch();
    let wall_anchor_unix_ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system wall clock precedes Unix epoch")?
        .as_nanos() as u64;
    let wall_anchor_monotonic_ns = ns(epoch);
    let shared = Arc::new(Shared {
        config: config.clone(),
        client,
        recorder,
        generator,
        metrics: Metrics::default(),
        stop: Mutex::new(None),
        failure: Mutex::new(None),
        nonce: AtomicU64::new(config.keys as u64 + 1),
    });
    let resources_before = resources();
    let mut stages = Stages::default();
    let startup_deadline = Instant::now() + SETUP_BUDGET;
    let start_ns = ns(epoch);
    let mut execution = initialize(&shared, &options, startup_deadline).await;
    stages.initialization = Some(Span {
        start_ns,
        end_ns: ns(epoch),
    });
    if execution.is_ok() {
        let start_ns = ns(epoch);
        for _ in 0..config.warmup_operations {
            if interrupted(&options) || Instant::now() >= startup_deadline {
                execution = Err("setup issuance stopped before warmup completed");
                break;
            }
            match shared.call(0, "warmup", shared.traffic()).await {
                Ok(report) if matches!(report.outcome, Outcome::Success { .. }) => {}
                Ok(report) => {
                    // Failed calls contain only bounded typed outcomes and
                    // attempts, never successful values, request data or RPC
                    // prose. Preserve routing evidence in performance mode too.
                    execution = atomic_json(
                        &options.output.join("warmup-failure.json"),
                        &serde_json::json!({"version": 1, "call": report}),
                    )
                    .and(Err("warmup operation was not acknowledged"));
                    break;
                }
                Err(error) => {
                    execution = Err(error);
                    break;
                }
            }
        }
        stages.warmup = Some(Span {
            start_ns,
            end_ns: ns(epoch),
        });
    }
    let mut measured_allocated = 0;
    if execution.is_ok() {
        execution = atomic_json(
            &options.output.join("ready.json"),
            &serde_json::json!({"version": 1, "keyspace_id": config.client.keyspace_id, "monotonic_ns": ns(epoch)}),
        );
    }
    if execution.is_ok() {
        match measure(shared.clone(), &options, &mut stages).await {
            Ok(count) => measured_allocated = count,
            Err(error) => execution = Err(error),
        }
    }
    if execution.is_ok()
        && shared
            .failure
            .lock()
            .map_err(|_| "workload failure lock poisoned")?
            .is_none()
    {
        let start_ns = ns(epoch);
        execution = verify(&shared, Instant::now() + VERIFY_BUDGET).await;
        stages.verification = Some(Span {
            start_ns,
            end_ns: ns(epoch),
        });
    }
    if let Err(error) = execution {
        shared.fail(error);
    }
    let history = shared.recorder.finish()?;
    if !history.accounting_complete {
        shared.fail("workload operation accounting is incomplete");
    }
    let metrics = shared.metrics.snapshot()?;
    let mut measured_completed = 0;
    let mut measured_successful = 0;
    for (_, counts) in metrics
        .logical_counts
        .iter()
        .enumerate()
        .filter(|(i, _)| *i == 2 || *i >= 4)
    {
        measured_completed += counts.iter().flatten().sum::<u64>();
        measured_successful += counts.iter().map(|row| row[0]).sum::<u64>();
    }
    if measured_allocated == 0 || measured_completed != measured_allocated {
        shared.fail("measured operations are empty or incomplete");
    }
    let cohort_elapsed_ns = stages
        .measurement
        .zip(stages.drain)
        .map_or(0, |(measurement, drain)| {
            drain.end_ns - measurement.start_ns
        });
    let rate =
        |count| (cohort_elapsed_ns > 0).then(|| count as f64 * 1e9 / cohort_elapsed_ns as f64);
    let history_sha256 = if config.mode == Mode::Correctness {
        Some(file_digest(&history_path, config.history_bytes)?)
    } else {
        None
    };
    let failure = *shared
        .failure
        .lock()
        .map_err(|_| "workload failure lock poisoned")?;
    let report = RunReport {
        version: 1,
        complete: failure.is_none(),
        failure,
        configuration: config,
        config_sha256: digest(&config_bytes),
        build,
        build_sha256: digest(&build_bytes),
        history_sha256,
        process_id: std::process::id(),
        wall_anchor_unix_ns,
        wall_anchor_monotonic_ns,
        elapsed_ns: ns(epoch),
        runtime_threads: tokio::runtime::Handle::current().metrics().num_workers(),
        workload_model: "closed_loop",
        independently_checked: false,
        stop: *shared
            .stop
            .lock()
            .map_err(|_| "workload stop lock poisoned")?,
        stages,
        measured_issued: measured_allocated,
        measured_completed,
        measured_successful,
        cohort_elapsed_ns,
        cohort_terminal_ops_per_second: rate(measured_completed),
        cohort_success_ops_per_second: rate(measured_successful),
        resources_before,
        resources_after: resources(),
        history,
        metrics,
    };
    atomic_json(&options.output.join("report.json"), &report)?;
    Ok(report)
}
