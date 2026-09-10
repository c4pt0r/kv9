//! SDK contract tests use a controlled unary peer. They establish batch framing,
//! validation and retry behavior; the fixture is not an engine atomicity proof.

use super::*;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use tokio::sync::{oneshot, Notify};
use tonic::Response;

enum Action {
    Refuse {
        delay: Duration,
        status: Status,
    },
    Hold {
        entered: Arc<Notify>,
        release: Arc<Notify>,
    },
    ReadReply {
        values: Vec<proto::OptionalValue>,
        control: bool,
    },
    WriteReply {
        term: u64,
        index: u64,
    },
}

#[derive(Default)]
struct State {
    values: Mutex<BTreeMap<Vec<u8>, Vec<u8>>>,
    actions: Mutex<VecDeque<Action>>,
    requests: AtomicUsize,
    writes: AtomicUsize,
    timeouts: Mutex<Vec<Duration>>,
    read_keys: Mutex<Vec<Vec<Vec<u8>>>>,
    write_pairs: Mutex<Vec<Vec<proto::KeyValue>>>,
}

fn timeout_duration(value: &str) -> Duration {
    let (number, unit) = value.split_at(value.len() - 1);
    let number: u64 = number.parse().unwrap();
    match unit {
        "H" => Duration::from_secs(number * 3600),
        "M" => Duration::from_secs(number * 60),
        "S" => Duration::from_secs(number),
        "m" => Duration::from_millis(number),
        "u" => Duration::from_micros(number),
        "n" => Duration::from_nanos(number),
        _ => panic!("invalid grpc-timeout unit"),
    }
}

impl State {
    fn inspect<T>(&self, request: &Request<T>) -> Option<Action> {
        assert_eq!(
            single(request.metadata(), "authorization"),
            Some("Bearer batch-secret")
        );
        self.requests.fetch_add(1, Ordering::SeqCst);
        self.timeouts.lock().unwrap().push(timeout_duration(
            single(request.metadata(), "grpc-timeout").unwrap(),
        ));
        self.actions.lock().unwrap().pop_front()
    }

    fn push(&self, action: Action) {
        self.actions.lock().unwrap().push_back(action);
    }
}

fn assert_context(context: Option<proto::RequestContext>) {
    let context = context.unwrap();
    assert_eq!(context.keyspace_id, 19);
    let epoch = context.region_epoch.unwrap();
    assert_eq!((epoch.conf_ver, epoch.version), (3, 5));
}

#[derive(Clone)]
struct Service(Arc<State>);

macro_rules! batch_service {
    ($( $name:ident : $request:ident => $response:ident ),* $(,)?) => {
        #[tonic::async_trait]
        impl proto::kv9_server::Kv9 for Service {
            async fn raw_batch_get(&self, request: Request<proto::RawBatchGetRequest>) -> Result<Response<proto::RawBatchGetResponse>, Status> {
                let action = self.0.inspect(&request);
                let request = request.into_inner();
                assert_context(request.context);
                self.0.read_keys.lock().unwrap().push(request.keys.clone());
                match action {
                    Some(Action::Refuse { delay, status }) => {
                        tokio::time::sleep(delay).await;
                        return Err(status);
                    }
                    Some(Action::Hold { entered, release }) => {
                        entered.notify_one();
                        release.notified().await;
                        return Err(Status::unavailable("controlled lost batch read reply"));
                    }
                    Some(Action::ReadReply { values, control }) => {
                        let mut response = Response::new(proto::RawBatchGetResponse { values });
                        if control {
                            response.metadata_mut().insert(NOT_LEADER_KEY, "true".parse().unwrap());
                        }
                        return Ok(response);
                    }
                    Some(Action::WriteReply { .. }) => panic!("write action on read"),
                    None => {}
                }
                let values = self.0.values.lock().unwrap();
                Ok(Response::new(proto::RawBatchGetResponse {
                    values: request.keys.iter().map(|key| {
                        let value = values.get(key).cloned();
                        proto::OptionalValue { found: value.is_some(), value: value.unwrap_or_default() }
                    }).collect(),
                }))
            }

            async fn raw_batch_put(&self, request: Request<proto::RawBatchPutRequest>) -> Result<Response<proto::RawWriteResponse>, Status> {
                let action = self.0.inspect(&request);
                let request = request.into_inner();
                assert_context(request.context);
                self.0.write_pairs.lock().unwrap().push(request.pairs.clone());
                if let Some(Action::Refuse { delay, status }) = action {
                    tokio::time::sleep(delay).await;
                    return Err(status);
                }
                let index = self.0.writes.fetch_add(1, Ordering::SeqCst) as u64 + 1;
                {
                    let mut values = self.0.values.lock().unwrap();
                    for pair in request.pairs {
                        values.insert(pair.key, pair.value);
                    }
                }
                match action {
                    Some(Action::Hold { entered, release }) => {
                        // Every item is visible before the withheld response.
                        entered.notify_one();
                        release.notified().await;
                        Err(Status::unavailable("controlled applied batch reply loss"))
                    }
                    Some(Action::WriteReply { term, index }) => Ok(Response::new(proto::RawWriteResponse {
                        applied_term: term, applied_index: index,
                    })),
                    Some(Action::ReadReply { .. }) => panic!("read action on write"),
                    _ => Ok(Response::new(proto::RawWriteResponse { applied_term: 7, applied_index: index })),
                }
            }

            $(async fn $name(&self, _: Request<proto::$request>) -> Result<Response<proto::$response>, Status> {
                Err(Status::unimplemented("unused batch fixture RPC"))
            })*
        }
    }
}

