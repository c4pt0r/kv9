//! Bounded correctness workload. Every atomic batch has one invocation/result.
//! Throughput measurements require a separate, accepted performance run mode.
use kv9_common::metrics::{Latency, Outcome as MetricOutcome};
use kv9_server::client::{
    CallReport, ClientConfig, Outcome, PersistentRawClient, RawOperation, Reason, TransportKind,
    Value,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::task::JoinSet;

#[cfg(test)]
#[path = "kv9-batch-workload/tests.rs"]
mod tests;

const MAX_HISTORY: u64 = 268_435_456;
const KINDS: [&str; 5] = ["get", "put", "delete", "batch_get", "batch_put"];
const EXTRA_PHASES: [&str; 5] = [
    "client-link-delay",
    "client-link-loss",
    "client-link-partition",
    "client-stream-drop",
    "quorum-loss",
];

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Config {
    version: u32,
    client: ClientConfig,
    #[serde(default)]
    rpc_transport: TransportKind,
    run_id: String,
    keyspace_name: String,
    workers: usize,
    seed: u64,
    keys: usize,
    batch_size: usize,
    value_bytes: usize,
    mix: [u8; 5],
    max_calls: u64,
    history_bytes: u64,
    measure_ms: u64,
    interval_ms: u64,
}
impl Config {
    fn allowance(&self) -> u64 {
        16_384 + 8 * self.batch_size as u64 * (self.run_id.len() + 17 + self.value_bytes) as u64
    }
    fn validate(&self) -> Result<(), String> {
        self.client.validate()?;
        let valid_name = |s: &str| {
            !s.is_empty()
                && s.len() <= 64
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        };
        if self.version != 1
            || !valid_name(&self.run_id)
            || !valid_name(&self.keyspace_name)
            || self.client.keyspace_id == 0
            || self.client.keyspace_id >= 1 << 24
            || self.client.epoch_conf_ver != 1
            || self.client.epoch_version != 1
            || !(1..=self.client.max_in_flight).contains(&self.workers)
            || !(1..=256).contains(&self.keys)
            || !(1..=256).contains(&self.batch_size)
            || !(16..=8192).contains(&self.value_bytes)
            || self.mix.iter().map(|n| u32::from(*n)).sum::<u32>() != 100
            || !(1..=3_600_000).contains(&self.measure_ms)
            || self.interval_ms > 1000
            || self.max_calls > 100_000
            || self.max_calls
                < (3 * (self.keys + 1).div_ceil(self.batch_size) + self.workers + 2) as u64
            || self.history_bytes > MAX_HISTORY
            || self.history_bytes < 65536 + self.allowance() * self.max_calls
            || self.batch_size * (self.run_id.len() + 17 + self.value_bytes + 32) + 32
                > kv9_server::client::MAX_MESSAGE_BYTES
        {
            return Err("invalid bounded batch workload configuration".into());
        }
        Ok(())
    }
    fn key(&self, i: usize) -> Vec<u8> {
        format!("{}:{i:016x}", self.run_id).into_bytes()
    }
    fn value(&self, nonce: u64, item: usize) -> Vec<u8> {
        if nonce == 0 && item == self.keys {
            return Vec::new();
        }
        let mut value = vec![b'v'; self.value_bytes];
        value[..8].copy_from_slice(&nonce.to_be_bytes());
        value[8..16].copy_from_slice(&(item as u64).to_be_bytes());
        value
    }
    fn operation(&self, nonce: u64) -> RawOperation {
        // A separate nonlinear key projection avoids coupling operation kind
        // and key residues (for example keys=100 with a 100-slot mix).
        let mut key_hash = (nonce ^ self.seed).wrapping_add(0x9e3779b97f4a7c15);
        key_hash = (key_hash ^ (key_hash >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        key_hash = (key_hash ^ (key_hash >> 27)).wrapping_mul(0x94d049bb133111eb);
        key_hash ^= key_hash >> 31;
        let point = (key_hash % self.keys as u64) as usize;
        let selected = ((nonce * 73 + 19 + self.seed % 100) % 100) as u8;
        let mut cumulative = 0;
        let kind = self
            .mix
            .iter()
            .position(|weight| {
                cumulative += *weight;
                selected < cumulative
            })
            .unwrap();
        match kind {
            0 => RawOperation::Get {
                key: self.key(point),
            },
            1 => RawOperation::Put {
                key: self.key(point),
                value: self.value(nonce + 1, 0),
            },
            2 => RawOperation::Delete {
                key: self.key(point),
            },
            3 => RawOperation::BatchGet {
                keys: (0..self.batch_size)
                    .map(|i| self.key((point + i) % self.keys))
                    .collect(),
            },
            _ => RawOperation::BatchPut {
                pairs: (0..self.batch_size)
                    .map(|i| (self.key((point + i) % self.keys), self.value(nonce + 1, i)))
                    .collect(),
            },
        }
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn ns(start: Instant) -> u64 {
    start.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
}
fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn read(path: &Path, bound: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|_| "cannot open bounded input")?
        .take(bound + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "cannot read bounded input")?;
    if bytes.len() as u64 > bound {
        return Err("input exceeds bound".into());
    }
    Ok(bytes)
}
fn file_sha(path: &Path, bound: u64) -> Result<String, String> {
    let mut file = File::open(path).map_err(|_| "cannot open artifact")?;
    let mut hash = Sha256::new();
    let mut size = 0;
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(|_| "cannot hash artifact")?;
        if n == 0 {
            break;
        }
        size += n as u64;
        if size > bound {
            return Err("artifact exceeds bound".into());
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
        .map_err(|_| "cannot create artifact".into())
}
fn atomic_json(path: &Path, value: &Json) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|_| "cannot encode report")?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("report exceeds bound".into());
    }
    let temporary = path.with_extension("tmp");
    let mut file = File::create(&temporary).map_err(|_| "cannot create report")?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "cannot persist report")?;
    fs::rename(temporary, path).map_err(|_| "cannot publish report".into())
}
fn phase_valid(phase: &str) -> bool {
    kv9_server::workload::PHASES.contains(&phase) || EXTRA_PHASES.contains(&phase)
}
fn phase(path: Option<&Path>) -> Result<String, String> {
    let Some(path) = path else {
        return Ok("measure".into());
    };
    let bytes = read(path, 128)?;
    let value = std::str::from_utf8(&bytes)
        .map_err(|_| "invalid phase encoding")?
        .trim();
    if !phase_valid(value) || matches!(value, "initialization" | "warmup" | "verify") {
        return Err("invalid batch workload phase".into());
    }
    Ok(value.into())
}
fn arguments(op: &RawOperation, keyspace: u32) -> Json {
    match op {
        RawOperation::Get { key } | RawOperation::Delete { key } => {
            json!({"keyspace":keyspace,"key":hex(key)})
        }
        RawOperation::Put { key, value } => {
            json!({"keyspace":keyspace,"key":hex(key),"value":hex(value)})
        }
        RawOperation::BatchGet { keys } => {
            json!({"keyspace":keyspace,"keys":keys.iter().map(|k|hex(k)).collect::<Vec<_>>()})
        }
        RawOperation::BatchPut { pairs } => {
            json!({"keyspace":keyspace,"pairs":pairs.iter().map(|(k,v)|[hex(k),hex(v)]).collect::<Vec<_>>()})
        }
    }
}
fn items(op: &RawOperation) -> usize {
    match op {
        RawOperation::BatchGet { keys } => keys.len(),
        RawOperation::BatchPut { pairs } => pairs.len(),
        _ => 1,
    }
}
fn kind(op: &RawOperation) -> &'static str {
    match op {
        RawOperation::Get { .. } => KINDS[0],
        RawOperation::Put { .. } => KINDS[1],
        RawOperation::Delete { .. } => KINDS[2],
        RawOperation::BatchGet { .. } => KINDS[3],
        RawOperation::BatchPut { .. } => KINDS[4],
    }
}

