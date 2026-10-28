use super::*;
use crate::api::RawRouteLookup;
use crate::grpc::Kv9Grpc;
use crate::point_test_support::{memory_channel, Backend};
use kv9_common::{Error, KeyspaceId, NodeId, RegionId, StoreIncarnation, TenantId};
use kv9_meta::endpoint::NodeEndpoint;
use std::sync::atomic::{AtomicBool, AtomicUsize};

type Page = (Vec<(Vec<u8>, Vec<u8>)>, Option<Vec<u8>>, usize);
struct State {
    ranges: Mutex<Vec<DataRange>>,
    calls: Mutex<Vec<(u64, u64, Operation)>>,
    lookups: Mutex<Vec<(u64, Vec<u8>)>>,
    lookup_mode: AtomicUsize,
    wrong_scope: AtomicBool,
    unsupported: AtomicBool,
    lose_data: AtomicBool,
    data_entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
    pages: Mutex<BTreeMap<Vec<u8>, Page>>,
}
fn binding(region: u64, start: &[u8], end: &[u8]) -> DataRange {
    DataRange {
        root: RootDigest::from_bytes([1; 32]),
        creation: RootDigest::from_bytes([region as u8; 32]),
        region: RegionId(region),
        keyspace: KeyspaceId(100),
        tenant: TenantId(1),
        conf_ver: 1,
        version: 1,
        start: start.to_vec(),
        end: end.to_vec(),
        sealed: false,
    }
}
fn endpoints() -> Vec<NodeEndpoint> {
    (1..=3)
        .map(|id| NodeEndpoint {
            node: NodeId(id),
            incarnation: StoreIncarnation::from_bytes([id as u8; 16]),
            address: format!("127.0.0.1:{}", 23000 + id).parse().unwrap(),
            generation: 0,
            previous_address: None,
            active: true,
        })
        .collect()
}
struct Fixture {
    client: RoutedTtlClient,
    state: Arc<State>,
    servers: Vec<tokio::task::JoinHandle<()>>,
    admissions: Vec<Arc<crate::admission::PublicAdmission>>,
}
impl Fixture {
    async fn new(transport: TransportKind, deadline_ms: u64, max_attempts: usize) -> Self {
        let state = Arc::new(State {
            ranges: Mutex::new(vec![
                binding(100, b"", b"m"),
                binding(101, b"m", b"t"),
                binding(102, b"t", b""),
            ]),
            calls: Mutex::default(),
            lookups: Mutex::default(),
            lookup_mode: AtomicUsize::new(0),
            wrong_scope: AtomicBool::new(false),
            unsupported: AtomicBool::new(false),
            lose_data: AtomicBool::new(false),
            data_entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
            pages: Mutex::default(),
        });
        let config = RoutedConfig {
            version: 1,
            root_digest: [1; 32],
            tenant_id: 1,
            keyspace_id: 100,
            seeds: endpoints()
                .into_iter()
                .take(2)
                .map(|e| Peer {
                    node_id: e.node.0,
                    address: e.address,
                })
                .collect(),
            max_in_flight: 4,
            max_attempts,
            deadline_ms,
            probe_timeout_ms: 50.min(deadline_ms),
            retry_backoff_ms: 1,
            cache_capacity: 2,
        };
        let client = RoutedTtlClient::new_with_transport(config, "secret", transport).unwrap();
        let mut servers = Vec::new();
        let mut admissions = Vec::new();
        for endpoint in endpoints() {
            let id = endpoint.node.0;
            let backend = Arc::new(Backend {
                route_lookup: Some(Arc::new({
                    let state = state.clone();
                    move |root, tenant, keyspace, key| {
                        assert_eq!(root, RootDigest::from_bytes([1; 32]));
                        assert_eq!(tenant, TenantId(1));
                        assert_eq!(keyspace, KeyspaceId(100));
                        state.lookups.lock().unwrap().push((id, key.to_vec()));
                        let mode = state.lookup_mode.load(Ordering::SeqCst);
                        if mode == 1 && id == 1 {
                            std::thread::sleep(Duration::from_millis(200));
                        }
                        let mut range = state
                            .ranges
                            .lock()
                            .unwrap()
                            .iter()
                            .find(|r| r.contains(key))
                            .cloned()
                            .unwrap();
                        if state.wrong_scope.load(Ordering::SeqCst) {
                            range.tenant = TenantId(99);
                        }
                        let hints_only = mode == 2 && id != 3;
                        Ok(RawRouteLookup {
                            root,
                            metadata_peers: endpoints(),
                            metadata_leader: Some(NodeId(if id == 1 { 2 } else { 1 })),
                            range: (!hints_only).then_some(range),
                            replicas: if hints_only { vec![] } else { endpoints() },
                            data_leader: (!hints_only).then_some(NodeId(1)), // Intentionally stale: actual leader below is 3.
                        })
                    }
                })),
                route_target: Some(Arc::new({
                    let state = state.clone();
                    move |scope| {
                        if !state.ranges.lock().unwrap().contains(scope) {
                            return Err(Error::StaleEpoch {
                                region: scope.region,
                            });
                        }
                        let region = scope.region.0;
                        let state = state.clone();
                        Ok(Arc::new(Backend {
                            ttl_prepare: Some(Arc::new(move |_, op, _| {
                                let state = state.clone();
                                Box::new(move || {
                                    state.calls.lock().unwrap().push((id, region, op.clone()));
                                    if state.unsupported.load(Ordering::SeqCst) {
                                        return Err(Error::NotImplemented("old endpoint"));
                                    }
                                    if id != 3 {
                                        return Err(Error::NotLeader {
                                            leader: Some(NodeId(3)),
                                        });
                                    }
                                    Ok(Box::pin(async move {
                                        if matches!(op, Operation::Capabilities) {
                                            return Ok(Reply::Capabilities);
                                        }
                                        if state.lose_data.load(Ordering::SeqCst) {
                                            state.data_entered.notify_one();
                                            state.release.notified().await;
                                        }
                                        Ok(match op {
                                            Operation::Get { keys } => Reply::Values {
                                                values: keys.into_iter().map(Some).collect(),
                                            },
                                            Operation::Scan { start, .. } => {
                                                let (rows, next, examined) = state
                                                    .pages
                                                    .lock()
                                                    .unwrap()
                                                    .get(&start)
                                                    .cloned()
                                                    .unwrap_or_default();
                                                Reply::Page {
                                                    rows,
                                                    next,
                                                    examined,
                                                }
                                            }
                                            Operation::DeleteRange { start, .. } => {
                                                let (rows, next, _) = state
                                                    .pages
                                                    .lock()
                                                    .unwrap()
                                                    .get(&start)
                                                    .cloned()
                                                    .unwrap_or_default();
                                                Reply::Deleted {
                                                    processed_keys: rows.len(),
                                                    applied: (!rows.is_empty()).then_some(
                                                        crate::raw_ttl::Receipt {
                                                            term: 1,
                                                            index: 2,
                                                        },
                                                    ),
                                                    next,
                                                }
                                            }
                                            _ => Reply::Applied { term: 1, index: 2 },
                                        })
                                    }))
                                })
                            })),
                            ..Backend::default()
                        }) as Arc<dyn crate::api::RawApi>)
                    }
                })),
                ..Backend::default()
            });
            let api = Kv9Grpc::new(backend);
            admissions.push(api.admission());
            let (channel, server) = memory_channel(api).await;
            client.inject_test_channel(endpoint.address, channel);
            servers.push(server);
        }
        Self {
            client,
            state,
            servers,
            admissions,
        }
    }
    async fn close(self) {
        tokio::time::timeout(Duration::from_secs(3), async {
            while self.admissions.iter().any(|a| a.snapshot().in_flight > 0) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        for server in self.servers {
            server.abort();
            let _ = server.await;
        }
    }
}
fn put() -> Operation {
    Operation::Put {
        pairs: vec![(b"a".to_vec(), b"value".to_vec())],
        ttl_secs: Some(60),
    }
}
fn success(report: &Report) {
    assert!(
        matches!(report.outcome, Outcome::Success { .. }),
        "{report:?}"
    );
}

#[tokio::test]
async fn discovery_redirects_capabilities_and_refreshes_a_stale_scope_without_replaying_data() {
    for transport in [TransportKind::TonicUnary, TransportKind::TonicStream] {
        let f = Fixture::new(transport, 2000, 12).await;
        let first = f.client.call(put()).await;
        success(&first);
        assert_eq!(
            first.attempts.iter().map(|a| a.phase).collect::<Vec<_>>(),
            vec![
                Phase::Lookup,
                Phase::Capability,
                Phase::Capability,
                Phase::Data
            ]
        );
        assert_eq!(
            first.attempts.iter().map(|a| a.node_id).collect::<Vec<_>>(),
            vec![1, 1, 3, 3]
        );
        f.state.ranges.lock().unwrap()[0].version += 1;
        let second = f.client.call(put()).await;
        success(&second);
        assert!(second.attempts.iter().any(|a| a.failure
            == Some(Failure::Ttl {
                failure: TtlFailure::Refused {
                    refusal: Refusal::StaleScope
                }
            })));
        assert_eq!(
            second
                .attempts
                .iter()
                .filter(|a| a.phase == Phase::Data)
                .count(),
            1
        );
        assert_eq!(
            f.state
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, _, op)| matches!(op, Operation::Put { .. }))
                .count(),
            2
        );
        assert!(f.client.0 .0.routes.lock().unwrap().len() <= 2);
        f.close().await;
    }
}

