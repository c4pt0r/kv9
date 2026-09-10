use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::Serialize;
use serde_json::json;

use crate::client::{CallReport, OperationKind, Outcome, RawOperation, Reason, Value};

use super::config::{Mode, WorkloadConfig, HEADER_ALLOWANCE};

/// Fixed vocabulary; neither a caller's labels nor fault names can grow state.
pub const PHASES: [&str; 27] = [
    "initialization",
    "warmup",
    "measure",
    "verify",
    "baseline",
    "healing",
    "registration-seed-blackhole",
    "pod-failure-1",
    "pod-failure-2",
    "pod-failure-3",
    "partition",
    "public-admission-overload",
    "delay",
    "io-voter-1-errno-5",
    "io-voter-1-errno-28",
    "io-voter-2-errno-5",
    "io-voter-2-errno-28",
    "io-voter-3-errno-5",
    "io-voter-3-errno-28",
    "store-loss-voter-1-log-missing",
    "store-loss-voter-2-log-missing",
    "store-loss-voter-3-log-missing",
    "store-loss-voter-1-pvc-replacement",
    "store-loss-voter-2-pvc-replacement",
    "store-loss-voter-3-pvc-replacement",
    "endpoint-migration-pending",
    "endpoint-migration-recovered",
];

#[derive(Clone, Copy)]
struct Invocation {
    kind: OperationKind,
    phase: usize,
    terminal_space: u64,
}

/// Consumed by complete. Forgetting/dropping a ticket prevents a complete report.
/// Private fields prevent a caller from inventing a ticket for another recorder.
pub struct Ticket {
    id: u64,
    recorder: Arc<()>,
}