batch_service! {
    raw_get: RawGetRequest => RawGetResponse,
    raw_put: RawPutRequest => RawWriteResponse,
    raw_delete: RawDeleteRequest => RawWriteResponse,
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
    shutdown: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl Server {
    async fn new() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let state = Arc::new(State::default());
        let service = Service(state.clone());
        let (shutdown, stopped) = oneshot::channel();
        let task = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(proto::kv9_server::Kv9Server::new(service))
                .serve_with_incoming_shutdown(
                    tokio_stream::wrappers::TcpListenerStream::new(listener),
                    async {
                        let _ = stopped.await;
                    },
                )
                .await
                .unwrap();
        });
        Self {
            state,
            address,
            shutdown: Some(shutdown),
            task,
        }
    }

    fn client(&self) -> PersistentRawClient {
        client(config(&[self.address]))
    }

    async fn stop(mut self) {
        self.shutdown.take().unwrap().send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), &mut self.task)
            .await
            .unwrap()
            .unwrap();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // Also bound cleanup when an assertion fails while an RPC is held.
        self.task.abort();
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
        keyspace_id: 19,
        epoch_conf_ver: 3,
        epoch_version: 5,
        max_in_flight: 4,
        max_attempts: 6,
        deadline_ms: 1500,
        retry_backoff_ms: 1,
    }
}

fn client(config: ClientConfig) -> PersistentRawClient {
    PersistentRawClient::new_with_transport(config, "batch-secret", TransportKind::TonicUnary)
        .unwrap()
}

fn context() -> Option<proto::RequestContext> {
    Some(proto::RequestContext {
        keyspace_id: 19,
        region_epoch: Some(proto::RegionEpoch {
            conf_ver: 3,
            version: 5,
        }),
    })
}

fn pairs() -> Vec<(Vec<u8>, Vec<u8>)> {
    vec![
        (b"a".to_vec(), b"one".to_vec()),
        (b"b".to_vec(), b"two".to_vec()),
    ]
}

fn not_leader(leader: &'static str) -> Status {
    let mut status = Status::failed_precondition("batch refused before execution");
    status
        .metadata_mut()
        .insert(NOT_LEADER_KEY, "true".parse().unwrap());
    status
        .metadata_mut()
        .insert(LEADER_HINT_KEY, leader.parse().unwrap());
    status
}

fn assert_rejected(report: &CallReport) {
    assert_eq!(
        report.outcome,
        Outcome::ClientRejected {
            reason: Reason::ClientInput
        }
    );
    assert_eq!(report.stop, Stop::ClientRejected);
    assert!(report.attempts.is_empty());
}

