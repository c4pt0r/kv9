//! Bounded RESP2 batch/point-read reference. Single-instance Redis, no Raft claims.
#[path = "../batch/artifact.rs"]
mod artifact;
#[path = "../../../../crates/server/src/bin/kv9-batch-benchmark/common.rs"]
mod common;
#[path = "../batch/metrics.rs"]
mod metrics;
#[path = "../batch/model.rs"]
mod model;
#[path = "../batch/wire.rs"]
mod wire;
use artifact::*;
use common::Load;
use metrics::{Metrics, Sample};
use model::{Config, ReadApi};
use serde_json::{json, Value as Json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::task::JoinSet;
use wire::{Connection, Operation, Reply};

struct Timing {
    epoch: Instant,
    cutoff: Option<Instant>,
    scheduled: Option<Instant>,
}
async fn traffic(
    c: &Config,
    conn: &mut Option<Connection>,
    nonce: u64,
    t: Timing,
    m: &mut Metrics,
    issued: Option<&AtomicU64>,
) -> Option<(bool, bool, u64)> {
    let start = Instant::now();
    if t.cutoff.is_some_and(|cutoff| start >= cutoff) {
        return None;
    }
    if let Some(issued) = issued {
        issued.fetch_add(1, Ordering::Relaxed);
    }
    let read = c.is_read(nonce);
    let first = c.first_key(nonce);
    let mut op = Operation::new(
        c,
        read,
        (0..c.batch_size).map(|i| (first + i) % c.keys),
        nonce,
    );
    op.read_api = c.effective_read_api();
    let call = wire::call(conn, c, &op).await;
    let valid = match &call.result {
        Ok(Reply::Applied) => !read,
        Ok(Reply::Value(value)) => {
            read && c.effective_read_api() == ReadApi::Get
                && value
                    .as_ref()
                    .is_some_and(|value| c.valid_value(first, value, c.warmup_calls + c.max_calls))
        }
        Ok(Reply::Values(values)) => {
            read && c.effective_read_api() == ReadApi::Mget
                && values.len() == c.batch_size
                && values.iter().enumerate().all(|(i, v)| {
                    v.as_ref().is_some_and(|v| {
                        c.valid_value((first + i) % c.keys, v, c.warmup_calls + c.max_calls)
                    })
                })
        }
        Err(e) => !e.fatal(),
    };
    let success = call.result.is_ok() && valid;
    let end = Instant::now();
    m.record(Sample {
        call: &call,
        read,
        items: c.batch_size,
        whole_call_ns: end.duration_since(start).as_nanos() as u64,
        before_cutoff: t.cutoff.is_none_or(|cutoff| end < cutoff),
        scheduled_ns: t
            .scheduled
            .map(|at| end.duration_since(at).as_nanos() as u64),
        lateness_ns: t
            .scheduled
            .map(|at| start.duration_since(at).as_nanos() as u64),
        valid,
    });
    Some((
        success,
        valid,
        end.duration_since(t.epoch).as_nanos() as u64,
    ))
}
struct Worker {
    id: usize,
    metrics: Metrics,
    dropped: u64,
    last: u64,
    stopped: u64,
    failed: bool,
}
struct Shared {
    config: Arc<Config>,
    epoch: Instant,
    allocated: AtomicU64,
    issued: AtomicU64,
    cancel: AtomicBool,
}
async fn worker(shared: Arc<Shared>, id: usize, connection: Connection) -> Worker {
    let c = &shared.config;
    let cutoff = shared.epoch + Duration::from_millis(c.measure_ms);
    let mut conn = Some(connection);
    let mut w = Worker {
        id,
        metrics: Metrics::default(),
        dropped: 0,
        last: 0,
        stopped: 0,
        failed: false,
    };
    let mut next = id as u64;
    loop {
        if shared.cancel.load(Ordering::Relaxed) {
            w.failed = true;
            break;
        }
        let (slot, scheduled) = match c.load {
            Load::ClosedLoop => {
                if Instant::now() >= cutoff {
                    break;
                }
                let slot =
                    match shared
                        .allocated
                        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                            (n < c.max_calls).then_some(n + 1)
                        }) {
                        Ok(n) => n,
                        Err(_) => break,
                    };
                (slot, None)
            }
            Load::FixedRate { .. } => {
                let total = c.offered_slots().unwrap();
                if next >= total {
                    break;
                }
                let at = shared.epoch + Duration::from_nanos(c.slot_due_ns(next).unwrap());
                if Instant::now() < at {
                    tokio::time::sleep_until(at.into()).await;
                }
                if shared.cancel.load(Ordering::Relaxed) {
                    w.failed = true;
                    break;
                }
                if Instant::now() >= cutoff {
                    w.dropped += 1 + (total - 1 - next) / c.workers as u64;
                    break;
                }
                if let Some(latest) = common::latest_due_slot(
                    c.load,
                    c.measure_ms,
                    nanos(shared.epoch),
                    id,
                    c.workers,
                ) {
                    if latest > next {
                        w.dropped += (latest - next) / c.workers as u64;
                        next = latest;
                    }
                }
                let slot = next;
                next += c.workers as u64;
                (
                    slot,
                    Some(shared.epoch + Duration::from_nanos(c.slot_due_ns(slot).unwrap())),
                )
            }
        };
        match traffic(
            c,
            &mut conn,
            c.warmup_calls + slot + 1,
            Timing {
                epoch: shared.epoch,
                cutoff: Some(cutoff),
                scheduled,
            },
            &mut w.metrics,
            Some(&shared.issued),
        )
        .await
        {
            Some((_, valid, end)) => {
                w.last = end;
                if !valid {
                    w.failed = true;
                    shared.cancel.store(true, Ordering::Relaxed);
                    break;
                }
            }
            None => {
                if scheduled.is_some() {
                    w.dropped += 1;
                }
            }
        }
    }
    w.stopped = nanos(shared.epoch);
    w
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
    preconnected_workers: usize,
}
async fn setup(
    c: &Config,
    conn: &mut Option<Connection>,
    m: &mut Metrics,
    first: usize,
    end: usize,
    write: bool,
    verify: bool,
) -> bool {
    let start = Instant::now();
    let op = Operation::new(c, !write, first..end, 0);
    let call = wire::call(conn, c, &op).await;
    let valid = match &call.result {
        Ok(Reply::Applied) => write,
        Ok(Reply::Value(_)) => false,
        Ok(Reply::Values(values)) => {
            !write
                && values.len() == end - first
                && values.iter().enumerate().all(|(i, v)| {
                    if !verify {
                        v.is_none()
                    } else {
                        v.as_ref().is_some_and(|v| {
                            if first + i == c.keys {
                                *v == c.value(c.keys, 0)
                            } else {
                                c.valid_value(first + i, v, c.warmup_calls + c.max_calls)
                            }
                        })
                    }
                })
        }
        Err(_) => false,
    };
    m.record(Sample {
        call: &call,
        read: !write,
        items: end - first,
        whole_call_ns: nanos(start),
        before_cutoff: true,
        scheduled_ns: None,
        lateness_ns: None,
        valid,
    });
    valid
}
async fn run(c: Arc<Config>, out: &Path, r: &mut Run) -> Result<(), String> {
    let epoch = Instant::now();
    let setup_deadline = epoch + Duration::from_secs(300);
    let mut conn = None;
    for first in (0..=c.keys).step_by(c.batch_size) {
        if Instant::now() >= setup_deadline {
            return Err("initialization budget exhausted".into());
        }
        let end = (first + c.batch_size).min(c.keys + 1);
        if !setup(
            &c,
            &mut conn,
            &mut r.initialization,
            first,
            end,
            false,
            false,
        )
        .await
        {
            return Err("reference requires a fresh empty dataset".into());
        }
        if !setup(
            &c,
            &mut conn,
            &mut r.initialization,
            first,
            end,
            true,
            false,
        )
        .await
        {
            return Err("dataset initialization was unconfirmed".into());
        }
    }
    r.stages.insert(
        "initialization",
        json!({"start_ns":0,"end_ns":nanos(epoch)}),
    );
    let begin = nanos(epoch);
    for nonce in 1..=c.warmup_calls {
        if Instant::now() >= setup_deadline {
            return Err("warmup budget exhausted".into());
        }
        let (success, valid, _) = traffic(
            &c,
            &mut conn,
            nonce,
            Timing {
                epoch,
                cutoff: None,
                scheduled: None,
            },
            &mut r.warmup,
            None,
        )
        .await
        .unwrap();
        if !success || !valid {
            return Err("reference warmup failed".into());
        }
    }
    r.stages
        .insert("warmup", json!({"start_ns":begin,"end_ns":nanos(epoch)}));
    let mut connections = Vec::new();
    let connection_deadline = tokio::time::Instant::now() + Duration::from_millis(c.deadline_ms);
    for _ in 0..c.workers {
        let connection = tokio::time::timeout_at(connection_deadline, wire::connect(&c))
            .await
            .map_err(|_| "worker connection budget exhausted")?
            .map_err(|_| "worker connection failed")?;
        connections.push(connection);
        r.preconnected_workers += 1;
    }
    write_json(
        &out.join("ready.json"),
        &json!({"version":1,"process_id":std::process::id(),"state":"ready_for_measurement"}),
    )?;
    r.proc_before = proc_stat();
    let start = Instant::now();
    let offset = start.duration_since(epoch).as_nanos() as u64;
    r.measurement_start_unix_ns = Some(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "wall clock precedes epoch")?
            .as_nanos(),
    );
    let shared = Arc::new(Shared {
        config: c.clone(),
        epoch: start,
        allocated: AtomicU64::new(0),
        issued: AtomicU64::new(0),
        cancel: AtomicBool::new(false),
    });
    let mut tasks = JoinSet::new();
    for (id, connection) in connections.into_iter().enumerate() {
        tasks.spawn(worker(shared.clone(), id, connection));
    }
    let mut workers = Vec::new();
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(w) => workers.push(w),
            Err(_) => {
                r.task_failures += 1;
                shared.cancel.store(true, Ordering::Relaxed);
            }
        }
    }
    if matches!(c.load, Load::FixedRate { .. }) && !shared.cancel.load(Ordering::Relaxed) {
        tokio::time::sleep_until((start + Duration::from_millis(c.measure_ms)).into()).await;
    }
    r.proc_after = proc_stat();
    let stopped = workers.iter().map(|w| w.stopped).max().unwrap_or(0);
    let last = workers.iter().map(|w| w.last).max().unwrap_or(0);
    let nominal = c.measure_ms * 1_000_000;
    let reason = if shared.cancel.load(Ordering::Relaxed) {
        "worker_failure"
    } else if matches!(c.load, Load::ClosedLoop)
        && shared.allocated.load(Ordering::Relaxed) >= c.max_calls
    {
        "operation_limit"
    } else {
        "duration"
    };
    let cutoff = if reason == "duration" {
        nominal
    } else {
        stopped.min(nominal)
    };
    r.cohort_elapsed_ns = cutoff.max(last);
    r.stop_reason = Some(reason);
    r.stages.insert(
        "measurement",
        json!({"start_ns":offset,"end_ns":offset+cutoff}),
    );
    r.stages.insert(
        "drain",
        json!({"start_ns":offset+cutoff,"end_ns":offset+r.cohort_elapsed_ns}),
    );
    workers.sort_by_key(|w| w.id);
    r.measured_issued = shared.issued.load(Ordering::Relaxed);
    for w in workers {
        r.dropped += w.dropped;
        r.workers.push(json!({"worker":w.id,"issued":w.metrics.calls(),"dropped_slots":w.dropped,"last_terminal_ns":w.last,"stopped_ns":w.stopped,"stopped_for_failure":w.failed}));
        r.measurement.merge(&w.metrics);
    }
    if r.task_failures > 0
        || shared.cancel.load(Ordering::Relaxed)
        || !r.measurement.valid()
        || r.measured_issued == 0
        || r.measurement.calls() != r.measured_issued
        || c.offered_slots()
            .is_some_and(|n| n != r.measured_issued + r.dropped)
    {
        return Err("measurement accounting or integrity failed".into());
    }
    let begin = nanos(epoch);
    let verification_deadline = Instant::now() + Duration::from_secs(60);
    for first in (0..=c.keys).step_by(c.batch_size) {
        if Instant::now() >= verification_deadline {
            return Err("verification budget exhausted".into());
        }
        if !setup(
            &c,
            &mut conn,
            &mut r.verification,
            first,
            (first + c.batch_size).min(c.keys + 1),
            false,
            true,
        )
        .await
        {
            return Err("final dataset verification failed".into());
        }
    }
    r.stages.insert(
        "verification",
        json!({"start_ns":begin,"end_ns":nanos(epoch)}),
    );
    Ok(())
}

