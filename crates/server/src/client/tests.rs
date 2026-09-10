use super::*;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use tokio::sync::{oneshot, Notify};
use tokio_stream::StreamExt;
use tonic::Response;

enum Action {
    Pass,
    Refuse {
        delay: Duration,
        status: Status,
    },
    LoseReply {
        applied: Arc<Notify>,
        release: Arc<Notify>,
    },
    BadWrite,
    BadRead,
}

#[derive(Default)]
struct State {
    values: Mutex<BTreeMap<Vec<u8>, Vec<u8>>>,
    actions: Mutex<VecDeque<Action>>,
    writes: AtomicUsize,
    requests: AtomicUsize,
    connections: AtomicUsize,
    timeouts: Mutex<Vec<String>>,
}

impl State {
    fn inspect<T>(&self, request: &Request<T>) -> Option<Action> {
        assert_eq!(
            single(request.metadata(), "authorization"),
            Some("Bearer test-secret")
        );
        self.requests.fetch_add(1, Ordering::SeqCst);
        self.timeouts.lock().unwrap().push(
            single(request.metadata(), "grpc-timeout")
                .unwrap()
                .to_owned(),
        );
        self.actions.lock().unwrap().pop_front()
    }

    async fn write(
        &self,
        key: Vec<u8>,
        value: Option<Vec<u8>>,
        action: Option<Action>,
    ) -> Result<Response<proto::RawWriteResponse>, Status> {
        if let Some(Action::Refuse { delay, status }) = action {
            tokio::time::sleep(delay).await;
            return Err(status);
        }
        let index = self.writes.fetch_add(1, Ordering::SeqCst) as u64 + 1;
        {
            let mut values = self.values.lock().unwrap();
            if let Some(value) = value {
                values.insert(key, value);
            } else {
                values.remove(&key);
            }
        }
        if let Some(Action::LoseReply { applied, release }) = action {
            // The effect is visible before the response can complete. Other
            // connections continue reading/writing while the first is held.
            applied.notify_one();
            release.notified().await;
            return Err(Status::unavailable("applied response deliberately lost"));
        }
        Ok(Response::new(proto::RawWriteResponse {
            applied_term: if matches!(action, Some(Action::BadWrite)) {
                0
            } else {
                7
            },
            applied_index: index,
        }))
    }
}

#[derive(Clone)]
struct Service(Arc<State>);

macro_rules! service {
    ($( $name:ident : $request:ident => $response:ident ),* $(,)?) => {
        #[tonic::async_trait]
        impl proto::kv9_server::Kv9 for Service {
            async fn raw_get(&self, request: Request<proto::RawGetRequest>) -> Result<Response<proto::RawGetResponse>, Status> {
                let action = self.0.inspect(&request);
                if let Some(Action::Refuse { delay, status }) = action {
                    tokio::time::sleep(delay).await;
                    return Err(status);
                }
                if matches!(action, Some(Action::BadRead)) {
                    return Ok(Response::new(proto::RawGetResponse { value: None }));
                }
                let value = self.0.values.lock().unwrap().get(&request.into_inner().key).cloned();
                Ok(Response::new(proto::RawGetResponse { value: Some(proto::OptionalValue {
                    found: value.is_some(), value: value.unwrap_or_default(),
                }) }))
            }
            async fn raw_put(&self, request: Request<proto::RawPutRequest>) -> Result<Response<proto::RawWriteResponse>, Status> {
                let action = self.0.inspect(&request);
                let request = request.into_inner();
                self.0.write(request.key, Some(request.value), action).await
            }
            async fn raw_delete(&self, request: Request<proto::RawDeleteRequest>) -> Result<Response<proto::RawWriteResponse>, Status> {
                let action = self.0.inspect(&request);
                self.0.write(request.into_inner().key, None, action).await
            }
            $(async fn $name(&self, _: Request<proto::$request>) -> Result<Response<proto::$response>, Status> {
                Err(Status::unimplemented("unused fixture RPC"))
            })*
        }
    }
}