struct State {
    file: Option<File>,
    issued: u64,
    terminal: u64,
    sequence: u64,
    bytes: u64,
    reserved: u64,
    active: BTreeMap<u64, Invocation>,
    failed: Option<&'static str>,
    finished: bool,
    successes: [[u64; 3]; PHASES.len()],
    recorder_ns: u64,
    peak_in_flight: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct HistorySummary {
    pub version: u32,
    pub mode: Mode,
    pub issued: u64,
    pub terminal: u64,
    pub events: u64,
    pub bytes: u64,
    pub accounting_complete: bool,
    pub full_history_complete: bool,
    pub failure: Option<&'static str>,
    pub independently_checked: bool,
    pub recorder_ns: u64,
    pub peak_in_flight: usize,
    pub phase_successes: Vec<PhaseSuccess>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PhaseSuccess {
    pub phase: &'static str,
    pub get: u64,
    pub put: u64,
    pub delete: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Progress {
    pub issued: u64,
    pub terminal: u64,
    pub in_flight: usize,
    pub peak_in_flight: usize,
    pub phase_successes: Vec<PhaseSuccess>,
}

pub struct Recorder {
    config: WorkloadConfig,
    start: Instant,
    identity: Arc<()>,
    state: Mutex<State>,
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(DIGITS[(byte >> 4) as usize] as char);
        text.push(DIGITS[(byte & 15) as usize] as char);
    }
    text
}

fn ns(start: Instant) -> u64 {
    start.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
}

fn line(value: &serde_json::Value) -> Result<Vec<u8>, &'static str> {
    let mut bytes = serde_json::to_vec(value).map_err(|_| "history encoding failed")?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn invalidate<T>(state: &mut State, reason: &'static str) -> Result<T, &'static str> {
    state.failed.get_or_insert(reason);
    Err(reason)
}

impl Recorder {
    /// path must be absent in performance-only mode; correctness creates a new
    /// file exclusively. Opening an existing history can never truncate it.
    pub fn new(config: WorkloadConfig, path: Option<&Path>) -> Result<Self, &'static str> {
        config.validate()?;
        if (config.mode == Mode::Correctness) != path.is_some() {
            return Err("history path must match the configured mode");
        }
        let mut file = path
            .map(|path| OpenOptions::new().write(true).create_new(true).open(path))
            .transpose()
            .map_err(|_| "cannot create new history file")?;
        let mut bytes = 0;
        if let Some(file) = &mut file {
            let header = line(&json!({
                "type": "header", "version": 1, "range_chunk_size": 1024,
                "generator": "kv9-persistent-workload", "configuration": config,
                "initial": {"keyspaces": [{"name": config.keyspace_name, "id": config.client.keyspace_id}], "kv": []},
            }))?;
            if header.len() as u64 > HEADER_ALLOWANCE {
                return Err("history header exceeds its reserved space");
            }
            file.write_all(&header)
                .map_err(|_| "cannot write history header")?;
            bytes = header.len() as u64;
        }
        Ok(Self {
            config,
            start: Instant::now(),
            identity: Arc::new(()),
            state: Mutex::new(State {
                file,
                issued: 0,
                terminal: 0,
                sequence: 0,
                bytes,
                reserved: 0,
                active: BTreeMap::new(),
                failed: None,
                finished: false,
                successes: [[0; 3]; PHASES.len()],
                recorder_ns: 0,
                peak_in_flight: 0,
            }),
        })
    }

    pub fn begin(
        &self,
        worker: usize,
        phase: &str,
        operation: &RawOperation,
    ) -> Result<Ticket, &'static str> {
        let started = Instant::now();
        let mut state = self.state.lock().map_err(|_| "history lock poisoned")?;
        if let Some(reason) = state.failed {
            return Err(reason);
        }
        if state.finished {
            return Err("history is already finished");
        }
        let Some(phase_index) = PHASES.iter().position(|known| *known == phase) else {
            return invalidate(&mut state, "unknown history phase");
        };
        if worker >= self.config.workers || state.active.len() >= self.config.workers {
            return invalidate(&mut state, "history worker capacity exceeded");
        }
        if state.issued >= self.config.max_operations {
            return Err("history operation limit reached");
        }
        let (key, value) = match operation {
            RawOperation::Get { key } | RawOperation::Delete { key } => (key, None),
            RawOperation::Put { key, value } => (key, Some(value)),
            RawOperation::BatchGet { .. } | RawOperation::BatchPut { .. } => {
                return invalidate(&mut state, "batch calls require atomic batch history")
            }
        };
        if key.len() as u64 > self.config.key_bytes()
            || value.is_some_and(|v| v.len() > self.config.value_bytes)
        {
            return invalidate(&mut state, "operation exceeds the workload payload bound");
        }
        let id = state.issued;
        let mut terminal_space = 0;
        if self.config.mode == Mode::Correctness {
            let allowance = self.config.event_pair_allowance();
            if state.bytes + state.reserved + allowance > self.config.history_bytes {
                return invalidate(
                    &mut state,
                    "complete history space exhausted before invocation",
                );
            }
            let mut args = json!({"keyspace": self.config.client.keyspace_id, "key": hex(key)});
            if let Some(value) = value {
                args["value"] = hex(value).into();
            }
            let event = line(&json!({
                "type": "invoke", "seq": state.sequence, "monotonic_ns": ns(self.start),
                "id": id, "client": worker.to_string(), "phase": phase,
                "op": operation.kind(), "args": args,
            }))?;
            // Retain this invocation's exact remaining reservation until its
            // terminal is written. Other completions cannot spend this space.
            if event.len() as u64 > allowance {
                return invalidate(&mut state, "invocation exceeds its reserved space");
            }
            if state.file.as_mut().unwrap().write_all(&event).is_err() {
                return invalidate(&mut state, "history invocation write failed");
            }
            state.bytes += event.len() as u64;
            terminal_space = allowance - event.len() as u64;
            state.reserved += terminal_space;
        }
        state.issued += 1;
        state.sequence += 1;
        state.active.insert(
            id,
            Invocation {
                kind: operation.kind(),
                phase: phase_index,
                terminal_space,
            },
        );
        state.peak_in_flight = state.peak_in_flight.max(state.active.len());
        state.recorder_ns = state.recorder_ns.saturating_add(ns(started));
        Ok(Ticket {
            id,
            recorder: self.identity.clone(),
        })
    }

    pub fn complete(&self, ticket: Ticket, report: &CallReport) -> Result<(), &'static str> {
        let started = Instant::now();
        let mut state = self.state.lock().map_err(|_| "history lock poisoned")?;
        if state.finished {
            return invalidate(&mut state, "terminal arrived after recorder finalization");
        }
        if !Arc::ptr_eq(&ticket.recorder, &self.identity) {
            return invalidate(&mut state, "ticket belongs to another recorder");
        }
        let Some(invocation) = state.active.remove(&ticket.id) else {
            return invalidate(&mut state, "missing or duplicated terminal record");
        };
        state.reserved -= invocation.terminal_space;
        // Accounting survives an output failure; the failure flag still prevents
        // the report from claiming that the file contains a complete history.
        state.terminal += 1;
        if report.operation != invocation.kind
            || report.attempts.len() > self.config.client.max_attempts
        {
            return invalidate(&mut state, "terminal report does not match its invocation");
        }
        let outcome = match &report.outcome {
            Outcome::Success {
                value: Value::Get { value },
            } if invocation.kind == OperationKind::Get => {
                if value
                    .as_ref()
                    .is_some_and(|value| value.len() > self.config.value_bytes)
                {
                    return invalidate(
                        &mut state,
                        "observed value exceeds the declared workload bound",
                    );
                }
                "ok"
            }
            Outcome::Success {
                value: Value::Applied { term, index },
            } if invocation.kind != OperationKind::Get && *term > 0 && *index > 0 => "ok",
            Outcome::Refused {
                reason:
                    Reason::NotLeader { .. }
                    | Reason::AdmissionCount
                    | Reason::AdmissionBytes
                    | Reason::AdmissionOversize,
            } => "refused",
            Outcome::ClientRejected { .. } if report.attempts.is_empty() => "refused",
            Outcome::UnknownWrite { .. } if invocation.kind != OperationKind::Get => "unknown",
            Outcome::ReadFailure { .. } if invocation.kind == OperationKind::Get => "unknown",
            _ => return invalidate(&mut state, "invalid terminal outcome contract"),
        };
        if outcome == "ok" {
            state.successes[invocation.phase][invocation.kind as usize] += 1;
        }
        if self.config.mode == Mode::Correctness {
            let result = match &report.outcome {
                Outcome::Success {
                    value: Value::Get { value },
                } => json!({"value": value.as_ref().map(|value| hex(value))}),
                _ if outcome == "refused" => json!({"proof": "precommit"}),
                _ => json!({}),
            };
            let reason = match &report.outcome {
                Outcome::Success { .. } => None,
                Outcome::Refused { reason }
                | Outcome::UnknownWrite { reason }
                | Outcome::ReadFailure { reason }
                | Outcome::ClientRejected { reason } => Some(reason),
            };
            let receipt = match &report.outcome {
                Outcome::Success {
                    value: Value::Applied { term, index },
                } => Some(json!({"applied_term": term, "applied_index": index})),
                _ => None,
            };
            let event = line(&json!({
                "type": "return", "seq": state.sequence, "monotonic_ns": ns(self.start),
                "id": ticket.id, "outcome": outcome, "result": result,
                "observation": {"attempts": report.attempts, "elapsed_ns": report.elapsed_ns,
                    "stop": report.stop, "reason": reason, "receipt": receipt,
                    "malformed": if reason == Some(&Reason::Protocol) { Some("protocol_response") } else { None }},
            }))?;
            if event.len() as u64 > invocation.terminal_space
                || state.bytes + state.reserved + event.len() as u64 > self.config.history_bytes
            {
                return invalidate(&mut state, "terminal exceeds complete history space");
            }
            if state.file.as_mut().unwrap().write_all(&event).is_err() {
                return invalidate(&mut state, "history terminal write failed");
            }
            state.bytes += event.len() as u64;
        }
        state.sequence += 1;
        state.recorder_ns = state.recorder_ns.saturating_add(ns(started));
        Ok(())
    }