#[derive(Default)]
struct Metric {
    calls: u64,
    input_items: u64,
    successful_items: u64,
    unknown_write_items: u64,
    refused_items: u64,
    attempts: u64,
    reasons: BTreeMap<String, u64>,
    logical: Latency,
    attempt: Latency,
}
#[derive(Clone)]
struct Active {
    worker: usize,
    phase: String,
    kind: &'static str,
    items: usize,
}
struct State {
    file: File,
    issued: u64,
    terminal: u64,
    sequence: u64,
    bytes: u64,
    reserved: u64,
    peak: usize,
    active: BTreeMap<u64, Active>,
    metrics: BTreeMap<(String, &'static str), Metric>,
    failure: Option<String>,
}
struct Recorder {
    config: Config,
    start: Instant,
    state: Mutex<State>,
}
impl Recorder {
    fn new(config: Config, output: &Path, start: Instant) -> Result<Self, String> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output.join("history.jsonl"))
            .map_err(|_| "cannot create history")?;
        let mut state = State {
            file,
            issued: 0,
            terminal: 0,
            sequence: 0,
            bytes: 0,
            reserved: 0,
            peak: 0,
            active: BTreeMap::new(),
            metrics: BTreeMap::new(),
            failure: None,
        };
        Self::append(
            &mut state,
            &json!({"type":"header","version":2,"range_chunk_size":1024,"generator":"kv9-native-batch-workload","configuration":config,"initial":{"keyspaces":[{"name":config.keyspace_name,"id":config.client.keyspace_id}],"kv":[]}}),
            config.history_bytes,
        )?;
        Ok(Self {
            config,
            start,
            state: Mutex::new(state),
        })
    }
    fn append(state: &mut State, value: &Json, limit: u64) -> Result<(), String> {
        let mut bytes = serde_json::to_vec(value).map_err(|_| "cannot encode history")?;
        bytes.push(b'\n');
        if state.bytes + state.reserved + bytes.len() as u64 > limit {
            return Err("history reservation exhausted".into());
        }
        state
            .file
            .write_all(&bytes)
            .map_err(|_| "cannot write complete history")?;
        state.bytes += bytes.len() as u64;
        Ok(())
    }
    fn begin(
        &self,
        worker: usize,
        phase: &str,
        nonce: Option<u64>,
        op: &RawOperation,
    ) -> Result<u64, String> {
        let mut s = self.state.lock().map_err(|_| "history lock poisoned")?;
        if s.failure.is_some()
            || worker >= self.config.workers
            || s.active.values().any(|a| a.worker == worker)
            || s.issued >= self.config.max_calls
            || !phase_valid(phase)
        {
            return Err("cannot admit history invocation".into());
        }
        let id = s.issued;
        let sequence = s.sequence;
        s.reserved += self.config.allowance();
        let event = json!({"type":"invoke","seq":sequence,"monotonic_ns":ns(self.start),"id":id,"client":worker.to_string(),"phase":phase,"nonce":nonce,"op":kind(op),"args":arguments(op,self.config.client.keyspace_id)});
        if let Err(error) = Self::append(&mut s, &event, self.config.history_bytes) {
            s.reserved -= self.config.allowance();
            s.failure = Some(error.clone());
            return Err(error);
        }
        s.sequence += 1;
        s.issued += 1;
        s.active.insert(
            id,
            Active {
                worker,
                phase: phase.into(),
                kind: kind(op),
                items: items(op),
            },
        );
        s.peak = s.peak.max(s.active.len());
        Ok(id)
    }
    fn complete(&self, id: u64, op: &RawOperation, report: &CallReport) -> Result<(), String> {
        let mut s = self.state.lock().map_err(|_| "history lock poisoned")?;
        let active = s
            .active
            .get(&id)
            .cloned()
            .ok_or("missing batch invocation")?;
        let operation_matches =
            report.operation == op.kind() && active.kind == kind(op) && active.items == items(op);
        let local = report.attempts.is_empty();
        let (outcome, result, reason, receipt, group) = match &report.outcome {
            Outcome::Success {
                value: Value::Get { value },
            } if matches!(op, RawOperation::Get { .. }) => (
                "ok",
                json!({"value":value.as_ref().map(|v|hex(v))}),
                None,
                None,
                MetricOutcome::Success,
            ),
            Outcome::Success {
                value: Value::BatchGet { values },
            } if matches!(op,RawOperation::BatchGet { keys } if keys.len()==values.len()) => (
                "ok",
                json!({"values":values.iter().map(|v|v.as_ref().map(|v|hex(v))).collect::<Vec<_>>()}),
                None,
                None,
                MetricOutcome::Success,
            ),
            Outcome::Success {
                value: Value::Applied { term, index },
            } if !op.kind().is_read() && *term > 0 && *index > 0 => (
                "ok",
                json!({}),
                None,
                Some(json!({"applied_term":term,"applied_index":index})),
                MetricOutcome::Success,
            ),
            Outcome::Refused {
                reason:
                    reason @ (Reason::NotLeader { .. }
                    | Reason::AdmissionCount
                    | Reason::AdmissionBytes
                    | Reason::AdmissionOversize),
            } => (
                "refused",
                json!({"proof":"precommit"}),
                Some(reason),
                None,
                MetricOutcome::Rejected,
            ),
            Outcome::ClientRejected { reason } if local => (
                "refused",
                json!({"proof":"precommit"}),
                Some(reason),
                None,
                MetricOutcome::Released,
            ),
            Outcome::ReadFailure { reason } if op.kind().is_read() => (
                "unknown",
                json!({}),
                Some(reason),
                None,
                MetricOutcome::Error,
            ),
            Outcome::UnknownWrite { reason } if !op.kind().is_read() => (
                "unknown",
                json!({}),
                Some(reason),
                None,
                MetricOutcome::Unconfirmed,
            ),
            _ => {
                s.failure = Some("invalid batch terminal contract".into());
                return Err("invalid batch terminal contract".into());
            }
        };
        if !operation_matches
            || report.attempts.len() > self.config.client.max_attempts
            || reason == Some(&Reason::Protocol)
        {
            s.failure = Some("invalid batch operation/protocol observation".into());
            return Err("invalid batch operation/protocol observation".into());
        }
        let sequence = s.sequence;
        let event = json!({"type":"return","seq":sequence,"monotonic_ns":ns(self.start),"id":id,"outcome":outcome,"result":result,"observation":{"attempts":report.attempts,"elapsed_ns":report.elapsed_ns,"stop":report.stop,"reason":reason,"receipt":receipt,"malformed":if reason==Some(&Reason::Protocol){Some("protocol_response")}else{None}}});
        let encoded = serde_json::to_vec(&event).map_err(|_| "cannot encode terminal")?;
        if encoded.len() as u64 + 1 > self.config.allowance() {
            s.failure = Some("terminal exceeds its reservation".into());
            return Err("terminal exceeds its reservation".into());
        }
        s.reserved -= self.config.allowance();
        if let Err(error) = Self::append(&mut s, &event, self.config.history_bytes) {
            s.reserved += self.config.allowance();
            s.failure = Some(error.clone());
            return Err(error);
        }
        s.active.remove(&id);
        s.terminal += 1;
        let m = s.metrics.entry((active.phase, active.kind)).or_default();
        m.calls += 1;
        m.input_items += active.items as u64;
        if outcome == "ok" {
            m.successful_items += active.items as u64;
        } else if outcome == "refused" {
            m.refused_items += active.items as u64;
        } else if !op.kind().is_read() {
            m.unknown_write_items += active.items as u64;
        }
        let population = reason
            .map(|r| {
                serde_json::to_value(r).unwrap()["kind"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .unwrap_or_else(|| "success".into());
        *m.reasons.entry(population).or_default() += 1;
        m.logical
            .record(Duration::from_nanos(report.elapsed_ns), group);
        for attempt in &report.attempts {
            m.attempts += 1;
            let group = match &attempt.failure {
                None => MetricOutcome::Success,
                Some(
                    Reason::NotLeader { .. }
                    | Reason::AdmissionCount
                    | Reason::AdmissionBytes
                    | Reason::AdmissionOversize,
                ) => MetricOutcome::Rejected,
                _ if op.kind().is_read() => MetricOutcome::Error,
                _ => MetricOutcome::Unconfirmed,
            };
            m.attempt
                .record(Duration::from_nanos(attempt.elapsed_ns), group);
        }
        s.sequence += 1;
        Ok(())
    }
    async fn call(
        &self,
        client: &PersistentRawClient,
        worker: usize,
        phase: &str,
        nonce: Option<u64>,
        op: RawOperation,
    ) -> Result<CallReport, String> {
        let id = self.begin(worker, phase, nonce, &op)?;
        let report = client.call(op.clone()).await;
        self.complete(id, &op, &report)?;
        Ok(report)
    }
    fn progress(&self) -> Json {
        let s = self.state.lock().unwrap();
        json!({"issued":s.issued,"terminal":s.terminal,"in_flight":s.active.len(),"peak_in_flight":s.peak,"successes":s.metrics.iter().map(|((phase,op),m)|json!({"phase":phase,"operation":op,"calls":m.logical.snapshot().outcomes[0].count})).collect::<Vec<_>>()})
    }
    fn ensure_traffic(&self) -> Result<(), String> {
        let state = self.state.lock().map_err(|_| "history lock poisoned")?;
        if !state.metrics.iter().any(|((phase, _), metric)| {
            phase != "initialization" && phase != "verify" && metric.calls > 0
        }) {
            return Err("batch workload produced no traffic".into());
        }
        Ok(())
    }
    fn finish(&self) -> Result<Json, String> {
        let s = self.state.lock().map_err(|_| "history lock poisoned")?;
        s.file.sync_all().map_err(|_| "cannot persist history")?;
        let metrics=s.metrics.iter().map(|((phase,op),m)|json!({"phase":phase,"operation":op,"calls":m.calls,"input_items":m.input_items,"successful_items":m.successful_items,"unknown_write_items":m.unknown_write_items,"refused_items":m.refused_items,"attempts":m.attempts,"reasons":m.reasons,"logical_latency":m.logical.snapshot(),"attempt_latency":m.attempt.snapshot()})).collect::<Vec<_>>();
        Ok(
            json!({"issued":s.issued,"terminal":s.terminal,"events":s.sequence,"bytes":s.bytes,"peak_in_flight":s.peak,"accounting_complete":s.issued==s.terminal && s.active.is_empty() && s.reserved==0,"failure":s.failure,"metrics":metrics}),
        )
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    if let Err(error) = execute().await {
        eprintln!("FAIL: {error}");
        std::process::exit(1);
    }
}
async fn execute() -> Result<(), String> {
    let mut args = BTreeMap::new();
    let mut input = std::env::args_os().skip(1);
    while let Some(key) = input.next() {
        let key = key.into_string().map_err(|_| "invalid argument")?;
        if ![
            "--config",
            "--output",
            "--build-manifest",
            "--stop-file",
            "--phase-file",
        ]
        .contains(&key.as_str())
            || args.contains_key(&key)
        {
            return Err("invalid or duplicate batch workload argument".into());
        }
        args.insert(
            key,
            PathBuf::from(input.next().ok_or("missing argument value")?),
        );
    }
    let config: Config = serde_json::from_slice(&read(
        args.get("--config").ok_or("--config required")?,
        65536,
    )?)
    .map_err(|_| "invalid batch configuration")?;
    config.validate()?;
    let build_bytes = read(
        args.get("--build-manifest")
            .ok_or("--build-manifest required")?,
        65536,
    )?;
    let build: Json = serde_json::from_slice(&build_bytes).map_err(|_| "invalid build manifest")?;
    if build["binary_sha256"].as_str()
        != Some(
            file_sha(
                &std::env::current_exe().map_err(|_| "cannot identify executable")?,
                536870912,
            )?
            .as_str(),
        )
    {
        return Err("build manifest does not bind executing workload".into());
    }
    let token = std::env::var("KV9_CLIENT_TOKEN").map_err(|_| "KV9_CLIENT_TOKEN required")?;
    let client = PersistentRawClient::new_with_transport(
        config.client.clone(),
        &token,
        config.rpc_transport,
    )?;
    let output = args.get("--output").ok_or("--output required")?.clone();
    fs::create_dir(&output).map_err(|_| "output directory must be new")?;
    let config_bytes =
        serde_json::to_vec_pretty(&config).map_err(|_| "cannot encode configuration")?;
    write_new(&output.join("config.json"), &config_bytes)?;
    write_new(&output.join("build.json"), &build_bytes)?;
    let start = Instant::now();
    let wall = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "wall clock before epoch")?
        .as_nanos() as u64;
    let anchor = ns(start);
    let recorder = Arc::new(Recorder::new(config.clone(), &output, start)?);
    let mut stages = BTreeMap::new();
    let mut stop: Option<Json> = None;
    let result = run(
        &config,
        &client,
        recorder.clone(),
        &output,
        args.get("--stop-file").cloned(),
        args.get("--phase-file").cloned(),
        &mut stages,
        &mut stop,
    )
    .await;
    let history = recorder.finish()?;
    let complete =
        result.is_ok() && history["accounting_complete"] == true && history["failure"].is_null();
    let report = json!({"version":1,"complete":complete,"failure":result.as_ref().err(),"configuration":config,"config_sha256":sha(&config_bytes),"build":build,"build_sha256":sha(&build_bytes),"history_sha256":file_sha(&output.join("history.jsonl"),MAX_HISTORY)?,"process_id":std::process::id(),"wall_anchor_unix_ns":wall,"wall_anchor_monotonic_ns":anchor,"elapsed_ns":ns(start),"runtime_threads":2,"workload_model":"closed_loop_correctness","independently_checked":false,"stages":stages,"stop":stop,"history":history});
    atomic_json(&output.join("report.json"), &report)?;
    if !complete {
        return Err("incomplete native batch workload; inspect retained report".into());
    }
    println!(
        "PASS: native batch workload drained; independent atomic history verification is required"
    );
    Ok(())
}
#[allow(clippy::too_many_arguments)]
async fn run(
    config: &Config,
    client: &PersistentRawClient,
    recorder: Arc<Recorder>,
    output: &Path,
    stop_path: Option<PathBuf>,
    phase_path: Option<PathBuf>,
    stages: &mut BTreeMap<&'static str, Json>,
    stop: &mut Option<Json>,
) -> Result<(), String> {
    let setup_start = ns(recorder.start);
    for first in (0..=config.keys).step_by(config.batch_size) {
        let keys = (first..(first + config.batch_size).min(config.keys + 1))
            .map(|i| config.key(i))
            .collect();
        let report = recorder
            .call(
                client,
                0,
                "initialization",
                None,
                RawOperation::BatchGet { keys },
            )
            .await?;
        if !matches!(report.outcome,Outcome::Success{value:Value::BatchGet{ref values}} if values.iter().all(Option::is_none))
        {
            return Err("batch workload requires a fresh empty keyspace".into());
        }
    }
    for first in (0..=config.keys).step_by(config.batch_size) {
        let pairs = (first..(first + config.batch_size).min(config.keys + 1))
            .map(|i| (config.key(i), config.value(0, i)))
            .collect();
        let report = recorder
            .call(
                client,
                0,
                "initialization",
                None,
                RawOperation::BatchPut { pairs },
            )
            .await?;
        if !matches!(report.outcome, Outcome::Success { .. }) {
            return Err("initial batch write is unconfirmed".into());
        }
    }
    stages.insert(
        "initialization",
        json!({"start_ns":setup_start,"end_ns":ns(recorder.start)}),
    );
    let measure_start = ns(recorder.start);
    let deadline = Instant::now() + Duration::from_millis(config.measure_ms);
    let issued = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let capacity = config.max_calls - (3 * (config.keys + 1).div_ceil(config.batch_size)) as u64;
    atomic_json(
        &output.join("ready.json"),
        &json!({"version":1,"process_id":std::process::id(),"monotonic_ns":measure_start}),
    )?;
    let cancel = tokio_util::sync::CancellationToken::new();
    let mut workers = JoinSet::new();
    for worker in 0..config.workers {
        let config = config.clone();
        let client = client.clone();
        let recorder = recorder.clone();
        let stop_path = stop_path.clone();
        let phase_path = phase_path.clone();
        let issued = issued.clone();
        let cancel = cancel.clone();
        workers.spawn(async move {
            loop {
                if cancel.is_cancelled()
                    || Instant::now() >= deadline
                    || stop_path.as_ref().is_some_and(|p| p.exists())
                {
                    break;
                }
                let nonce = issued.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if nonce >= capacity {
                    break;
                }
                let phase = phase(phase_path.as_deref())?;
                recorder
                    .call(
                        &client,
                        worker,
                        &phase,
                        Some(nonce),
                        config.operation(nonce),
                    )
                    .await?;
                if config.interval_ms > 0 {
                    tokio::time::sleep(Duration::from_millis(config.interval_ms)).await;
                }
            }
            Ok::<(), String>(())
        });
    }
    let mut failure = None;
    while !workers.is_empty() {
        tokio::select! {finished=workers.join_next()=>{match finished{Some(Ok(Ok(())))=>{},Some(Ok(Err(error)))=>{failure.get_or_insert(error);cancel.cancel();},Some(Err(_))=>{failure.get_or_insert("batch worker failed".into());cancel.cancel();},None=>{}}},_=tokio::time::sleep(Duration::from_millis(100))=>{if let Err(error)=atomic_json(&output.join("progress.json"),&recorder.progress()){failure.get_or_insert(error);cancel.cancel();}}}
    }
    let end = ns(recorder.start);
    *stop = Some(
        json!({"reason":if failure.is_some(){"worker_failure"}else if stop_path.as_ref().is_some_and(|p|p.exists()){"stop_file"}else if Instant::now()>=deadline{"duration"}else{"operation_limit"},"monotonic_ns":end}),
    );
    stages.insert(
        "measurement_and_drain",
        json!({"start_ns":measure_start,"end_ns":end}),
    );
    atomic_json(&output.join("progress.json"), &recorder.progress())?;
    if let Some(error) = failure {
        return Err(error);
    }
    recorder.ensure_traffic()?;
    let verify_start = ns(recorder.start);
    for first in (0..=config.keys).step_by(config.batch_size) {
        let keys = (first..(first + config.batch_size).min(config.keys + 1))
            .map(|i| config.key(i))
            .collect();
        let report = recorder
            .call(client, 0, "verify", None, RawOperation::BatchGet { keys })
            .await?;
        let Outcome::Success {
            value: Value::BatchGet { values },
        } = report.outcome
        else {
            return Err("final batch read is unconfirmed".into());
        };
        if first <= config.keys
            && config.keys < first + config.batch_size
            && values[config.keys - first] != Some(config.value(0, config.keys))
        {
            return Err("immutable sentinel changed".into());
        }
    }
    stages.insert(
        "verification",
        json!({"start_ns":verify_start,"end_ns":ns(recorder.start)}),
    );
    Ok(())
}
