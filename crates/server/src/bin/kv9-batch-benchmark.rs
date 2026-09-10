//! Native batch performance workload. Full correctness histories use the
//! separate kv9-batch-workload executable; this tool retains aggregate outcomes.
#[path = "kv9-batch-benchmark/common.rs"]
mod common;
#[path = "kv9-batch-benchmark/metrics.rs"]
mod metrics;
#[path = "kv9-batch-benchmark/model.rs"]
mod model;

use kv9_server::client::{CallReport, Outcome, PersistentRawClient, Reason, Value};
use metrics::{Metrics, Sample};
use model::{Config, Load};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Build {
    version: u32,
    revision: String,
    dirty: bool,
    source_tree_sha256: String,
    binary_sha256: String,
    profile: String,
    rustc: String,
}

fn nanos(start: Instant) -> u64 {
    start.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|_| "cannot open benchmark input")?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "cannot read benchmark input")?;
    if bytes.len() as u64 > limit {
        return Err("benchmark input exceeds its bound".into());
    }
    Ok(bytes)
}
fn file_digest(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|_| "cannot open benchmark executable")?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65_536];
    let mut total = 0u64;
    loop {
        let n = file
            .read(&mut buffer)
            .map_err(|_| "cannot hash benchmark executable")?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > 536_870_912 {
            return Err("benchmark executable exceeds its hash bound".into());
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|mut f| f.write_all(bytes))
        .map_err(|_| "cannot create benchmark artifact".into())
}
fn write_json(path: &Path, value: &Json) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|_| "cannot encode benchmark artifact")?;
    if bytes.len() > 16_777_216 {
        return Err("benchmark report exceeds its bound".into());
    }
    write_new(path, &bytes)
}
fn proc_stat() -> String {
    fs::read_to_string("/proc/self/stat").unwrap_or_default()
}

fn valid_outcome(config: &Config, nonce: u64, outcome: &Outcome) -> bool {
    match outcome {
        Outcome::Success {
            value: Value::BatchGet { values },
        } if config.is_read(nonce) => {
            let first = config.first_key(nonce);
            values.len() == config.batch_size
                && values.iter().enumerate().all(|(i, value)| {
                    value.as_ref().is_some_and(|v| {
                        config.valid_value(
                            (first + i) % config.keys,
                            v,
                            config.warmup_calls + config.max_calls,
                        )
                    })
                })
        }
        Outcome::Success {
            value: Value::Applied { term, index },
        } if !config.is_read(nonce) => *term > 0 && *index > 0,
        Outcome::Success { .. } => false,
        Outcome::UnknownWrite { reason } => !config.is_read(nonce) && *reason != Reason::Protocol,
        Outcome::ReadFailure { reason } => config.is_read(nonce) && *reason != Reason::Protocol,
        Outcome::Refused { reason } | Outcome::ClientRejected { reason } => {
            *reason != Reason::Protocol
        }
    }
}

struct CallTiming {
    epoch: Instant,
    cutoff: Option<Instant>,
    scheduled: Option<Instant>,
}
async fn traffic(
    config: &Config,
    client: &PersistentRawClient,
    nonce: u64,
    timing: CallTiming,
    metrics: &mut Metrics,
    issued: Option<&AtomicU64>,
) -> Option<(bool, bool, u64)> {
    let start = Instant::now();
    if timing.cutoff.is_some_and(|cutoff| start >= cutoff) {
        return None;
    }
    if let Some(issued) = issued {
        issued.fetch_add(1, Ordering::Relaxed);
    }
    let operation = config.operation(nonce);
    let expected_kind = operation.kind();
    let report = client.call(operation).await;
    let valid = report.operation == expected_kind && valid_outcome(config, nonce, &report.outcome);
    let finished = Instant::now();
    let success = matches!(report.outcome, Outcome::Success { .. });
    metrics.record(Sample {
        report: &report,
        items: config.batch_size,
        whole_call_ns: finished.duration_since(start).as_nanos() as u64,
        before_cutoff: timing.cutoff.is_none_or(|cutoff| finished < cutoff),
        scheduled_to_completion_ns: timing
            .scheduled
            .map(|scheduled| finished.saturating_duration_since(scheduled).as_nanos() as u64),
        dispatch_lateness_ns: timing
            .scheduled
            .map(|scheduled| start.saturating_duration_since(scheduled).as_nanos() as u64),
        data_valid: valid,
    });
    Some((
        success,
        valid,
        finished.duration_since(timing.epoch).as_nanos() as u64,
    ))
}