    pub fn finish(&self) -> Result<HistorySummary, &'static str> {
        let mut state = self.state.lock().map_err(|_| "history lock poisoned")?;
        if state.finished {
            return Err("history is already finished");
        }
        state.finished = true;
        if state.issued == 0
            || !state.active.is_empty()
            || state.issued != state.terminal
            || state.sequence != state.issued * 2
        {
            state
                .failed
                .get_or_insert("issued operations lack terminal records");
        }
        if let Some(file) = &mut state.file {
            if file.flush().and_then(|_| file.sync_all()).is_err() {
                state.failed.get_or_insert("history final flush failed");
            }
        }
        Ok(HistorySummary {
            version: 1,
            mode: self.config.mode,
            issued: state.issued,
            terminal: state.terminal,
            events: state.sequence,
            bytes: state.bytes,
            accounting_complete: state.failed.is_none(),
            full_history_complete: self.config.mode == Mode::Correctness && state.failed.is_none(),
            failure: state.failed,
            independently_checked: false,
            recorder_ns: state.recorder_ns,
            peak_in_flight: state.peak_in_flight,
            phase_successes: PHASES
                .iter()
                .enumerate()
                .map(|(i, phase)| PhaseSuccess {
                    phase,
                    get: state.successes[i][0],
                    put: state.successes[i][1],
                    delete: state.successes[i][2],
                })
                .collect(),
        })
    }

    pub(super) fn epoch(&self) -> Instant {
        self.start
    }

    pub fn progress(&self) -> Result<Progress, &'static str> {
        let state = self.state.lock().map_err(|_| "history lock poisoned")?;
        Ok(Progress {
            issued: state.issued,
            terminal: state.terminal,
            in_flight: state.active.len(),
            peak_in_flight: state.peak_in_flight,
            phase_successes: PHASES
                .iter()
                .enumerate()
                .map(|(i, phase)| PhaseSuccess {
                    phase,
                    get: state.successes[i][0],
                    put: state.successes[i][1],
                    delete: state.successes[i][2],
                })
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{Attempt, Stop};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            for _ in 0..100 {
                let path = std::env::temp_dir().join(format!(
                    "kv9-recorder-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                match std::fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("create fixture directory: {error}"),
                }
            }
            panic!("fixture directory collision budget exhausted");
        }

        fn path(&self) -> PathBuf {
            self.0.join("history.jsonl")
        }

        fn records(&self) -> Vec<serde_json::Value> {
            std::fs::read_to_string(self.path())
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect()
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn put(value: Vec<u8>) -> RawOperation {
        RawOperation::Put {
            key: b"key".to_vec(),
            value,
        }
    }
    fn get() -> RawOperation {
        RawOperation::Get {
            key: b"key".to_vec(),
        }
    }

    fn report(operation: OperationKind, outcome: Outcome) -> CallReport {
        CallReport {
            operation,
            elapsed_ns: 20,
            attempts: vec![Attempt {
                ordinal: 1,
                node_id: 1,
                elapsed_ns: 10,
                failure: match &outcome {
                    Outcome::Success { .. } => None,
                    Outcome::UnknownWrite { reason } => Some(reason.clone()),
                    _ => panic!("unsupported fixture outcome"),
                },
            }],
            stop: Stop::Terminal,
            outcome,
        }
    }

    fn applied() -> CallReport {
        report(
            OperationKind::Put,
            Outcome::Success {
                value: Value::Applied { term: 7, index: 11 },
            },
        )
    }

    #[test]
    fn complete_history_preserves_unknowns_and_out_of_order_terminals() {
        let temp = Temp::new();
        let recorder = Recorder::new(super::super::config::example(), Some(&temp.path())).unwrap();
        let first = recorder.begin(0, "measure", &put(b"v0".to_vec())).unwrap();
        let second = recorder.begin(1, "measure", &get()).unwrap();
        recorder
            .complete(
                second,
                &report(
                    OperationKind::Get,
                    Outcome::Success {
                        value: Value::Get {
                            value: Some(b"v0".to_vec()),
                        },
                    },
                ),
            )
            .unwrap();
        recorder
            .complete(
                first,
                &report(
                    OperationKind::Put,
                    Outcome::UnknownWrite {
                        reason: Reason::Deadline,
                    },
                ),
            )
            .unwrap();
        let summary = recorder.finish().unwrap();
        assert!(
            summary.accounting_complete && summary.full_history_complete,
            "complete workload lost a terminal record"
        );
        assert_eq!(
            (summary.issued, summary.terminal, summary.events),
            (2, 2, 4)
        );
        assert!(!summary.independently_checked);
        assert_eq!(
            (
                summary.phase_successes[2].get,
                summary.phase_successes[2].put
            ),
            (1, 0)
        );
        let records = temp.records();
        assert_eq!(
            records.len(),
            5,
            "complete history lost serialized terminal records"
        );
        assert_eq!(records[3]["id"], 1);
        assert_eq!(records[4]["id"], 0);
        assert_eq!(records[4]["outcome"], "unknown");
        assert_eq!(records[4]["result"], json!({}));
        assert_eq!(records[3]["result"]["value"], "7630");
        for (i, record) in records[1..].iter().enumerate() {
            assert_eq!(record["seq"], i);
        }
        assert_eq!(summary.bytes, std::fs::metadata(temp.path()).unwrap().len());
    }

    #[test]
    fn pending_or_empty_history_cannot_be_finalized_as_complete() {
        let temp = Temp::new();
        let recorder = Recorder::new(super::super::config::example(), Some(&temp.path())).unwrap();
        let ticket = recorder.begin(0, "measure", &put(b"v0".to_vec())).unwrap();
        drop(ticket);
        let summary = recorder.finish().unwrap();
        assert!(!summary.accounting_complete && !summary.full_history_complete);
        assert_eq!(summary.issued, 1);
        assert_eq!(summary.terminal, 0);
        assert_eq!(
            summary.failure,
            Some("issued operations lack terminal records")
        );
        let temp = Temp::new();
        let empty = Recorder::new(super::super::config::example(), Some(&temp.path())).unwrap();
        assert!(!empty.finish().unwrap().full_history_complete);
    }

    #[test]
    fn byte_guard_refuses_new_invocation_and_preserves_pending_terminal_space() {
        let temp = Temp::new();
        let mut recorder =
            Recorder::new(super::super::config::example(), Some(&temp.path())).unwrap();
        // Force the runtime boundary despite the normal preflight reservation.
        // This control exercises the guard independently of preflight arithmetic.
        recorder.config.history_bytes =
            recorder.state.lock().unwrap().bytes + recorder.config.event_pair_allowance();
        let first = recorder.begin(0, "measure", &put(b"v0".to_vec())).unwrap();
        assert!(matches!(
            recorder.begin(1, "measure", &get()),
            Err("complete history space exhausted before invocation")
        ));
        recorder.complete(first, &applied()).unwrap();
        let summary = recorder.finish().unwrap();
        assert_eq!((summary.issued, summary.terminal), (1, 1));
        assert!(!summary.full_history_complete);
        assert!(summary.bytes <= recorder.config.history_bytes);
        assert_eq!(
            temp.records().len(),
            3,
            "failed admission invented an invocation"
        );
    }

    #[test]
    fn maximum_configured_values_fit_pair_reservations() {
        let temp = Temp::new();
        let mut config = super::super::config::example();
        config.value_bytes = 8192;
        let recorder = Recorder::new(config, Some(&temp.path())).unwrap();
        let write = recorder.begin(0, "measure", &put(vec![255; 8192])).unwrap();
        let read = recorder.begin(1, "measure", &get()).unwrap();
        recorder
            .complete(
                read,
                &report(
                    OperationKind::Get,
                    Outcome::Success {
                        value: Value::Get {
                            value: Some(vec![255; 8192]),
                        },
                    },
                ),
            )
            .unwrap();
        recorder.complete(write, &applied()).unwrap();
        assert!(recorder.finish().unwrap().full_history_complete);
        assert_eq!(temp.records().len(), 5);
    }

    #[test]
    fn performance_mode_reports_accounting_without_claiming_full_history() {
        let mut config = super::super::config::example();
        config.mode = Mode::Performance;
        config.history_bytes = 0;
        let recorder = Recorder::new(config, None).unwrap();
        let ticket = recorder.begin(0, "measure", &put(b"v0".to_vec())).unwrap();
        // Moving the recorder does not change ticket identity.
        let moved = Box::new(recorder);
        moved.complete(ticket, &applied()).unwrap();
        let summary = moved.finish().unwrap();
        assert!(summary.accounting_complete);
        assert!(!summary.full_history_complete && !summary.independently_checked);
        assert_eq!(summary.bytes, 0);
        assert_eq!(summary.phase_successes[2].put, 1);
    }
}