service! {
    raw_batch_get: RawBatchGetRequest => RawBatchGetResponse,
    raw_batch_put: RawBatchPutRequest => RawWriteResponse,
    raw_scan: RawScanRequest => ScanResponse,
    raw_delete_range: RawDeleteRangeRequest => RawDeleteRangeResponse,
    kv_begin: KvBeginRequest => KvBeginResponse,
    kv_get: KvGetRequest => KvGetResponse,
    kv_batch_get: KvBatchGetRequest => KvBatchGetResponse,
    kv_scan: KvScanRequest => ScanResponse,
    kv_prewrite: KvPrewriteRequest => Empty,
    kv_commit: KvCommitRequest => Empty,
    kv_pessimistic_lock: KvPessimisticLockRequest => Empty,
    kv_pessimistic_rollback: KvPessimisticRollbackRequest => Empty,
    kv_resolve_lock: KvResolveLockRequest => Empty,
    kv_cleanup: KvCleanupRequest => Empty,
    kv_check_txn_status: KvCheckTxnStatusRequest => KvCheckTxnStatusResponse,
    create_keyspace: CreateKeyspaceRequest => CreateKeyspaceResponse,
    list_keyspaces: ListKeyspacesRequest => ListKeyspacesResponse,
    get_region: GetRegionRequest => GetRegionResponse,
    split_region: SplitRegionRequest => Empty,
    cluster_info: ClusterInfoRequest => ClusterInfoResponse,
    admit_node: AdmitNodeRequest => MembershipChangeResponse,
    promote_node: PromoteNodeRequest => MembershipChangeResponse,
    get_node_endpoint: GetNodeEndpointRequest => GetNodeEndpointResponse,
    change_node_endpoint: ChangeNodeEndpointRequest => ChangeNodeEndpointResponse,
}

struct Server {
    state: Arc<State>,
    address: SocketAddr,
    shutdown: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
}

impl Server {
    async fn at(address: SocketAddr, state: Arc<State>) -> Self {
        let listener = tokio::net::TcpListener::bind(address).await.unwrap();
        let address = listener.local_addr().unwrap();
        let connections = state.clone();
        let incoming =
            tokio_stream::wrappers::TcpListenerStream::new(listener).map(move |result| {
                if result.is_ok() {
                    connections.connections.fetch_add(1, Ordering::SeqCst);
                }
                result
            });
        let (shutdown, stopped) = oneshot::channel();
        let task = tokio::spawn(
            tonic::transport::Server::builder()
                .add_service(proto::kv9_server::Kv9Server::new(Service(state.clone())))
                .serve_with_incoming_shutdown(incoming, async {
                    let _ = stopped.await;
                }),
        );
        Self {
            state,
            address,
            shutdown,
            task,
        }
    }

    async fn new() -> Self {
        Self::at("127.0.0.1:0".parse().unwrap(), Arc::new(State::default())).await
    }

    async fn stop(self) {
        self.shutdown.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), self.task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

    fn client(&self) -> PersistentRawClient {
        PersistentRawClient::new(config(&[self.address]), "test-secret").unwrap()
    }
}

fn config(addresses: &[SocketAddr]) -> ClientConfig {
    ClientConfig {
        version: 1,
        peers: addresses
            .iter()
            .enumerate()
            .map(|(i, address)| Peer {
                node_id: i as u64 + 1,
                address: *address,
            })
            .collect(),
        keyspace_id: 1,
        epoch_conf_ver: 1,
        epoch_version: 1,
        max_in_flight: 4,
        max_attempts: 6,
        deadline_ms: 1500,
        retry_backoff_ms: 1,
    }
}

fn put(value: &[u8]) -> RawOperation {
    RawOperation::Put {
        key: b"key".to_vec(),
        value: value.to_vec(),
    }
}

fn get() -> RawOperation {
    RawOperation::Get {
        key: b"key".to_vec(),
    }
}

fn not_leader(hint: Option<&str>) -> Status {
    let mut status = Status::failed_precondition("not leader");
    status
        .metadata_mut()
        .insert(NOT_LEADER_KEY, "true".parse().unwrap());
    if let Some(hint) = hint {
        status
            .metadata_mut()
            .insert(LEADER_HINT_KEY, hint.parse().unwrap());
    }
    status
}

fn refusal(status: Status) -> Action {
    Action::Refuse {
        delay: Duration::ZERO,
        status,
    }
}