#[tokio::test]
async fn discovery_fails_over_and_stale_hints_cannot_starve_learned_metadata_peers() {
    for mode in [1, 2] {
        let f = Fixture::new(TransportKind::TonicUnary, 2000, 12).await;
        f.state.lookup_mode.store(mode, Ordering::SeqCst);
        let report = f.client.call(put()).await;
        success(&report);
        let lookups = report
            .attempts
            .iter()
            .filter(|a| a.phase == Phase::Lookup)
            .map(|a| a.node_id)
            .collect::<Vec<_>>();
        assert_eq!(lookups, if mode == 1 { vec![1, 2] } else { vec![1, 2, 3] });
        f.close().await;
    }
}

#[tokio::test]
async fn wrong_namespace_unsupported_ttl_and_cross_range_batch_never_dispatch_data() {
    for mode in 0..4 {
        let f = Fixture::new(TransportKind::TonicStream, 1000, 6).await;
        if mode == 0 {
            f.state.wrong_scope.store(true, Ordering::SeqCst);
        }
        if mode == 1 {
            f.state.unsupported.store(true, Ordering::SeqCst);
        }
        let operation = match mode {
            2 => Operation::Put {
                pairs: vec![(b"a".to_vec(), vec![]), (b"z".to_vec(), vec![])],
                ttl_secs: Some(0),
            },
            3 => Operation::Get {
                keys: vec![b"a".to_vec(), b"z".to_vec()],
            },
            _ => put(),
        };
        let report = f.client.call(operation).await;
        assert!(
            !matches!(report.outcome, Outcome::Success { .. }),
            "{report:?}"
        );
        assert!(report.attempts.iter().all(|a| a.phase != Phase::Data));
        assert!(f
            .state
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|(_, _, op)| matches!(op, Operation::Capabilities)));
        f.close().await;
    }
}

