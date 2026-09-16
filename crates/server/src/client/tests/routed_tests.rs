use super::*;
use crate::client::routed::{Failure, Phase, RoutedConfig, RoutedOutcome, RoutedRawClient};
use kv9_common::{data_range::DataRange, KeyspaceId, RegionId, RootDigest, TenantId};

async fn fixture() -> (Vec<Server>, RoutedConfig) {
    let servers = vec![
        Server::new().await,
        Server::new().await,
        Server::new().await,
    ];
    let scope = DataRange {
        root: RootDigest::from_bytes([1; 32]),
        creation: RootDigest::from_bytes([2; 32]),
        region: RegionId(100),
        keyspace: KeyspaceId(100),
        tenant: TenantId(1),
        conf_ver: 1,
        version: 1,
        start: vec![],
        end: vec![],
        sealed: false,
    };
    let peers: Vec<_> = servers
        .iter()
        .enumerate()
        .map(|(i, s)| proto::NodeEndpoint {
            node_id: i as u64 + 1,
            store_incarnation: vec![i as u8 + 1; 16],
            address: s.address.to_string(),
            generation: 0,
            previous_address: None,
            active: true,
        })
        .collect();
    for server in &servers {
        *server.state.route.lock().unwrap() = Some(proto::LookupRawRouteResponse {
            root_digest: vec![1; 32],
            metadata_peers: peers.clone(),
            metadata_leader: Some(1),
            binding: scope.encode(),
            replicas: peers.clone(),
            data_leader: Some(1),
        });
    }
    let config = RoutedConfig {
        version: 1,
        root_digest: [1; 32],
        tenant_id: 1,
        keyspace_id: 100,
        seeds: servers
            .iter()
            .enumerate()
            .map(|(i, s)| Peer {
                node_id: i as u64 + 1,
                address: s.address,
            })
            .collect(),
        max_in_flight: 4,
        max_attempts: 10,
        deadline_ms: 1000,
        probe_timeout_ms: 100,
        retry_backoff_ms: 1,
        cache_capacity: 4,
    };
    (servers, config)
}
async fn stop(servers: Vec<Server>, client: RoutedRawClient) {
    drop(client);
    for server in servers {
        server.stop().await;
    }
}
fn client(config: RoutedConfig) -> RoutedRawClient {
    RoutedRawClient::new_with_transport(config, "test-secret", TransportKind::TonicUnary).unwrap()
}

#[tokio::test]
async fn routed_discovery_skips_a_silent_seed_within_the_original_deadline() {
    let (servers, config) = fixture().await;
    servers[0]
        .state
        .lookup_delay_ms
        .store(700, Ordering::SeqCst);
    let c = client(config);
    let started = Instant::now();
    let report = c.call(get()).await;
    assert!(
        matches!(report.outcome, RoutedOutcome::Success { .. }),
        "{report:?}"
    );
    assert!(started.elapsed() < Duration::from_millis(600));
    assert_eq!(report.attempts[0].phase, Phase::Lookup);
    assert!(report.attempts[0].failure.is_some());
    assert_eq!(report.attempts[1].node_id, 2);
    stop(servers, c).await;
}

#[tokio::test]
async fn routed_stale_metadata_hints_cannot_starve_a_learned_third_endpoint() {
    let (servers, mut config) = fixture().await;
    config.seeds.truncate(2);
    config.max_attempts = 4;
    for (i, server) in servers.iter().enumerate().take(2) {
        let mut route = server.state.route.lock().unwrap();
        let route = route.as_mut().unwrap();
        route.binding.clear();
        route.replicas.clear();
        route.data_leader = None;
        route.metadata_leader = Some(2 - i as u64);
    }
    servers[2]
        .state
        .route
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .data_leader = Some(3);
    let c = client(config);
    let report = c.call(get()).await;
    assert!(
        matches!(report.outcome, RoutedOutcome::Success { .. }),
        "{report:?}"
    );
    assert_eq!(
        report
            .attempts
            .iter()
            .map(|a| a.node_id)
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 3]
    );
    stop(servers, c).await;
}