#[tokio::test]
async fn warmup_failure_preserves_attempts_and_does_not_start_measurement() {
    use crate::workload::{run, Mix, Mode, RunOptions, WorkloadConfig};
    use sha2::{Digest, Sha256};
    use std::io::Read;

    // Both modes must retain diagnosis without turning failed warmup into an
    // accepted run. An unmarked write error must stay unknown and unreplayed.
    for mode in [Mode::Correctness, Mode::Performance] {
        for unknown in [false, true] {
            let server = Server::new().await;
            let output = std::env::temp_dir().join(format!(
                "kv9-warmup-failure-{}-{}",
                std::process::id(),
                server.address.port()
            ));
            std::fs::create_dir(&output).unwrap();
            let mut hasher = Sha256::new();
            let mut executable = std::fs::File::open(std::env::current_exe().unwrap()).unwrap();
            let mut buffer = [0u8; 65536];
            loop {
                let read = executable.read(&mut buffer).unwrap();
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
            }
            let manifest = output.join("build.json");
            std::fs::write(&manifest, serde_json::to_vec(&serde_json::json!({
                "version": 1, "revision": "0".repeat(40), "dirty": true,
                "source_tree_sha256": "0".repeat(64), "binary_sha256": format!("{:x}", hasher.finalize()),
                "profile": "debug", "rustc": "test fixture executable",
            })).unwrap()).unwrap();
            let configuration = WorkloadConfig {
                #[cfg(feature = "rpc-experiment")]
                rpc_transport: crate::rpc_experiment::TransportKind::TonicUnary,
                version: 1,
                client: config(&[server.address]),
                mode,
                run_id: "warmup-failure".into(),
                keyspace_name: "fresh-warmup".into(),
                seed: 40,
                workers: 1,
                keys: 1,
                value_bytes: 16,
                mix: Mix {
                    get: 0,
                    put: 100,
                    delete: 0,
                },
                warmup_operations: 2,
                max_operations: 32,
                measure_ms: 1000,
                interval_ms: 0,
                history_bytes: if mode == Mode::Correctness {
                    1024 * 1024
                } else {
                    0
                },
            };
            {
                let mut actions = server.state.actions.lock().unwrap();
                actions.extend([Action::Pass, Action::Pass, Action::Pass, Action::Pass]);
                for _ in 0..6 {
                    actions.push_back(refusal(if unknown {
                        Status::unavailable("private server error must not appear in artifacts")
                    } else {
                        not_leader(Some("1"))
                    }));
                }
            }
            let options = RunOptions {
                output: output.join("run"),
                build_manifest: manifest,
                stop_file: None,
                phase_file: None,
            };
            let report = run(configuration, "test-secret", options).await.unwrap();
            assert!(!report.complete);
            assert_eq!(
                report.failure,
                Some("warmup operation was not acknowledged")
            );
            assert_eq!(report.measured_issued, 0);
            assert_eq!(report.history.issued, 5);
            assert_eq!(report.history.terminal, 5);
            assert!(report.history.accounting_complete);
            assert!(!output.join("run/ready.json").exists());
            assert!(!output.join("run/progress.json").exists());
            let bytes = std::fs::read(output.join("run/warmup-failure.json")).unwrap();
            assert!(bytes.len() < 4096);
            let text = String::from_utf8(bytes).unwrap();
            assert!(!text.contains("test-secret") && !text.contains("private server error"));
            let diagnostic: serde_json::Value = serde_json::from_str(&text).unwrap();
            let call = &diagnostic["call"];
            assert_eq!(call["operation"], "put");
            assert_eq!(
                call["outcome"]["kind"],
                if unknown { "unknown_write" } else { "refused" }
            );
            assert_eq!(
                call["stop"],
                if unknown { "terminal" } else { "attempt_limit" }
            );
            let attempts = call["attempts"].as_array().unwrap();
            assert_eq!(attempts.len(), if unknown { 1 } else { 6 });
            for (i, attempt) in attempts.iter().enumerate() {
                assert_eq!(attempt["ordinal"], i + 1);
                assert_eq!(attempt["node_id"], 1);
                assert!(attempt["elapsed_ns"].as_u64().unwrap() > 0);
                if !unknown {
                    assert_eq!(attempt["failure"]["leader"], 1);
                }
            }
            assert_eq!(
                server.state.requests.load(Ordering::SeqCst),
                4 + attempts.len()
            );
            assert_eq!(server.state.writes.load(Ordering::SeqCst), 2);
            server.stop().await;
            std::fs::remove_dir_all(output).unwrap();
        }
    }
}