fn assert_read_protocol(report: &CallReport) {
    assert_eq!(report.operation, OperationKind::BatchGet);
    assert_eq!(
        report.outcome,
        Outcome::ReadFailure {
            reason: Reason::Protocol
        }
    );
    assert_eq!(report.attempts.len(), 1);
    assert_eq!(report.stop, Stop::Terminal);
}

#[tokio::test]
async fn batch_input_bounds_reject_locally_without_dispatch_or_splitting() {
    let server = Server::new().await;
    let client = server.client();
    let reads = [
        vec![],
        vec![vec![]; MAX_BATCH_ITEMS + 1],
        vec![vec![0; MAX_KEY_BYTES + 1]],
        vec![vec![0; MAX_KEY_BYTES]; MAX_BATCH_ITEMS],
    ];
    for keys in reads {
        assert_rejected(&client.batch_get(keys).await);
    }
    let writes = [
        vec![],
        vec![(vec![], vec![]); MAX_BATCH_ITEMS + 1],
        vec![(vec![0; MAX_KEY_BYTES + 1], vec![])],
        vec![(vec![], vec![0; MAX_VALUE_BYTES + 1])],
        vec![(vec![], vec![0; MAX_VALUE_BYTES]); 16],
    ];
    for pairs in writes {
        assert_rejected(&client.batch_put(pairs).await);
    }
    assert_eq!(server.state.requests.load(Ordering::SeqCst), 0);
    assert_eq!(server.state.writes.load(Ordering::SeqCst), 0);
    server.stop().await;
}

#[tokio::test]
async fn batch_get_exact_encoded_request_boundary_is_inclusive() {
    let server = Server::new().await;
    let client = server.client();
    let mut request = proto::RawBatchGetRequest {
        context: context(),
        keys: vec![vec![0; MAX_KEY_BYTES]; MAX_BATCH_ITEMS],
    };
    let excess = request.encoded_len() - MAX_MESSAGE_BYTES;
    request
        .keys
        .last_mut()
        .unwrap()
        .truncate(MAX_KEY_BYTES - excess);
    assert_eq!(request.encoded_len(), MAX_MESSAGE_BYTES);
    let report = client.batch_get(request.keys.clone()).await;
    assert_eq!(
        report.outcome,
        Outcome::Success {
            value: Value::BatchGet {
                values: vec![None; MAX_BATCH_ITEMS]
            }
        }
    );
    assert_eq!(report.attempts.len(), 1);
    request.keys.last_mut().unwrap().push(0);
    assert_eq!(request.encoded_len(), MAX_MESSAGE_BYTES + 1);
    assert_rejected(&client.batch_get(request.keys).await);
    assert_eq!(server.state.requests.load(Ordering::SeqCst), 1);
    server.stop().await;
}

#[tokio::test]
async fn batch_put_exact_encoded_request_boundary_is_inclusive() {
    let server = Server::new().await;
    let client = server.client();
    let mut request = proto::RawBatchPutRequest {
        context: context(),
        pairs: (0..16)
            .map(|key| proto::KeyValue {
                key: vec![key],
                value: vec![1; MAX_VALUE_BYTES],
            })
            .collect(),
    };
    let excess = request.encoded_len() - MAX_MESSAGE_BYTES;
    request
        .pairs
        .last_mut()
        .unwrap()
        .value
        .truncate(MAX_VALUE_BYTES - excess);
    assert_eq!(request.encoded_len(), MAX_MESSAGE_BYTES);
    let as_pairs = |request: &proto::RawBatchPutRequest| {
        request
            .pairs
            .iter()
            .map(|pair| (pair.key.clone(), pair.value.clone()))
            .collect()
    };
    let report = client.batch_put(as_pairs(&request)).await;
    assert_eq!(
        report.outcome,
        Outcome::Success {
            value: Value::Applied { term: 7, index: 1 }
        }
    );
    assert_eq!(report.attempts.len(), 1);
    request.pairs.last_mut().unwrap().value.push(1);
    assert_eq!(request.encoded_len(), MAX_MESSAGE_BYTES + 1);
    assert_rejected(&client.batch_put(as_pairs(&request)).await);
    assert_eq!(server.state.requests.load(Ordering::SeqCst), 1);
    assert_eq!(server.state.writes.load(Ordering::SeqCst), 1);
    server.stop().await;
}