fn setup_observation(
    metrics: &mut Metrics,
    report: &CallReport,
    items: usize,
    start: Instant,
    valid: bool,
) {
    metrics.record(Sample {
        report,
        items,
        whole_call_ns: nanos(start),
        before_cutoff: true,
        scheduled_to_completion_ns: None,
        dispatch_lateness_ns: None,
        data_valid: valid,
    });
}

struct Worker {
    id: usize,
    metrics: Metrics,
    dropped: u64,
    last_terminal_ns: u64,
    stopped_ns: u64,
    stopped_for_failure: bool,
}

/// Each fixed-rate worker owns a strided set of slots. At most one slot is
/// waiting or executing per worker. Overdue slots are counted and shed rather
/// than accumulated into a burst or an unbounded client queue.
fn latest_due_slot(config: &Config, elapsed_ns: u64, worker: usize) -> Option<u64> {
    common::latest_due_slot(
        config.load,
        config.measure_ms,
        elapsed_ns,
        worker,
        config.workers,
    )
}

async fn worker(
    config: Arc<Config>,
    client: PersistentRawClient,
    id: usize,
    epoch: Instant,
    allocated: Arc<AtomicU64>,
    issued: Arc<AtomicU64>,
    cancel: CancellationToken,
) -> Worker {
    let cutoff = epoch + Duration::from_millis(config.measure_ms);
    let mut result = Worker {
        id,
        metrics: Metrics::default(),
        dropped: 0,
        last_terminal_ns: 0,
        stopped_ns: 0,
        stopped_for_failure: false,
    };
    let mut next = id as u64;
    loop {
        if cancel.is_cancelled() {
            result.stopped_for_failure = true;
            break;
        }
        let (slot, scheduled) = match config.load {
            Load::ClosedLoop => {
                if Instant::now() >= cutoff {
                    break;
                }
                let slot = match allocated.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                    (n < config.max_calls).then_some(n + 1)
                }) {
                    Ok(slot) => slot,
                    Err(_) => break,
                };
                (slot, None)
            }
            Load::FixedRate { .. } => {
                let total = config.offered_slots().unwrap();
                if next >= total {
                    break;
                }
                let scheduled = epoch + Duration::from_nanos(config.slot_due_ns(next).unwrap());
                if Instant::now() < scheduled {
                    tokio::select! {
                        _=tokio::time::sleep_until(scheduled.into()) => {},
                        _=cancel.cancelled() => { result.stopped_for_failure=true; break; }
                    }
                }
                if Instant::now() >= cutoff {
                    result.dropped += 1 + (total - 1 - next) / config.workers as u64;
                    break;
                }
                if let Some(latest) = latest_due_slot(&config, nanos(epoch), id) {
                    if latest > next {
                        result.dropped += (latest - next) / config.workers as u64;
                        next = latest;
                    }
                }
                let slot = next;
                next += config.workers as u64;
                (
                    slot,
                    Some(epoch + Duration::from_nanos(config.slot_due_ns(slot).unwrap())),
                )
            }
        };
        match traffic(
            &config,
            &client,
            config.warmup_calls + slot + 1,
            CallTiming {
                epoch,
                cutoff: Some(cutoff),
                scheduled,
            },
            &mut result.metrics,
            Some(&issued),
        )
        .await
        {
            Some((_, valid, end)) => {
                result.last_terminal_ns = end;
                if !valid {
                    result.stopped_for_failure = true;
                    cancel.cancel();
                    break;
                }
            }
            None => {
                if scheduled.is_some() {
                    result.dropped += 1;
                }
            }
        }
    }
    result.stopped_ns = nanos(epoch);
    result
}

#[derive(Default)]
struct Run {
    initialization: Metrics,
    warmup: Metrics,
    measurement: Metrics,
    verification: Metrics,
    stages: BTreeMap<&'static str, Json>,
    workers: Vec<Json>,
    measured_issued: u64,
    dropped: u64,
    task_failures: u64,
    cohort_elapsed_ns: u64,
    stop_reason: Option<&'static str>,
    measurement_start_unix_ns: Option<u128>,
    proc_before: String,
    proc_after: String,
}