#[tokio::test]
async fn workload_stop_drains_an_applied_write_until_its_terminal_response() {
    use crate::workload::{run, Mix, Mode, RunOptions, WorkloadConfig};
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let server = Server::new().await;
    // A bound but unserved second endpoint forces the first final read to time
    // out after the unknown write rotates the preferred endpoint. Verification
    // must retain that failed read and use a new bounded read on the survivor.
    let unavailable = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let output = std::env::temp_dir().join(format!(
        "kv9-workload-drain-{}-{}",
        std::process::id(),
        server.address.port()
    ));
    std::fs::create_dir(&output).unwrap();
    let manifest = output.join("build.json");
    let mut executable = std::fs::File::open(std::env::current_exe().unwrap()).unwrap();
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let read = executable.read(&mut buffer).unwrap();
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    std::fs::write(&manifest, serde_json::to_vec(&serde_json::json!({
        "version": 1, "revision": "0".repeat(40), "dirty": true,
        "source_tree_sha256": "0".repeat(64), "binary_sha256": format!("{:x}", hasher.finalize()),
        "profile": "debug", "rustc": "test fixture executable",
    })).unwrap()).unwrap();
    let configuration = WorkloadConfig {
        #[cfg(feature = "rpc-experiment")]
        rpc_transport: crate::rpc_experiment::TransportKind::TonicUnary,
        version: 1,
        client: config(&[server.address, unavailable.local_addr().unwrap()]),
        mode: Mode::Correctness,
        run_id: "drain-test".into(),
        keyspace_name: "fresh-drain".into(),
        seed: 40,
        workers: 1,
        keys: 1,
        value_bytes: 16,
        mix: Mix {
            get: 0,
            put: 100,
            delete: 0,
        },
        warmup_operations: 0,
        max_operations: 32,
        measure_ms: 10000,
        interval_ms: 0,
        history_bytes: 1024 * 1024,
    };
    let applied = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    {
        let mut actions = server.state.actions.lock().unwrap();
        actions.extend([Action::Pass, Action::Pass, Action::Pass, Action::Pass]);
        actions.push_back(Action::LoseReply {
            applied: applied.clone(),
            release: release.clone(),
        });
    }
    let stop = output.join("stop");
    let options = RunOptions {
        output: output.join("run"),
        build_manifest: manifest,
        stop_file: Some(stop.clone()),
        phase_file: None,
    };
    let task = tokio::spawn(run(configuration, "test-secret", options));
    tokio::time::timeout(Duration::from_secs(30), applied.notified())
        .await
        .unwrap();
    std::fs::write(stop, b"stop").unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(
        !task.is_finished(),
        "stop abandoned the issued write before its terminal response"
    );
    release.notify_one();
    let report = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        report.complete,
        "drained workload lost its complete accounting"
    );
    assert_eq!(report.stop.unwrap().reason, "stop_file");
    assert_eq!(report.measured_issued, 1);
    assert_eq!(report.measured_completed, 1);
    assert_eq!(report.measured_successful, 0);
    assert_eq!(report.metrics.logical_counts[2][1][7], 1);
    assert_eq!(report.history.issued, 8);
    assert_eq!(report.history.terminal, 8);
    assert_eq!(
        report.metrics.logical_counts[3][0][7] + report.metrics.logical_counts[3][0][9],
        1
    );
    assert_eq!(report.metrics.logical_counts[3][0][0], 2);
    assert_eq!(report.history.peak_in_flight, 1);
    assert_eq!(
        server.state.writes.load(Ordering::SeqCst),
        3,
        "drain retried an unknown write"
    );
    let drain = report.stages.drain.unwrap();
    assert!(drain.end_ns - drain.start_ns >= 100_000_000);
    let events: Vec<serde_json::Value> = std::fs::read_to_string(output.join("run/history.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(events.len(), 17);
    assert_eq!(
        events.iter().filter(|e| e["outcome"] == "unknown").count(),
        2
    );
    server.stop().await;
    std::fs::remove_dir_all(output).unwrap();
}

#[tokio::test]
async fn persistent_connections_are_observed_by_the_server() {
    let server = Server::new().await;
    let client = server.client();
    for i in 0..32 {
        let write = client.call(put(&[i])).await;
        assert_eq!(
            write.outcome,
            Outcome::Success {
                value: Value::Applied {
                    term: 7,
                    index: u64::from(i) + 1
                }
            }
        );
        assert_eq!(write.attempts.len(), 1);
        assert_eq!(
            client.clone().call(get()).await.outcome,
            Outcome::Success {
                value: Value::Get {
                    value: Some(vec![i])
                }
            }
        );
    }
    let deleted = client
        .call(RawOperation::Delete {
            key: b"key".to_vec(),
        })
        .await;
    assert_eq!(
        deleted.outcome,
        Outcome::Success {
            value: Value::Applied { term: 7, index: 33 }
        }
    );
    assert_eq!(
        client.call(get()).await.outcome,
        Outcome::Success {
            value: Value::Get { value: None }
        }
    );
    assert_eq!(server.state.requests.load(Ordering::SeqCst), 66);
    assert_eq!(
        server.state.connections.load(Ordering::SeqCst),
        1,
        "per-RPC TCP connection creation is not persistent reuse"
    );
    server.stop().await;
}

#[tokio::test]
async fn persistent_channel_reconnects_after_server_restart() {
    let first = Server::new().await;
    let address = first.address;
    let client = first.client();
    assert!(matches!(
        client.call(get()).await.outcome,
        Outcome::Success { .. }
    ));
    assert_eq!(first.state.connections.load(Ordering::SeqCst), 1);
    first.stop().await;
    let restarted = Server::at(address, Arc::new(State::default())).await;
    let mut success = false;
    // Distinct reads, each with its own terminal report. Reconnection never
    // retroactively replays an uncertain operation from the old connection.
    for _ in 0..4 {
        let report = client.call(get()).await;
        assert_eq!(report.attempts.len(), 1);
        if matches!(report.outcome, Outcome::Success { .. }) {
            success = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(success, "bounded distinct calls did not reconnect");
    for _ in 0..16 {
        assert!(matches!(
            client.call(get()).await.outcome,
            Outcome::Success { .. }
        ));
    }
    assert_eq!(restarted.state.connections.load(Ordering::SeqCst), 1);
    restarted.stop().await;
}

#[tokio::test]
async fn exclusive_refusal_follows_only_a_configured_identity() {
    let follower = Server::new().await;
    let leader = Server::new().await;
    follower
        .state
        .actions
        .lock()
        .unwrap()
        .push_back(refusal(not_leader(Some("2"))));
    let client =
        PersistentRawClient::new(config(&[follower.address, leader.address]), "test-secret")
            .unwrap();
    let report = client.call(put(b"value")).await;
    assert!(matches!(report.outcome, Outcome::Success { .. }));
    assert_eq!(
        report
            .attempts
            .iter()
            .map(|a| a.node_id)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(follower.state.writes.load(Ordering::SeqCst), 0);
    assert_eq!(leader.state.writes.load(Ordering::SeqCst), 1);
    assert_eq!(client.next_peer(0, Some(999)), 1);
    assert_eq!(client.next_peer(1, Some(2)), 0);
    follower.stop().await;
    leader.stop().await;
}

#[tokio::test]
async fn unknown_write_is_not_replayed_after_an_intervening_read_and_write() {
    let server = Server::new().await;
    let applied = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    server
        .state
        .actions
        .lock()
        .unwrap()
        .push_back(Action::LoseReply {
            applied: applied.clone(),
            release: release.clone(),
        });
    let original = server.client();
    let pending = tokio::spawn(async move { original.call(put(b"v0")).await });
    tokio::time::timeout(Duration::from_secs(5), applied.notified())
        .await
        .unwrap();
    let other = server.client();
    assert_eq!(
        other.call(get()).await.outcome,
        Outcome::Success {
            value: Value::Get {
                value: Some(b"v0".to_vec())
            }
        }
    );
    assert!(matches!(
        other.call(put(b"v1")).await.outcome,
        Outcome::Success { .. }
    ));
    release.notify_one();
    let report = pending.await.unwrap();
    assert_eq!(report.attempts.len(), 1, "unknown v0 was dispatched again");
    assert_eq!(
        report.outcome,
        Outcome::UnknownWrite {
            reason: Reason::RpcStatus {
                code: Code::Unavailable as i32
            }
        }
    );
    assert_eq!(
        other.call(get()).await.outcome,
        Outcome::Success {
            value: Value::Get {
                value: Some(b"v1".to_vec())
            }
        }
    );
    assert_eq!(
        server.state.writes.load(Ordering::SeqCst),
        2,
        "hidden duplicate effect"
    );
    server.stop().await;
}

#[tokio::test]
async fn logical_deadline_is_not_reset_by_refusals() {
    let server = Server::new().await;
    for _ in 0..6 {
        server
            .state
            .actions
            .lock()
            .unwrap()
            .push_back(Action::Refuse {
                delay: Duration::from_millis(120),
                status: not_leader(None),
            });
    }
    let mut configuration = config(&[server.address]);
    configuration.deadline_ms = 300;
    let client = PersistentRawClient::new(configuration, "test-secret").unwrap();
    let started = Instant::now();
    let report = client.call(put(b"value")).await;
    assert!(
        started.elapsed() < Duration::from_millis(550),
        "logical deadline was restarted on retry"
    );
    assert!(
        (2..=3).contains(&report.attempts.len()),
        "unexpected attempt population: {report:?}"
    );
    assert!(
        matches!(report.outcome, Outcome::UnknownWrite { .. }),
        "last submitted attempt must remain uncertain"
    );
    let timeouts = server.state.timeouts.lock().unwrap().clone();
    fn decode(text: &str) -> u64 {
        let (number, unit) = text.split_at(text.len() - 1);
        number.parse::<u64>().unwrap()
            * match unit {
                "n" => 1,
                "u" => 1000,
                "m" => 1_000_000,
                "S" => 1_000_000_000,
                _ => panic!("unexpected timeout unit"),
            }
    }
    assert!(timeouts.len() >= 2);
    assert!(
        decode(&timeouts[1]) + 80_000_000 < decode(&timeouts[0]),
        "wire attempts received a fresh deadline: {timeouts:?}"
    );
    assert_eq!(server.state.writes.load(Ordering::SeqCst), 0);
    server.stop().await;
}

/// Invoked by the independent history gate with a new output directory. Both
/// the correct client and the unsafe retry mutant must finish this fixture;
/// the independent checker, rather than an expected-value assertion here,
/// decides whether the retained public history admits a legal execution.
#[tokio::test]
#[ignore = "requires an isolated output path and independent history verification"]
async fn persistent_response_loss_history_fixture() {
    use crate::workload::{Mix, Mode, Recorder, WorkloadConfig};
    let output = std::path::PathBuf::from(
        std::env::var("KV9_WORKLOAD_HISTORY_FIXTURE").expect("fixture output path required"),
    );
    assert!(output.is_dir());
    let server = Server::new().await;
    let configuration = WorkloadConfig {
        #[cfg(feature = "rpc-experiment")]
        rpc_transport: crate::rpc_experiment::TransportKind::TonicUnary,
        version: 1,
        client: config(&[server.address]),
        mode: Mode::Correctness,
        run_id: "response-loss-fixture".into(),
        keyspace_name: "fixture-fresh".into(),
        seed: 40,
        workers: 2,
        keys: 1,
        value_bytes: 16,
        mix: Mix {
            get: 50,
            put: 50,
            delete: 0,
        },
        warmup_operations: 0,
        max_operations: 100,
        measure_ms: 1000,
        interval_ms: 0,
        history_bytes: 1_048_576,
    };
    let recorder =
        Arc::new(Recorder::new(configuration, Some(&output.join("history.jsonl"))).unwrap());
    let applied = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    server
        .state
        .actions
        .lock()
        .unwrap()
        .push_back(Action::LoseReply {
            applied: applied.clone(),
            release: release.clone(),
        });
    let original = server.client();
    let first = recorder.begin(0, "measure", &put(b"v0")).unwrap();
    let retained = recorder.clone();
    let pending = tokio::spawn(async move {
        let report = original.call(put(b"v0")).await;
        retained.complete(first, &report).unwrap();
    });
    tokio::time::timeout(Duration::from_secs(5), applied.notified())
        .await
        .unwrap();
    let other = server.client();
    let read = recorder.begin(1, "measure", &get()).unwrap();
    let report = other.call(get()).await;
    recorder.complete(read, &report).unwrap();
    let write = recorder.begin(1, "measure", &put(b"v1")).unwrap();
    let report = other.call(put(b"v1")).await;
    recorder.complete(write, &report).unwrap();
    release.notify_one();
    pending.await.unwrap();
    let read = recorder.begin(1, "measure", &get()).unwrap();
    let report = other.call(get()).await;
    recorder.complete(read, &report).unwrap();
    let summary = recorder.finish().unwrap();
    assert!(summary.accounting_complete && summary.full_history_complete);
    std::fs::write(
        output.join("fixture.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "version": 1, "fixture_only": true, "history": summary,
            "observed_server_effects": server.state.writes.load(Ordering::SeqCst),
            "observed_tcp_connections": server.state.connections.load(Ordering::SeqCst),
        }))
        .unwrap(),
    )
    .unwrap();
    server.stop().await;
}

#[tokio::test]
async fn spaced_refusals_can_succeed_on_the_last_bounded_attempt() {
    let server = Server::new().await;
    for _ in 0..5 {
        server
            .state
            .actions
            .lock()
            .unwrap()
            .push_back(refusal(not_leader(None)));
    }
    let mut configuration = config(&[server.address]);
    configuration.deadline_ms = 1500;
    configuration.retry_backoff_ms = 100;
    let client = PersistentRawClient::new(configuration, "test-secret").unwrap();
    let report = client.call(put(b"value")).await;
    assert!(matches!(report.outcome, Outcome::Success { .. }));
    assert_eq!(report.attempts.len(), 6);
    assert!(
        report.elapsed_ns >= 500_000_000,
        "refusal retries omitted their configured spacing"
    );
    assert!(report.attempts[..5]
        .iter()
        .all(|a| matches!(a.failure, Some(Reason::NotLeader { .. }))));
    assert_eq!(server.state.requests.load(Ordering::SeqCst), 6);
    assert_eq!(server.state.writes.load(Ordering::SeqCst), 1);
    server.stop().await;
}

#[tokio::test]
async fn all_refused_attempts_remain_refused_when_hops_or_time_run_out() {
    let server = Server::new().await;
    for _ in 0..2 {
        server
            .state
            .actions
            .lock()
            .unwrap()
            .push_back(refusal(not_leader(None)));
    }
    let mut configuration = config(&[server.address]);
    configuration.max_attempts = 2;
    let client = PersistentRawClient::new(configuration, "test-secret").unwrap();
    let report = client.call(put(b"v0")).await;
    assert_eq!(report.stop, Stop::AttemptLimit);
    assert_eq!(report.attempts.len(), 2);
    assert_eq!(
        report.outcome,
        Outcome::Refused {
            reason: Reason::NotLeader { leader: None }
        }
    );

    server
        .state
        .actions
        .lock()
        .unwrap()
        .push_back(refusal(not_leader(None)));
    let mut configuration = config(&[server.address]);
    configuration.deadline_ms = 100;
    configuration.retry_backoff_ms = 100;
    let client = PersistentRawClient::new(configuration, "test-secret").unwrap();
    let report = client.call(put(b"v1")).await;
    assert_eq!(report.stop, Stop::Deadline);
    assert_eq!(
        report.attempts.len(),
        1,
        "backoff allowed a dispatch after the original deadline"
    );
    assert_eq!(
        report.outcome,
        Outcome::Refused {
            reason: Reason::NotLeader { leader: None }
        }
    );
    assert_eq!(server.state.writes.load(Ordering::SeqCst), 0);
    server.stop().await;
}

#[tokio::test]
async fn expired_write_remains_unknown_and_releases_only_client_capacity() {
    let server = Server::new().await;
    let applied = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    server
        .state
        .actions
        .lock()
        .unwrap()
        .push_back(Action::LoseReply {
            applied: applied.clone(),
            release,
        });
    let mut configuration = config(&[server.address]);
    configuration.max_in_flight = 1;
    configuration.deadline_ms = 100;
    let client = PersistentRawClient::new(configuration, "test-secret").unwrap();
    let cloned = client.clone();
    let pending = tokio::spawn(async move { cloned.call(put(b"v0")).await });
    tokio::time::timeout(Duration::from_secs(5), applied.notified())
        .await
        .unwrap();
    let refused = client.call(get()).await;
    assert_eq!(
        refused.outcome,
        Outcome::ClientRejected {
            reason: Reason::ClientCapacity
        }
    );
    assert!(refused.attempts.is_empty());
    let report = pending.await.unwrap();
    assert!(matches!(report.outcome, Outcome::UnknownWrite { .. }));
    assert_eq!(report.attempts.len(), 1);
    assert_eq!(
        client.call(get()).await.outcome,
        Outcome::Success {
            value: Value::Get {
                value: Some(b"v0".to_vec())
            }
        }
    );
    assert_eq!(server.state.writes.load(Ordering::SeqCst), 1);
    server.stop().await;
}

#[tokio::test]
async fn malformed_responses_and_admission_are_terminal() {
    let server = Server::new().await;
    let client = server.client();
    let mut mixed = not_leader(Some("1"));
    mixed
        .metadata_mut()
        .insert(ADMISSION_REFUSED_KEY, "request_count".parse().unwrap());
    let mut duplicate = not_leader(Some("1"));
    duplicate
        .metadata_mut()
        .append(NOT_LEADER_KEY, "true".parse().unwrap());
    for status in [
        mixed,
        duplicate,
        not_leader(Some("01")),
        not_leader(Some("0")),
    ] {
        server
            .state
            .actions
            .lock()
            .unwrap()
            .push_back(refusal(status));
        let report = client.call(put(b"value")).await;
        assert_eq!(report.attempts.len(), 1);
        assert_eq!(
            report.outcome,
            Outcome::UnknownWrite {
                reason: Reason::Protocol
            }
        );
    }
    server
        .state
        .actions
        .lock()
        .unwrap()
        .push_back(Action::BadWrite);
    assert_eq!(
        client.call(put(b"value")).await.outcome,
        Outcome::UnknownWrite {
            reason: Reason::Protocol
        }
    );
    server
        .state
        .actions
        .lock()
        .unwrap()
        .push_back(Action::BadRead);
    assert_eq!(
        client.call(get()).await.outcome,
        Outcome::ReadFailure {
            reason: Reason::Protocol
        }
    );
    let mut admission = Status::resource_exhausted("full");
    admission
        .metadata_mut()
        .insert(ADMISSION_REFUSED_KEY, "request_count".parse().unwrap());
    server
        .state
        .actions
        .lock()
        .unwrap()
        .push_back(refusal(admission));
    let report = client.call(put(b"value")).await;
    assert_eq!(
        report.outcome,
        Outcome::Refused {
            reason: Reason::AdmissionCount
        }
    );
    assert_eq!(report.attempts.len(), 1);
    assert_eq!(server.state.writes.load(Ordering::SeqCst), 1);
    server.stop().await;
}

#[tokio::test]
async fn dead_seed_does_not_prevent_later_operations_on_survivors() {
    let dead = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = dead.local_addr().unwrap();
    drop(dead);
    let live = Server::new().await;
    let client = PersistentRawClient::new(config(&[address, live.address]), "test-secret").unwrap();
    let first = client.call(put(b"v0")).await;
    assert!(matches!(first.outcome, Outcome::UnknownWrite { .. }));
    assert_eq!(first.attempts.len(), 1);
    let next = client.call(put(b"v1")).await;
    assert!(matches!(next.outcome, Outcome::Success { .. }));
    assert_eq!(next.attempts[0].node_id, 2);
    assert_eq!(live.state.writes.load(Ordering::SeqCst), 1);
    drop(client);
    assert!(matches!(
        live.client().call(get()).await.outcome,
        Outcome::Success { .. }
    ));
    live.stop().await;
}

#[tokio::test]
async fn limits_reject_before_any_rpc() {
    let server = Server::new().await;
    let mut configuration = config(&[server.address]);
    configuration.peers.push(configuration.peers[0].clone());
    assert!(PersistentRawClient::new(configuration, "test-secret").is_err());
    assert!(PersistentRawClient::new(config(&[server.address]), "secret\ninvalid").is_err());
    let report = server
        .client()
        .call(put(&vec![0; MAX_VALUE_BYTES + 1]))
        .await;
    assert_eq!(
        report.outcome,
        Outcome::ClientRejected {
            reason: Reason::ClientInput
        }
    );
    assert!(report.attempts.is_empty());
    assert_eq!(server.state.connections.load(Ordering::SeqCst), 0);
    assert_eq!(server.state.requests.load(Ordering::SeqCst), 0);
    server.stop().await;
}

#[test]
fn marker_decoder_fails_closed_and_keeps_read_failures_distinct() {
    let mut read = Status::unavailable("read not established");
    read.metadata_mut()
        .insert(READ_UNCONFIRMED_KEY, "quorum".parse().unwrap());
    assert_eq!(classify_status(&read, true), Reason::ReadQuorumUnconfirmed);
    assert_eq!(classify_status(&read, false), Reason::Protocol);
    read.metadata_mut()
        .insert(READ_UNCONFIRMED_KEY, "apply".parse().unwrap());
    assert_eq!(classify_status(&read, true), Reason::ReadApplyUnconfirmed);
    read.metadata_mut()
        .append(READ_UNCONFIRMED_KEY, "apply".parse().unwrap());
    assert_eq!(classify_status(&read, true), Reason::Protocol);
    let mut unknown = not_leader(None);
    unknown
        .metadata_mut()
        .insert("kv9-future-contract", "yes".parse().unwrap());
    assert_eq!(classify_status(&unknown, false), Reason::Protocol);
    let mut hint = not_leader(Some("2"));
    hint.metadata_mut()
        .append(LEADER_HINT_KEY, "2".parse().unwrap());
    assert_eq!(classify_status(&hint, false), Reason::Protocol);
    let unmarked = Status::failed_precondition("not leader, but prose is not a contract");
    assert_eq!(
        classify_status(&unmarked, false),
        Reason::RpcStatus {
            code: Code::FailedPrecondition as i32
        }
    );
}