#[tokio::test]
async fn batch_preserves_input_order_duplicates_missing_and_empty_values_with_one_receipt() {
    let server = Server::new().await;
    let client = server.client();
    let input = vec![
        (b"a".to_vec(), b"first".to_vec()),
        (b"b".to_vec(), vec![]),
        (b"a".to_vec(), b"last".to_vec()),
    ];
    let report = client.batch_put(input.clone()).await;
    assert_eq!(report.operation, OperationKind::BatchPut);
    assert_eq!(
        report.outcome,
        Outcome::Success {
            value: Value::Applied { term: 7, index: 1 }
        }
    );
    assert_eq!(report.attempts.len(), 1);
    {
        let captured = server.state.write_pairs.lock().unwrap();
        assert_eq!(captured.len(), 1);
        let actual: Vec<_> = captured[0]
            .iter()
            .map(|pair| (pair.key.clone(), pair.value.clone()))
            .collect();
        assert_eq!(actual, input);
    }
    let keys = vec![
        b"a".to_vec(),
        b"missing".to_vec(),
        b"b".to_vec(),
        b"a".to_vec(),
    ];
    let read = client.batch_get(keys.clone()).await;
    assert_eq!(read.operation, OperationKind::BatchGet);
    assert_eq!(
        read.outcome,
        Outcome::Success {
            value: Value::BatchGet {
                values: vec![
                    Some(b"last".to_vec()),
                    None,
                    Some(vec![]),
                    Some(b"last".to_vec())
                ],
            }
        }
    );
    assert_eq!(*server.state.read_keys.lock().unwrap(), vec![keys]);
    assert_eq!(server.state.writes.load(Ordering::SeqCst), 1);
    assert_eq!(server.state.requests.load(Ordering::SeqCst), 2);
    server.stop().await;
}

#[tokio::test]
async fn batch_item_count_limit_allows_256_items_in_one_write() {
    let server = Server::new().await;
    let report = server
        .client()
        .batch_put(
            (0..MAX_BATCH_ITEMS)
                .map(|i| (i.to_le_bytes().to_vec(), vec![]))
                .collect(),
        )
        .await;
    assert_eq!(
        report.outcome,
        Outcome::Success {
            value: Value::Applied { term: 7, index: 1 }
        }
    );
    assert_eq!(report.attempts.len(), 1);
    assert_eq!(
        server.state.write_pairs.lock().unwrap()[0].len(),
        MAX_BATCH_ITEMS
    );
    assert_eq!(server.state.writes.load(Ordering::SeqCst), 1);
    server.stop().await;
}

#[tokio::test]
async fn batch_read_malformed_vectors_are_terminal_read_failures() {
    let server = Server::new().await;
    let client = server.client();
    let absent = proto::OptionalValue {
        found: false,
        value: vec![],
    };
    let cases = [
        (vec![], false),
        (vec![absent.clone()], false),
        (vec![absent.clone(); 3], false),
        (
            vec![
                proto::OptionalValue {
                    found: false,
                    value: vec![1],
                },
                absent.clone(),
            ],
            false,
        ),
        (
            vec![
                proto::OptionalValue {
                    found: true,
                    value: vec![1; MAX_VALUE_BYTES + 1],
                },
                absent.clone(),
            ],
            false,
        ),
        (vec![absent; 2], true),
    ];
    for (values, control) in cases {
        server.state.push(Action::ReadReply { values, control });
        assert_read_protocol(&client.batch_get(vec![b"a".to_vec(), b"b".to_vec()]).await);
    }
    assert_eq!(server.state.requests.load(Ordering::SeqCst), 6);
    server.stop().await;
}