async fn run(
    config: &Config,
    client: &PersistentRawClient,
    output: &Path,
    result: &mut Run,
) -> Result<(), String> {
    let epoch = Instant::now();
    let setup_deadline = epoch + Duration::from_secs(300);
    for first in (0..=config.keys).step_by(config.batch_size) {
        if Instant::now() >= setup_deadline {
            return Err("initialization budget exhausted".into());
        }
        let end = (first + config.batch_size).min(config.keys + 1);
        let start = Instant::now();
        let report = client
            .batch_get((first..end).map(|i| config.key(i)).collect())
            .await;
        let valid = matches!(&report.outcome,Outcome::Success { value:Value::BatchGet { values } } if values.len()==end-first && values.iter().all(Option::is_none));
        setup_observation(
            &mut result.initialization,
            &report,
            end - first,
            start,
            valid,
        );
        if !valid {
            return Err("benchmark requires a fresh empty dataset".into());
        }
        let start = Instant::now();
        let report = client
            .batch_put(
                (first..end)
                    .map(|i| (config.key(i), config.value(i, 0)))
                    .collect(),
            )
            .await;
        let valid = matches!(&report.outcome,Outcome::Success { value:Value::Applied { term,index } } if *term>0 && *index>0);
        setup_observation(
            &mut result.initialization,
            &report,
            end - first,
            start,
            valid,
        );
        if !valid {
            return Err("dataset initialization was unconfirmed".into());
        }
    }
    result.stages.insert(
        "initialization",
        json!({"start_ns":0,"end_ns":nanos(epoch)}),
    );
    let warmup_start = nanos(epoch);
    for nonce in 1..=config.warmup_calls {
        if Instant::now() >= setup_deadline {
            return Err("warmup budget exhausted".into());
        }
        let (success, valid, _) = traffic(
            config,
            client,
            nonce,
            CallTiming {
                epoch,
                cutoff: None,
                scheduled: None,
            },
            &mut result.warmup,
            None,
        )
        .await
        .unwrap();
        if !success || !valid {
            return Err("benchmark warmup failed".into());
        }
    }
    result.stages.insert(
        "warmup",
        json!({"start_ns":warmup_start,"end_ns":nanos(epoch)}),
    );
    write_json(
        &output.join("ready.json"),
        &json!({"version":1,"process_id":std::process::id(),"state":"ready_for_measurement"}),
    )?;
    result.proc_before = proc_stat();
    let measure_start = Instant::now();
    result.measurement_start_unix_ns = Some(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "wall clock is before Unix epoch")?
            .as_nanos(),
    );
    let offset = measure_start.duration_since(epoch).as_nanos() as u64;
    let allocated = Arc::new(AtomicU64::new(0));
    let issued = Arc::new(AtomicU64::new(0));
    let cancel = CancellationToken::new();
    let mut workers = JoinSet::new();
    let shared = Arc::new(config.clone());
    for id in 0..config.workers {
        workers.spawn(worker(
            shared.clone(),
            client.clone(),
            id,
            measure_start,
            allocated.clone(),
            issued.clone(),
            cancel.clone(),
        ));
    }
    let mut completed = Vec::new();
    while let Some(worker) = workers.join_next().await {
        match worker {
            Ok(worker) => completed.push(worker),
            Err(_) => {
                result.task_failures += 1;
                cancel.cancel();
            }
        }
    }
    if matches!(config.load, Load::FixedRate { .. }) && !cancel.is_cancelled() {
        tokio::time::sleep_until((measure_start + Duration::from_millis(config.measure_ms)).into())
            .await;
    }
    result.proc_after = proc_stat();
    let nominal_cutoff = config.measure_ms * 1_000_000;
    let stopped = completed.iter().map(|w| w.stopped_ns).max().unwrap_or(0);
    let stop = if cancel.is_cancelled() {
        "worker_failure"
    } else if matches!(config.load, Load::ClosedLoop)
        && allocated.load(Ordering::Relaxed) >= config.max_calls
    {
        "operation_limit"
    } else {
        "duration"
    };
    let cutoff = if stop == "duration" {
        nominal_cutoff
    } else {
        stopped.min(nominal_cutoff)
    };
    let last = completed
        .iter()
        .map(|w| w.last_terminal_ns)
        .max()
        .unwrap_or(0);
    result.cohort_elapsed_ns = cutoff.max(last);
    result.stop_reason = Some(stop);
    result.stages.insert(
        "measurement",
        json!({"start_ns":offset,"end_ns":offset+cutoff}),
    );
    result.stages.insert(
        "drain",
        json!({"start_ns":offset+cutoff,"end_ns":offset+result.cohort_elapsed_ns}),
    );
    completed.sort_by_key(|w| w.id);
    result.measured_issued = issued.load(Ordering::Relaxed);
    for worker in completed {
        let issued = worker.metrics.calls();
        result.dropped += worker.dropped;
        result.workers.push(json!({"worker":worker.id,"issued":issued,"dropped_slots":worker.dropped,
            "last_terminal_ns":worker.last_terminal_ns,"stopped_ns":worker.stopped_ns,"stopped_for_failure":worker.stopped_for_failure}));
        result.measurement.merge(&worker.metrics);
    }
    if result.task_failures > 0
        || cancel.is_cancelled()
        || !result.measurement.valid()
        || result.measurement.data_failures() > 0
        || result.measured_issued == 0
        || result.measurement.calls() != result.measured_issued
        || config
            .offered_slots()
            .is_some_and(|offered| offered != result.measured_issued + result.dropped)
    {
        return Err("batch measurement failed complete accounting or response integrity".into());
    }
    let verify_start = nanos(epoch);
    let verify_deadline = Instant::now() + Duration::from_secs(60);
    for first in (0..=config.keys).step_by(config.batch_size) {
        if Instant::now() >= verify_deadline {
            return Err("verification budget exhausted".into());
        }
        let end = (first + config.batch_size).min(config.keys + 1);
        let start = Instant::now();
        let report = client
            .batch_get((first..end).map(|i| config.key(i)).collect())
            .await;
        let valid = matches!(&report.outcome,Outcome::Success { value:Value::BatchGet { values } }
            if values.len()==end-first && values.iter().enumerate().all(|(i,v)| v.as_ref().is_some_and(|v|
                if first+i==config.keys { *v==config.value(config.keys,0) }
                else { config.valid_value(first+i,v,config.warmup_calls+config.max_calls) })));
        setup_observation(&mut result.verification, &report, end - first, start, valid);
        if !valid {
            return Err("final dataset verification failed".into());
        }
    }
    result.stages.insert(
        "verification",
        json!({"start_ns":verify_start,"end_ns":nanos(epoch)}),
    );
    Ok(())
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    if let Err(error) = execute().await {
        eprintln!("FAIL: {error}");
        std::process::exit(1);
    }
}