#[tokio::test]
async fn routed_unknown_applied_write_has_exactly_one_data_attempt() {
    let (servers, mut config) = fixture().await;
    config.deadline_ms = 180;
    let applied = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    servers[0]
        .state
        .actions
        .lock()
        .unwrap()
        .push_back(Action::LoseReply {
            applied: applied.clone(),
            release: release.clone(),
        });
    let c = client(config);
    let caller = c.clone();
    let operation = tokio::spawn(async move { caller.call(put(b"applied-once")).await });
    tokio::time::timeout(Duration::from_secs(1), applied.notified())
        .await
        .unwrap();
    let report = operation.await.unwrap();
    assert!(
        matches!(report.outcome, RoutedOutcome::UnknownWrite { .. }),
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
    assert_eq!(
        servers
            .iter()
            .map(|s| s.state.writes.load(Ordering::SeqCst))
            .sum::<usize>(),
        1
    );
    assert_eq!(
        servers[0]
            .state
            .values
            .lock()
            .unwrap()
            .get(b"key".as_slice())
            .unwrap(),
        b"applied-once"
    );
    release.notify_waiters();
    stop(servers, c).await;
}

#[tokio::test]
async fn routed_stale_scope_refreshes_and_rejects_crossing_batches_without_dispatch() {
    let (servers, config) = fixture().await;
    let c = client(config);
    assert!(matches!(
        c.call(put(b"first")).await.outcome,
        RoutedOutcome::Success { .. }
    ));
    for server in &servers {
        let mut guard = server.state.route.lock().unwrap();
        let route = guard.as_mut().unwrap();
        let mut scope = DataRange::decode(&route.binding).unwrap();
        scope.version = 2;
        scope.start = b"b".to_vec();
        scope.end = b"m".to_vec();
        route.binding = scope.encode();
    }
    let report = c.call(put(b"second")).await;
    assert!(
        matches!(report.outcome, RoutedOutcome::Success { .. }),
        "{report:?}"
    );
    assert_eq!(
        report
            .attempts
            .iter()
            .filter(|a| a.phase == Phase::Data)
            .count(),
        2
    );
    assert!(report
        .attempts
        .iter()
        .any(|a| a.failure == Some(Failure::ScopeRefused)));
    assert_eq!(servers[0].state.writes.load(Ordering::SeqCst), 2);
    let before: usize = servers
        .iter()
        .map(|s| s.state.requests.load(Ordering::SeqCst))
        .sum();
    let report = c
        .call(RawOperation::BatchPut {
            pairs: vec![(b"key".to_vec(), vec![8]), (b"z".to_vec(), vec![9])],
        })
        .await;
    assert!(matches!(
        report.outcome,
        RoutedOutcome::Refused {
            failure: Failure::CrossRange
        }
    ));
    assert!(report.attempts.is_empty());
    assert_eq!(
        servers
            .iter()
            .map(|s| s.state.requests.load(Ordering::SeqCst))
            .sum::<usize>(),
        before
    );
    stop(servers, c).await;
}

#[tokio::test]
async fn routed_wrong_root_or_namespace_cannot_dispatch_any_data() {
    for defect in 0..4 {
        let (servers, config) = fixture().await;
        for server in &servers {
            let mut guard = server.state.route.lock().unwrap();
            let route = guard.as_mut().unwrap();
            match defect {
                0 => route.root_digest = vec![9; 32],
                1 | 2 => {
                    let mut scope = DataRange::decode(&route.binding).unwrap();
                    if defect == 1 {
                        scope.tenant = TenantId(9);
                    } else {
                        scope.keyspace = KeyspaceId(101);
                    }
                    route.binding = scope.encode();
                }
                _ => route.replicas[1] = route.replicas[0].clone(),
            }
        }
        let c = client(config);
        let report = c.call(put(b"forbidden")).await;
        assert!(
            matches!(
                report.outcome,
                RoutedOutcome::LookupFailure {
                    failure: Failure::InvalidRoute
                }
            ),
            "{report:?}"
        );
        assert_eq!(report.stop, Stop::AttemptLimit);
        assert_eq!(report.attempts.len(), 10);
        assert!(report.attempts.iter().all(|a| a.phase == Phase::Lookup));
        assert_eq!(
            servers
                .iter()
                .map(|s| s.state.requests.load(Ordering::SeqCst))
                .sum::<usize>(),
            0
        );
        stop(servers, c).await;
    }
}

#[test]
fn routed_refusals_require_exclusive_well_formed_metadata() {
    for (code, text, duplicate, mixed, expected) in [
        (Code::FailedPrecondition, "scope", false, false, true),
        (Code::FailedPrecondition, "scope", true, false, false),
        (Code::FailedPrecondition, "scope", false, true, false),
        (Code::Unavailable, "scope", false, false, false),
        (Code::FailedPrecondition, "unknown", false, false, false),
    ] {
        let mut status = Status::new(code, "controlled");
        status
            .metadata_mut()
            .insert("kv9-route-refused", text.parse().unwrap());
        if duplicate {
            status
                .metadata_mut()
                .append("kv9-route-refused", "scope".parse().unwrap());
        }
        if mixed {
            status
                .metadata_mut()
                .insert("kv9-not-leader", "true".parse().unwrap());
        }
        let refusal = crate::client::routed::classify(&status, false);
        assert_eq!(refusal == Failure::ScopeRefused, expected);
        if !expected {
            assert_eq!(
                refusal,
                Failure::Rpc {
                    reason: Reason::Protocol
                }
            );
        }
    }
}