#[tokio::test]
async fn lost_data_reply_is_terminal_and_cancellation_keeps_server_admission() {
    for transport in [TransportKind::TonicUnary, TransportKind::TonicStream] {
        let f = Fixture::new(transport, 300, 12).await;
        f.state.lose_data.store(true, Ordering::SeqCst);
        let report = f.client.call(put()).await;
        assert!(
            matches!(report.outcome, Outcome::UnknownWrite { .. }),
            "{report:?}"
        );
        assert_eq!(
            report
                .attempts
                .iter()
                .filter(|a| a.phase == Phase::Data)
                .count(),
            1
        );
        assert_eq!(f.admissions[2].snapshot().in_flight, 1);
        f.state.release.notify_one();
        f.close().await;
    }
}

#[tokio::test]
async fn scan_continues_across_expired_and_empty_pages_and_range_boundaries() {
    for transport in [TransportKind::TonicUnary, TransportKind::TonicStream] {
        let f = Fixture::new(transport, 2000, 12).await;
        f.state.pages.lock().unwrap().extend([
            (vec![], (vec![], Some(b"b".to_vec()), 4096)),
            (b"b".to_vec(), (vec![(b"c".to_vec(), vec![])], None, 1)),
            (
                b"t".to_vec(),
                (vec![(b"z".to_vec(), b"z".to_vec())], None, 1),
            ),
        ]);
        let first = f
            .client
            .call(Operation::Scan {
                start: vec![],
                end: vec![],
                limit: 10,
            })
            .await;
        success(&first);
        let mut cursor = first.continuation.unwrap();
        assert_eq!(cursor.next(), b"b");
        let second = f.client.continue_scan(&cursor, 10).await;
        success(&second);
        cursor =
            serde_json::from_slice(&serde_json::to_vec(&second.continuation.unwrap()).unwrap())
                .unwrap();
        assert_eq!(cursor.next(), b"m");
        let empty = f.client.continue_scan(&cursor, 10).await;
        success(&empty);
        assert!(
            matches!(empty.outcome, Outcome::Success { reply: Reply::Page { rows, examined: 0, .. } } if rows.is_empty())
        );
        cursor = empty.continuation.unwrap();
        assert_eq!(cursor.next(), b"t");
        let last = f.client.continue_scan(&cursor, 10).await;
        success(&last);
        assert!(last.continuation.is_none());
        let ends = f
            .state
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(_, _, op)| {
                if let Operation::Scan { end, .. } = op {
                    Some(end.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(
            ends,
            vec![b"m".to_vec(), b"m".to_vec(), b"t".to_vec(), vec![]]
        );
        f.close().await;
    }
}

#[tokio::test]
async fn cursor_is_query_bound_and_survives_topology_replacement_and_maximum_key() {
    let f = Fixture::new(TransportKind::TonicUnary, 2000, 12).await;
    let first = f
        .client
        .call(Operation::Scan {
            start: vec![],
            end: b"z".to_vec(),
            limit: 10,
        })
        .await;
    let cursor = first.continuation.unwrap();
    for mode in 0..7 {
        let mut bad = cursor.clone();
        match mode {
            0 => bad.version = 2,
            1 => bad.root_digest = [8; 32],
            2 => bad.tenant_id += 1,
            3 => bad.keyspace_id += 1,
            4 => bad.next = vec![],
            5 => bad.next = bad.end.clone(),
            _ => bad.next = vec![8; crate::raw_ttl::MAX_RANGE_BOUND_BYTES + 2],
        }
        let report = f.client.continue_scan(&bad, 10).await;
        assert!(matches!(report.outcome, Outcome::ClientRejected { .. }));
        assert!(report.attempts.is_empty());
    }
    *f.state.ranges.lock().unwrap() = vec![binding(103, b"", b"")];
    let report = f.client.continue_scan(&cursor, 10).await;
    success(&report);
    assert!(report.continuation.is_none()); // finite end z, now in one replacement range
    let mut max_cursor = cursor.clone();
    max_cursor.end.clear();
    max_cursor.next = vec![255; MAX_KEY_BYTES];
    max_cursor.next.push(0);
    f.client.0 .0.routes.lock().unwrap().clear();
    let report = f.client.continue_scan(&max_cursor, 10).await;
    success(&report);
    assert!(f
        .state
        .lookups
        .lock()
        .unwrap()
        .iter()
        .any(|(_, key)| key == &max_cursor.next));
    f.close().await;
}

#[tokio::test]
async fn shared_attempt_budget_cannot_turn_capability_success_into_write_success() {
    let f = Fixture::new(TransportKind::TonicUnary, 2000, 3).await;
    let report = f.client.call(put()).await;
    assert_eq!(report.stop, Stop::AttemptLimit);
    assert_eq!(report.outcome, Outcome::NotDispatched);
    assert_eq!(report.attempts.len(), 3); // Lookup, stale leader, successful capability.
    assert!(f
        .state
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|(_, _, op)| matches!(op, Operation::Capabilities)));
    f.close().await;
}

#[tokio::test]
async fn discovery_cannot_reset_the_call_deadline_or_dispatch_after_it() {
    let f = Fixture::new(TransportKind::TonicUnary, 30, 12).await;
    f.state.lookup_mode.store(1, Ordering::SeqCst);
    let report = f.client.call(put()).await;
    assert_eq!(report.stop, Stop::Deadline);
    assert!(report.attempts.iter().all(|a| a.phase == Phase::Lookup));
    assert!(f.state.calls.lock().unwrap().is_empty());
    f.close().await;
}

#[tokio::test]
async fn encoded_scope_overhead_is_checked_before_sending_data() {
    let f = Fixture::new(TransportKind::TonicUnary, 2000, 12).await;
    let pairs = (0..16)
        .map(|_| (b"a".to_vec(), vec![0; MAX_VALUE_BYTES - 1]))
        .collect();
    let operation = Operation::Put {
        pairs,
        ttl_secs: None,
    };
    assert!(operation.validate().is_ok()); // Exactly 1 MiB before protobuf/scope overhead.
    let report = f.client.call(operation).await;
    assert!(matches!(
        report.outcome,
        Outcome::ClientRejected {
            reason: Reason::ClientInput
        }
    ));
    assert!(report.attempts.iter().all(|a| a.phase == Phase::Lookup));
    assert!(f.state.calls.lock().unwrap().is_empty());
    f.close().await;
}

#[tokio::test]
async fn long_catalog_boundaries_advance_across_empty_ranges_without_truncation() {
    for transport in [TransportKind::TonicUnary, TransportKind::TonicStream] {
        let f = Fixture::new(transport, 5000, 12).await;
        let b = vec![1; crate::raw_ttl::MAX_RANGE_BOUND_BYTES];
        let c = vec![2; crate::raw_ttl::MAX_RANGE_BOUND_BYTES];
        *f.state.ranges.lock().unwrap() = vec![
            binding(100, b"", &b),
            binding(101, &b, &c),
            binding(102, &c, b""),
        ];
        let first = f
            .client
            .call(Operation::Scan {
                start: vec![],
                end: vec![],
                limit: 16,
            })
            .await;
        success(&first);
        let cursor = first.continuation.unwrap();
        assert_eq!(cursor.next(), b);
        let second = f.client.continue_scan(&cursor, 16).await;
        success(&second);
        let cursor = second.continuation.unwrap();
        assert_eq!(cursor.next(), c);
        let last = f.client.continue_scan(&cursor, 16).await;
        success(&last);
        assert!(last.continuation.is_none());
        f.close().await;
    }
}

#[tokio::test]
async fn delete_continuation_keeps_confirmed_pages_and_stops_on_an_unknown_chunk() {
    for transport in [TransportKind::TonicUnary, TransportKind::TonicStream] {
        let f = Fixture::new(transport, 300, 12).await;
        f.state
            .pages
            .lock()
            .unwrap()
            .insert(vec![], (vec![(b"a".to_vec(), vec![])], None, 1));
        let first = f
            .client
            .call(Operation::DeleteRange {
                start: vec![],
                end: vec![],
                limit: 16,
            })
            .await;
        success(&first);
        assert!(matches!(
            first.outcome,
            Outcome::Success {
                reply: Reply::Deleted {
                    processed_keys: 1,
                    applied: Some(_),
                    ..
                }
            }
        ));
        assert!(first.continuation.is_none());
        let cursor = first.delete_continuation.unwrap();
        assert_eq!(cursor.next(), b"m");
        let empty = f.client.continue_delete_range(&cursor, 16).await;
        success(&empty);
        assert!(matches!(
            empty.outcome,
            Outcome::Success {
                reply: Reply::Deleted {
                    processed_keys: 0,
                    applied: None,
                    ..
                }
            }
        ));
        let cursor: DeleteCursor = serde_json::from_slice(
            &serde_json::to_vec(&empty.delete_continuation.unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(cursor.next(), b"t");
        f.state.lose_data.store(true, Ordering::SeqCst);
        let failed = f.client.continue_delete_range(&cursor, 16).await;
        assert!(
            matches!(failed.outcome, Outcome::UnknownWrite { .. }),
            "{failed:?}"
        );
        assert_eq!(
            failed
                .attempts
                .iter()
                .filter(|a| a.phase == Phase::Data)
                .count(),
            1
        );
        assert!(failed.delete_continuation.is_none());
        assert_eq!(f.admissions[2].snapshot().in_flight, 1);
        f.state.release.notify_one();
        f.close().await;
    }
}