async fn execute() -> Result<(), String> {
    let mut config_path = None;
    let mut output = None;
    let mut build_path = None;
    let mut arguments = std::env::args_os().skip(1);
    while let Some(key) = arguments.next() {
        let target=match key.to_str() {
            Some("--config")=>&mut config_path,Some("--output")=>&mut output,Some("--build-manifest")=>&mut build_path,
            _=>return Err("usage: kv9-batch-benchmark --config FILE --output NEW_DIRECTORY --build-manifest FILE".into()),
        };
        if target.is_some() {
            return Err("duplicate benchmark argument".into());
        }
        *target = Some(PathBuf::from(
            arguments.next().ok_or("missing benchmark argument value")?,
        ));
    }
    let config_bytes = read_bounded(&config_path.ok_or("--config is required")?, 65_536)?;
    let config: Config = serde_json::from_slice(&config_bytes)
        .map_err(|_| "invalid benchmark configuration JSON")?;
    let sizes = config.validate()?;
    let build_bytes = read_bounded(&build_path.ok_or("--build-manifest is required")?, 65_536)?;
    let build: Build =
        serde_json::from_slice(&build_bytes).map_err(|_| "invalid benchmark build manifest")?;
    let is_hex = |text: &str, length: usize| {
        text.len() == length
            && text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    };
    if build.version != 1
        || (build.dirty && build.profile == "release")
        || !is_hex(&build.revision, 40)
        || !is_hex(&build.source_tree_sha256, 64)
        || !is_hex(&build.binary_sha256, 64)
        || !["debug", "release"].contains(&build.profile.as_str())
        || build.rustc.is_empty()
        || build.rustc.len() > 4096
        || file_digest(
            &std::env::current_exe().map_err(|_| "cannot resolve benchmark executable")?,
        )? != build.binary_sha256
    {
        return Err(
            "benchmark executable/source binding is invalid or the release source is dirty".into(),
        );
    }
    let token = std::env::var("KV9_CLIENT_TOKEN").map_err(|_| "KV9_CLIENT_TOKEN is required")?;
    let client = PersistentRawClient::new_with_transport(
        config.client.clone(),
        &token,
        config.rpc_transport,
    )?;
    let output = output.ok_or("--output is required")?;
    fs::create_dir(&output).map_err(|_| "benchmark output must be a new directory")?;
    write_new(&output.join("config.json"), &config_bytes)?;
    write_new(&output.join("build.json"), &build_bytes)?;
    let mut result = Run::default();
    let outcome = run(&config, &client, &output, &mut result).await;
    let seconds = result.cohort_elapsed_ns as f64 / 1e9;
    let successes: u64 = result
        .measurement
        .operations
        .iter()
        .map(|op| op.populations[0].calls)
        .sum();
    let successful_items: u64 = result
        .measurement
        .operations
        .iter()
        .map(|op| op.populations[0].input_items)
        .sum();
    let report = json!({"version":1,"workload_model":"bounded_native_batch_performance","full_history_recorded":false,
        "independently_checked":false,"complete":outcome.is_ok(),"failure":outcome.as_ref().err(),
        "configuration":config,"config_sha256":digest(&config_bytes),"build":build,"build_sha256":digest(&build_bytes),
        "wire_sizes":sizes,"process_id":std::process::id(),"runtime_threads":2,
        "measurement_start_unix_ns":result.measurement_start_unix_ns,"stages":result.stages,"stop_reason":result.stop_reason,
        "measured_issued":result.measured_issued,"measured_completed":result.measurement.calls(),
        "offered_slots":config.offered_slots(),"dropped_slots":result.dropped,"task_failures":result.task_failures,
        "cohort_elapsed_ns":result.cohort_elapsed_ns,
        "completed_batches_per_second":if seconds>0.0 {Some(result.measurement.calls() as f64/seconds)} else {None},
        "successful_batches_per_second":if seconds>0.0 {Some(successes as f64/seconds)} else {None},
        "successful_input_items_per_second":if seconds>0.0 {Some(successful_items as f64/seconds)} else {None},
        "timing_eligible":outcome.is_ok() && result.stop_reason==Some("duration") && build.profile=="release" && !build.dirty,
        "proc_stat_before":result.proc_before,"proc_stat_after":result.proc_after,"workers":result.workers,
        "metrics":{"initialization":result.initialization.report(),"warmup":result.warmup.report(),
            "measurement":result.measurement.report(),"verification":result.verification.report()}});
    write_json(&output.join("report.json"), &report)?;
    outcome?;
    println!("PASS: native batch benchmark drained; independent artifact validation is required");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kv9_server::client::{ClientConfig, Peer, TransportKind};

    fn config() -> Config {
        Config {
            version: 1,
            client: ClientConfig {
                version: 1,
                peers: vec![Peer {
                    node_id: 1,
                    address: "127.0.0.1:20160".parse().unwrap(),
                }],
                keyspace_id: 1,
                epoch_conf_ver: 1,
                epoch_version: 1,
                max_in_flight: 16,
                max_attempts: 2,
                deadline_ms: 1500,
                retry_backoff_ms: 1,
            },
            rpc_transport: TransportKind::TonicStream,
            run_id: "batch_probe".into(),
            seed: 71,
            workers: 3,
            keys: 4096,
            batch_size: 16,
            value_bytes: 128,
            read_percent: 50,
            warmup_calls: 100,
            measure_ms: 1000,
            max_calls: 1000,
            load: Load::FixedRate {
                batches_per_second: 17,
            },
        }
    }

    #[test]
    fn fixed_rate_slots_use_integer_boundaries_and_do_not_hide_missed_work() {
        let c = config();
        c.validate().unwrap();
        assert_eq!(c.offered_slots(), Some(17));
        for slot in 0..17 {
            let due = c.slot_due_ns(slot).unwrap();
            let worker = (slot % 3) as usize;
            assert_eq!(latest_due_slot(&c, due, worker), Some(slot));
            if due > 0 {
                assert!(latest_due_slot(&c, due - 1, worker).is_none_or(|previous| previous < slot));
            }
        }
        let mut owned = Vec::new();
        for worker in 0..3 {
            owned.extend((worker..17).step_by(3));
        }
        owned.sort_unstable();
        assert_eq!(owned, (0..17).collect::<Vec<_>>());
    }

    #[test]
    fn dataset_validation_binds_key_position_and_full_value_bytes() {
        let c = config();
        let mut value = c.value(3, 17);
        assert!(c.valid_value(3, &value, 17));
        assert!(!c.valid_value(4, &value, 17));
        assert!(!c.valid_value(3, &value, 16));
        value[37] ^= 1;
        assert!(!c.valid_value(3, &value, 17));
    }

    #[test]
    fn oversized_batch_and_offered_load_are_rejected_before_network_work() {
        let mut c = config();
        c.batch_size = 256;
        c.value_bytes = 8192;
        assert!(c.validate().is_err());
        c.value_bytes = 128;
        c.validate().unwrap();
        c.load = Load::FixedRate {
            batches_per_second: 1001,
        };
        assert!(c.validate().is_err());
        c.load = Load::FixedRate {
            batches_per_second: 0,
        };
        assert!(c.validate().is_err());
    }
}