#[tokio::test]
async fn batch_read_encoded_response_limit_is_enforced_without_retry() {
    let server = Server::new().await;
    let client = server.client();
    let mut response = proto::RawBatchGetResponse {
        values: vec![
            proto::OptionalValue {
                found: true,
                value: vec![1; MAX_VALUE_BYTES]
            };
            16
        ],
    };
    let excess = response.encoded_len() - MAX_MESSAGE_BYTES;
    response
        .values
        .last_mut()
        .unwrap()
        .value
        .truncate(MAX_VALUE_BYTES - excess);
    assert_eq!(response.encoded_len(), MAX_MESSAGE_BYTES);
    let expected: Vec<_> = response
        .values
        .iter()
        .map(|value| Some(value.value.clone()))
        .collect();
    server.state.push(Action::ReadReply {
        values: response.values.clone(),
        control: false,
    });
    assert_eq!(
        client.batch_get(vec![b"a".to_vec(); 16]).await.outcome,
        Outcome::Success {
            value: Value::BatchGet { values: expected }
        }
    );
    response.values.last_mut().unwrap().value.push(1);
    assert_eq!(response.encoded_len(), MAX_MESSAGE_BYTES + 1);
    server.state.push(Action::ReadReply {
        values: response.values,
        control: false,
    });
    let report = client.batch_get(vec![b"a".to_vec(); 16]).await;
    // A transport decoder may reject the size before SDK semantic validation.
    assert!(matches!(report.outcome, Outcome::ReadFailure { .. }));
    assert_eq!(report.attempts.len(), 1);
    assert_eq!(server.state.requests.load(Ordering::SeqCst), 2);
    server.stop().await;
}

#[tokio::test]
async fn batch_put_lost_applied_reply_is_unknown_and_never_replayed() {
    let first = Server::new().await;
    let second = Server::new().await;
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    first.state.push(Action::Hold {
        entered: entered.clone(),
        release: release.clone(),
    });
    let client = client(config(&[first.address, second.address]));
    let call = tokio::spawn(async move { client.batch_put(pairs()).await });
    tokio::time::timeout(Duration::from_secs(2), entered.notified())
        .await
        .unwrap();
    {
        let values = first.state.values.lock().unwrap();
        assert_eq!(values.get(b"a".as_slice()), Some(&b"one".to_vec()));
        assert_eq!(values.get(b"b".as_slice()), Some(&b"two".to_vec()));
    }
    release.notify_one();
    let report = tokio::time::timeout(Duration::from_secs(2), call)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(report.operation, OperationKind::BatchPut);
    assert_eq!(
        report.outcome,
        Outcome::UnknownWrite {
            reason: Reason::RpcStatus {
                code: Code::Unavailable as i32
            }
        }
    );
    assert_eq!(report.attempts.len(), 1);
    assert_eq!(first.state.writes.load(Ordering::SeqCst), 1);
    assert_eq!(second.state.requests.load(Ordering::SeqCst), 0);
    first.stop().await;
    second.stop().await;
}

#[tokio::test]
async fn batch_put_invalid_whole_receipt_is_unknown_without_retry() {
    let server = Server::new().await;
    let client = server.client();
    for (term, index) in [(0, 9), (7, 0)] {
        server.state.push(Action::WriteReply { term, index });
        let report = client.batch_put(pairs()).await;
        assert_eq!(
            report.outcome,
            Outcome::UnknownWrite {
                reason: Reason::Protocol
            }
        );
        assert_eq!(report.attempts.len(), 1);
    }
    assert_eq!(server.state.requests.load(Ordering::SeqCst), 2);
    assert_eq!(server.state.writes.load(Ordering::SeqCst), 2);
    server.stop().await;
}