async fn execute() -> Result<(), String> {
    let mut config_path = None;
    let mut output = None;
    let mut build_path = None;
    let mut arguments = std::env::args_os().skip(1);
    while let Some(key) = arguments.next() {
        let target=match key.to_str() {
            Some("--config")=>&mut config_path,Some("--output")=>&mut output,Some("--build-manifest")=>&mut build_path,
            _=>return Err("usage: kv9-redis-batch-reference --config FILE --output NEW_DIRECTORY --build-manifest FILE".into()),
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
    let output = output.ok_or("--output is required")?;
    fs::create_dir(&output).map_err(|_| "benchmark output must be a new directory")?;
    write_new(&output.join("config.json"), &config_bytes)?;
    write_new(&output.join("build.json"), &build_bytes)?;
    let mut result = Run::default();
    let outcome = run(Arc::new(config.clone()), &output, &mut result).await;
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
    let report = json!({"version":config.version,
        "workload_model":if config.effective_read_api()==ReadApi::Get {"bounded_redis_point_get_diagnostic"}else{"bounded_redis_batch_performance"},
        "full_history_recorded":false,
        "independently_checked":false,"complete":outcome.is_ok(),"failure":outcome.as_ref().err(),
        "configuration":config,"config_sha256":digest(&config_bytes),"build":build,"build_sha256":digest(&build_bytes),
        "wire_sizes":sizes,"process_id":std::process::id(),"runtime_threads":2,"protocol":"resp2","preconnected_workers":result.preconnected_workers,
        "measurement_start_unix_ns":result.measurement_start_unix_ns,"stages":result.stages,"stop_reason":result.stop_reason,
        "measured_issued":result.measured_issued,"measured_completed":result.measurement.calls(),
        "offered_slots":config.offered_slots(),"dropped_slots":result.dropped,"task_failures":result.task_failures,
        "cohort_elapsed_ns":result.cohort_elapsed_ns,
        "completed_batches_per_second":if seconds>0.0 {Some(result.measurement.calls() as f64/seconds)} else {None},
        "successful_batches_per_second":if seconds>0.0 {Some(successes as f64/seconds)} else {None},
        "successful_input_items_per_second":if seconds>0.0 {Some(successful_items as f64/seconds)} else {None},
        "timing_eligible":outcome.is_ok() && result.stop_reason==Some("duration") && build.profile=="release" && !build.dirty,
        "proc_stat_before":result.proc_before,"proc_stat_after":result.proc_after,"workers":result.workers,
        "metrics":{"initialization":result.initialization.report(),"warmup":result.warmup.report_for_read_api(config.effective_read_api()),
            "measurement":result.measurement.report_for_read_api(config.effective_read_api()),"verification":result.verification.report()}});
    write_json(&output.join("report.json"), &report)?;
    outcome?;
    println!("PASS: Redis batch reference drained; independent artifact validation is required");
    Ok(())
}

fn main() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("cannot start reference runtime");
    if let Err(error) = runtime.block_on(execute()) {
        eprintln!("FAIL: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn selected_traffic_sends_get_and_missing_populated_value_is_a_failure() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                tokio::time::timeout(Duration::from_secs(5), async {
                    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                    let mut c: Config =
                        serde_json::from_value(model::tests::config_json()).unwrap();
                    c.version = 2;
                    c.read_api = Some(ReadApi::Get);
                    c.address = listener.local_addr().unwrap();
                    c.validate().unwrap();
                    let expected: Vec<_> = [1, 2]
                        .into_iter()
                        .map(|nonce| {
                            let key = c.key(c.first_key(nonce));
                            let mut frame =
                                format!("*2\r\n$3\r\nGET\r\n${}\r\n", key.len()).into_bytes();
                            frame.extend_from_slice(&key);
                            frame.extend_from_slice(b"\r\n");
                            frame
                        })
                        .collect();
                    let value = c.value(c.first_key(1), 0);
                    let peer = tokio::spawn(async move {
                        let (mut stream, _) = listener.accept().await.unwrap();
                        for (index, request) in expected.iter().enumerate() {
                            let mut actual = vec![0; request.len()];
                            stream.read_exact(&mut actual).await.unwrap();
                            assert_eq!(
                                &actual, request,
                                "selected GET traffic sent a different command"
                            );
                            let response = if index == 0 {
                                let mut bytes = format!("${}\r\n", value.len()).into_bytes();
                                bytes.extend_from_slice(&value);
                                bytes.extend_from_slice(b"\r\n");
                                bytes
                            } else {
                                b"$-1\r\n".to_vec()
                            };
                            stream.write_all(&response).await.unwrap();
                        }
                    });
                    let mut connection = None;
                    let mut metrics = Metrics::default();
                    let issued = AtomicU64::new(0);
                    let mut outcomes = Vec::new();
                    for nonce in [1, 2] {
                        let (success, valid, _) = traffic(
                            &c,
                            &mut connection,
                            nonce,
                            Timing {
                                epoch: Instant::now(),
                                cutoff: None,
                                scheduled: None,
                            },
                            &mut metrics,
                            Some(&issued),
                        )
                        .await
                        .unwrap();
                        outcomes.push((success, valid));
                    }
                    assert_eq!(outcomes, [(true, true), (false, false)]);
                    assert_eq!(issued.load(Ordering::Relaxed), 2);
                    assert_eq!(metrics.operations[0].populations[0].calls, 1);
                    assert_eq!(metrics.operations[0].populations[2].calls, 1);
                    assert_eq!(metrics.operations[0].reasons[5], 1);
                    assert_eq!(metrics.operations[0].command_attempts, 2);
                    assert!(!metrics.valid());
                    peer.await.unwrap();
                })
                .await
                .expect("bounded owned GET traffic did not finish");
            });
    }
}
