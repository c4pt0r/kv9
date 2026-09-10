//! Recorder/generator contracts only: these tests do not execute KV9 or Raft.

use super::*;
use kv9_server::client::{Attempt, Peer, Stop};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "kv9-batch-recorder-unit-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    // Close the history descriptor before removing this test's owned directory.
    recorder: Recorder,
    directory: Directory,
}

impl Fixture {
    fn new() -> Self {
        let directory = Directory::new();
        let config = config();
        config.validate().unwrap();
        let recorder = Recorder::new(config, &directory.0, Instant::now()).unwrap();
        Self {
            recorder,
            directory,
        }
    }

    fn events(&self) -> Vec<Json> {
        fs::read_to_string(self.directory.0.join("history.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

fn config() -> Config {
    Config {
        version: 1,
        client: ClientConfig {
            version: 1,
            peers: vec![Peer {
                node_id: 1,
                address: "127.0.0.1:19001".parse().unwrap(),
            }],
            keyspace_id: 1,
            epoch_conf_ver: 1,
            epoch_version: 1,
            max_in_flight: 2,
            max_attempts: 6,
            deadline_ms: 1500,
            retry_backoff_ms: 1,
        },
        rpc_transport: TransportKind::TonicStream,
        run_id: "recorders".into(),
        keyspace_name: "recorders".into(),
        workers: 2,
        seed: 40,
        keys: 8,
        batch_size: 3,
        value_bytes: 32,
        mix: [20; 5],
        max_calls: 512,
        history_bytes: 64 * 1024 * 1024,
        measure_ms: 1000,
        interval_ms: 0,
    }
}

fn duplicate_write() -> RawOperation {
    RawOperation::BatchPut {
        pairs: vec![
            (b"a".to_vec(), b"first".to_vec()),
            (b"b".to_vec(), Vec::new()),
            (b"a".to_vec(), b"last".to_vec()),
        ],
    }
}

fn report(op: &RawOperation, outcome: Outcome) -> CallReport {
    let failure = match &outcome {
        Outcome::Success { .. } => None,
        Outcome::Refused { reason }
        | Outcome::UnknownWrite { reason }
        | Outcome::ReadFailure { reason }
        | Outcome::ClientRejected { reason } => Some(reason.clone()),
    };
    CallReport {
        operation: op.kind(),
        elapsed_ns: 125_000,
        attempts: vec![Attempt {
            ordinal: 1,
            node_id: 1,
            elapsed_ns: 100_000,
            failure,
        }],
        stop: Stop::Terminal,
        outcome,
    }
}

fn successful_write(op: &RawOperation) -> CallReport {
    report(
        op,
        Outcome::Success {
            value: Value::Applied { term: 7, index: 19 },
        },
    )
}

fn assert_unrecorded(fixture: &Fixture, id: u64, original_bytes: u64) {
    let state = fixture.recorder.state.lock().unwrap();
    assert_eq!(state.issued, 1);
    assert_eq!(
        state.terminal, 0,
        "a return that was not written was counted"
    );
    assert_eq!(state.sequence, 1);
    assert_eq!(state.bytes, original_bytes);
    assert_eq!(state.active.len(), 1);
    assert!(state.active.contains_key(&id));
    assert_eq!(state.reserved, fixture.recorder.config.allowance());
    assert!(
        state.metrics.is_empty(),
        "unrecorded returns changed metrics"
    );
}

#[test]
fn invalid_whole_receipt_keeps_the_invocation_and_terminal_reservation() {
    for (term, index) in [(0, 19), (7, 0)] {
        let fixture = Fixture::new();
        let op = duplicate_write();
        let id = fixture.recorder.begin(0, "measure", Some(9), &op).unwrap();
        let bytes = fixture.recorder.state.lock().unwrap().bytes;
        let invalid = report(
            &op,
            Outcome::Success {
                value: Value::Applied { term, index },
            },
        );
        assert!(fixture.recorder.complete(id, &op, &invalid).is_err());
        assert_unrecorded(&fixture, id, bytes);
        let summary = fixture.recorder.finish().unwrap();
        assert_eq!(summary["accounting_complete"], false);
        assert!(!summary["failure"].is_null());
        let events = fixture.events();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1]["type"], "invoke");
        assert_eq!(events[1]["nonce"], 9);
    }
}

#[test]
fn unmatched_and_duplicate_terminals_cannot_create_history_or_metric_entries() {
    let fixture = Fixture::new();
    let op = duplicate_write();
    let id = fixture.recorder.begin(0, "measure", Some(0), &op).unwrap();
    let bytes = fixture.recorder.state.lock().unwrap().bytes;
    let valid = successful_write(&op);
    assert!(fixture.recorder.complete(id + 1, &op, &valid).is_err());
    assert_unrecorded(&fixture, id, bytes);
    fixture.recorder.complete(id, &op, &valid).unwrap();
    let before = fixture.recorder.finish().unwrap();
    let history_before = fixture.events();
    assert!(fixture.recorder.complete(id, &op, &valid).is_err());
    assert_eq!(fixture.recorder.finish().unwrap(), before);
    assert_eq!(fixture.events(), history_before);
    assert_eq!(before["issued"], 1);
    assert_eq!(before["terminal"], 1);
    assert_eq!(before["events"], 2);
    assert_eq!(before["accounting_complete"], true);
}

#[cfg(target_os = "linux")]
#[test]
fn failed_return_append_preserves_outstanding_accounting_and_zero_metrics() {
    let fixture = Fixture::new();
    let op = duplicate_write();
    let id = fixture.recorder.begin(0, "measure", Some(12), &op).unwrap();
    let original_bytes = fixture.recorder.state.lock().unwrap().bytes;
    let original_history = fixture.events();
    let original_file = {
        let mut state = fixture.recorder.state.lock().unwrap();
        std::mem::replace(
            &mut state.file,
            OpenOptions::new().write(true).open("/dev/full").unwrap(),
        )
    };
    let failure = fixture
        .recorder
        .complete(id, &op, &successful_write(&op))
        .unwrap_err();
    assert_eq!(failure, "cannot write complete history");
    assert_unrecorded(&fixture, id, original_bytes);
    // Restore only the output descriptor so finish can sync the retained good
    // prefix. The failed recording and its accounting are deliberately intact.
    fixture.recorder.state.lock().unwrap().file = original_file;
    let summary = fixture.recorder.finish().unwrap();
    assert_eq!(summary["accounting_complete"], false);
    assert_eq!(summary["failure"], failure);
    assert_eq!(summary["metrics"], json!([]));
    assert_eq!(fixture.events(), original_history);
    assert!(fixture.recorder.begin(1, "measure", Some(13), &op).is_err());
}

#[test]
fn duplicate_batch_items_have_one_logical_latency_sample_and_one_whole_receipt() {
    let fixture = Fixture::new();
    let op = duplicate_write();
    let id = fixture.recorder.begin(0, "measure", Some(4), &op).unwrap();
    let mut success = successful_write(&op);
    success.attempts = vec![
        Attempt {
            ordinal: 1,
            node_id: 1,
            elapsed_ns: 20_000,
            failure: Some(Reason::NotLeader { leader: Some(2) }),
        },
        Attempt {
            ordinal: 2,
            node_id: 2,
            elapsed_ns: 80_000,
            failure: None,
        },
    ];
    fixture.recorder.complete(id, &op, &success).unwrap();
    {
        let state = fixture.recorder.state.lock().unwrap();
        let metric = &state.metrics[&("measure".into(), "batch_put")];
        assert_eq!(
            (metric.calls, metric.input_items, metric.successful_items),
            (1, 3, 3)
        );
        assert_eq!(
            (
                metric.unknown_write_items,
                metric.refused_items,
                metric.attempts
            ),
            (0, 0, 2)
        );
        let logical = metric.logical.snapshot();
        assert!(logical.valid);
        assert_eq!(logical.outcomes.iter().map(|h| h.count).sum::<u64>(), 1);
        let success = &logical.outcomes[MetricOutcome::Success as usize];
        assert_eq!((success.count, success.sum_ns), (1, 125_000));
        assert_eq!(
            (success.min_ns, success.max_ns),
            (Some(125_000), Some(125_000))
        );
        let attempts = metric.attempt.snapshot();
        assert_eq!(attempts.outcomes.iter().map(|h| h.count).sum::<u64>(), 2);
        assert_eq!(
            attempts.outcomes[MetricOutcome::Rejected as usize].sum_ns,
            20_000
        );
        assert_eq!(
            attempts.outcomes[MetricOutcome::Success as usize].sum_ns,
            80_000
        );
        assert!(state.active.is_empty());
        assert_eq!(state.reserved, 0);
    }
    let events = fixture.events();
    assert_eq!(
        events.len(),
        3,
        "one batch must have exactly one invocation and return"
    );
    assert_eq!(
        events[1]["args"]["pairs"],
        json!([["61", "6669727374"], ["62", ""], ["61", "6c617374"]])
    );
    assert_eq!(events[2]["outcome"], "ok");
    assert_eq!(events[2]["result"], json!({}));
    assert_eq!(
        events[2]["observation"]["receipt"],
        json!({"applied_term": 7, "applied_index": 19})
    );
    assert_eq!(
        events[2]["observation"]["attempts"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn unknown_batch_retains_all_input_items_as_uncertain_without_a_receipt() {
    let fixture = Fixture::new();
    let op = duplicate_write();
    let id = fixture
        .recorder
        .begin(0, "quorum-loss", Some(5), &op)
        .unwrap();
    let unknown = report(
        &op,
        Outcome::UnknownWrite {
            reason: Reason::RpcStatus { code: 14 },
        },
    );
    fixture.recorder.complete(id, &op, &unknown).unwrap();
    {
        let state = fixture.recorder.state.lock().unwrap();
        let metric = &state.metrics[&("quorum-loss".into(), "batch_put")];
        assert_eq!(
            (metric.calls, metric.input_items, metric.unknown_write_items),
            (1, 3, 3)
        );
        assert_eq!(
            (
                metric.successful_items,
                metric.refused_items,
                metric.attempts
            ),
            (0, 0, 1)
        );
        assert_eq!(metric.reasons.get("rpc_status"), Some(&1));
        let logical = metric.logical.snapshot();
        assert_eq!(logical.outcomes.iter().map(|h| h.count).sum::<u64>(), 1);
        assert_eq!(
            logical.outcomes[MetricOutcome::Unconfirmed as usize].count,
            1
        );
        assert_eq!(
            metric.attempt.snapshot().outcomes[MetricOutcome::Unconfirmed as usize].count,
            1
        );
    }
    let events = fixture.events();
    assert_eq!(events.len(), 3);
    assert_eq!(events[2]["outcome"], "unknown");
    assert_eq!(events[2]["result"], json!({}));
    assert!(events[2]["observation"]["receipt"].is_null());
    assert_eq!(
        events[2]["observation"]["reason"],
        json!({"kind": "rpc_status", "code": 14})
    );
    assert_eq!(
        fixture.recorder.finish().unwrap()["accounting_complete"],
        true
    );
}

#[test]
fn batch_history_distinguishes_missing_empty_and_duplicate_read_positions() {
    let fixture = Fixture::new();
    let op = RawOperation::BatchGet {
        keys: vec![b"a".to_vec(), b"b".to_vec(), b"a".to_vec()],
    };
    let id = fixture.recorder.begin(0, "measure", Some(7), &op).unwrap();
    let result = report(
        &op,
        Outcome::Success {
            value: Value::BatchGet {
                values: vec![Some(Vec::new()), None, Some(Vec::new())],
            },
        },
    );
    fixture.recorder.complete(id, &op, &result).unwrap();
    let events = fixture.events();
    assert_eq!(events[1]["args"]["keys"], json!(["61", "62", "61"]));
    assert_eq!(events[2]["result"], json!({"values": ["", null, ""]}));
    assert!(events[2]["observation"]["receipt"].is_null());
    let summary = fixture.recorder.finish().unwrap();
    assert_eq!(summary["metrics"][0]["calls"], 1);
    assert_eq!(summary["metrics"][0]["input_items"], 3);
    assert_eq!(summary["metrics"][0]["successful_items"], 3);
    assert_eq!(
        summary["metrics"][0]["logical_latency"]["outcomes"][0]["count"],
        1
    );
}

#[test]
fn initialization_and_verification_alone_cannot_satisfy_the_traffic_guard() {
    let fixture = Fixture::new();
    assert!(fixture.recorder.ensure_traffic().is_err());
    let op = RawOperation::BatchGet {
        keys: vec![b"a".to_vec()],
    };
    let read = report(
        &op,
        Outcome::Success {
            value: Value::BatchGet { values: vec![None] },
        },
    );
    for phase in ["initialization", "verify"] {
        let id = fixture.recorder.begin(0, phase, None, &op).unwrap();
        fixture.recorder.complete(id, &op, &read).unwrap();
    }
    assert_eq!(
        fixture.recorder.finish().unwrap()["accounting_complete"],
        true
    );
    assert!(fixture.recorder.ensure_traffic().is_err());
    let id = fixture
        .recorder
        .begin(0, "client-link-delay", Some(0), &op)
        .unwrap();
    assert!(
        fixture.recorder.ensure_traffic().is_err(),
        "an invocation is not recorded traffic completion"
    );
    fixture.recorder.complete(id, &op, &read).unwrap();
    fixture.recorder.ensure_traffic().unwrap();
}

#[test]
fn former_hundred_key_mix_counterexample_now_has_shared_read_and_write_keys() {
    let mut config = config();
    config.keys = 100;
    config.batch_size = 1;
    config.validate().unwrap();
    let mut visited: BTreeMap<&str, BTreeSet<Vec<u8>>> = BTreeMap::new();
    let mut first_cycle = BTreeMap::new();
    let mut transitions = 0;
    let mut previous = None;
    for nonce in 0..2000 {
        let op = config.operation(nonce);
        let op_kind = kind(&op);
        let key = match &op {
            RawOperation::Get { key }
            | RawOperation::Put { key, .. }
            | RawOperation::Delete { key } => key,
            RawOperation::BatchGet { keys } => &keys[0],
            RawOperation::BatchPut { pairs } => &pairs[0].0,
        };
        visited.entry(op_kind).or_default().insert(key.clone());
        assert_ne!(*key, config.key(config.keys));
        if nonce < 100 {
            *first_cycle.entry(op_kind).or_insert(0) += 1;
            if previous.is_some_and(|old| old != op_kind) {
                transitions += 1;
            }
            previous = Some(op_kind);
        }
    }
    for op_kind in KINDS {
        assert_eq!(first_cycle[op_kind], 20);
    }
    assert!(
        transitions > 20,
        "the mix regressed to long homogeneous kind blocks"
    );
    assert!(
        !visited["get"].is_disjoint(&visited["put"]),
        "point reads and writes remain permanently disjoint"
    );
    assert!(
        !visited["batch_get"].is_disjoint(&visited["batch_put"]),
        "batch reads and writes remain permanently disjoint"
    );
    assert!(!visited["batch_get"].is_disjoint(&visited["put"]));
    assert!(!visited["get"].is_disjoint(&visited["batch_put"]));
}

#[test]
fn generated_values_retain_unique_call_and_item_identity_and_exclude_the_empty_sentinel() {
    let mut config = config();
    config.keys = 2;
    config.batch_size = 5;
    config.validate().unwrap();
    assert!(config.value(0, config.keys).is_empty());
    assert!(!config.value(0, 0).is_empty());
    let allowed: BTreeSet<_> = (0..config.keys).map(|i| config.key(i)).collect();
    let mut writes = 0;
    for nonce in 0..100 {
        let op = config.operation(nonce);
        assert_eq!(
            arguments(&op, 1),
            arguments(&config.clone().operation(nonce), 1)
        );
        let pairs = match op {
            RawOperation::Put { key, value } => vec![(key, value)],
            RawOperation::BatchPut { pairs } => pairs,
            RawOperation::Get { key } | RawOperation::Delete { key } => {
                assert!(allowed.contains(&key));
                continue;
            }
            RawOperation::BatchGet { keys } => {
                assert!(keys.iter().all(|key| allowed.contains(key)));
                continue;
            }
        };
        writes += 1;
        for (item, (key, value)) in pairs.iter().enumerate() {
            assert!(allowed.contains(key));
            assert_eq!(value.len(), config.value_bytes);
            assert_eq!(&value[..8], &(nonce + 1).to_be_bytes());
            assert_eq!(&value[8..16], &(item as u64).to_be_bytes());
        }
        if pairs.len() > config.keys {
            assert_eq!(pairs[0].0, pairs[config.keys].0);
            assert_ne!(
                pairs[0].1, pairs[config.keys].1,
                "duplicate keys lost ordered item identities"
            );
        }
    }
    assert_eq!(writes, 40);
}