#[tokio::test]
async fn batch_not_leader_retries_the_whole_payload_only_after_typed_refusal() {
    for read in [false, true] {
        let first = Server::new().await;
        let second = Server::new().await;
        first.state.push(Action::Refuse {
            delay: Duration::ZERO,
            status: not_leader("2"),
        });
        let client = client(config(&[first.address, second.address]));
        let report = if read {
            client.batch_get(vec![b"a".to_vec(), b"a".to_vec()]).await
        } else {
            client.batch_put(pairs()).await
        };
        assert!(matches!(report.outcome, Outcome::Success { .. }));
        assert_eq!(report.attempts.len(), 2);
        assert_eq!(
            report.attempts[0].failure,
            Some(Reason::NotLeader { leader: Some(2) })
        );
        assert_eq!(report.attempts[1].node_id, 2);
        assert_eq!(first.state.writes.load(Ordering::SeqCst), 0);
        assert_eq!(
            second.state.writes.load(Ordering::SeqCst),
            usize::from(!read)
        );
        if read {
            assert_eq!(
                *first.state.read_keys.lock().unwrap(),
                *second.state.read_keys.lock().unwrap()
            );
        } else {
            assert_eq!(
                *first.state.write_pairs.lock().unwrap(),
                *second.state.write_pairs.lock().unwrap()
            );
        }
        first.stop().await;
        second.stop().await;
    }
}

#[tokio::test]
async fn batch_redirect_preserves_original_deadline_for_reads_and_writes() {
    for read in [false, true] {
        let first = Server::new().await;
        let second = Server::new().await;
        first.state.push(Action::Refuse {
            delay: Duration::from_millis(120),
            status: not_leader("2"),
        });
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        second.state.push(Action::Hold {
            entered: entered.clone(),
            release: release.clone(),
        });
        let mut config = config(&[first.address, second.address]);
        config.deadline_ms = 500;
        let client = client(config);
        let call = tokio::spawn(async move {
            if read {
                client.batch_get(vec![b"a".to_vec()]).await
            } else {
                client.batch_put(pairs()).await
            }
        });
        tokio::time::timeout(Duration::from_secs(2), entered.notified())
            .await
            .unwrap();
        let report = tokio::time::timeout(Duration::from_secs(2), call)
            .await
            .unwrap()
            .unwrap();
        release.notify_one();
        assert_eq!(report.attempts.len(), 2);
        assert!(matches!(
            report.attempts[0].failure,
            Some(Reason::NotLeader { .. })
        ));
        if read {
            assert!(matches!(report.outcome, Outcome::ReadFailure { .. }));
        } else {
            assert!(matches!(report.outcome, Outcome::UnknownWrite { .. }));
        }
        let original = first.state.timeouts.lock().unwrap()[0];
        let remaining = second.state.timeouts.lock().unwrap()[0];
        assert!(
            remaining + Duration::from_millis(80) < original,
            "redirect reset deadline: first={original:?}, second={remaining:?}"
        );
        assert_eq!(first.state.requests.load(Ordering::SeqCst), 1);
        assert_eq!(second.state.requests.load(Ordering::SeqCst), 1);
        first.stop().await;
        second.stop().await;
    }
}

#[tokio::test]
async fn batch_uses_one_logical_client_permit_until_terminal_observation() {
    let server = Server::new().await;
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    server.state.push(Action::Hold {
        entered: entered.clone(),
        release: release.clone(),
    });
    let mut config = config(&[server.address]);
    config.max_in_flight = 1;
    let client = client(config);
    let writer = client.clone();
    let call = tokio::spawn(async move { writer.batch_put(pairs()).await });
    tokio::time::timeout(Duration::from_secs(2), entered.notified())
        .await
        .unwrap();
    let refused = client.batch_get(vec![b"a".to_vec()]).await;
    assert_eq!(
        refused.outcome,
        Outcome::ClientRejected {
            reason: Reason::ClientCapacity
        }
    );
    assert!(refused.attempts.is_empty());
    assert_eq!(server.state.requests.load(Ordering::SeqCst), 1);
    release.notify_one();
    let report = tokio::time::timeout(Duration::from_secs(2), call)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(report.outcome, Outcome::UnknownWrite { .. }));
    assert_eq!(
        client
            .batch_get(vec![b"a".to_vec(), b"b".to_vec()])
            .await
            .outcome,
        Outcome::Success {
            value: Value::BatchGet {
                values: vec![Some(b"one".to_vec()), Some(b"two".to_vec())]
            }
        }
    );
    assert_eq!(server.state.requests.load(Ordering::SeqCst), 2);
    server.stop().await;
}
