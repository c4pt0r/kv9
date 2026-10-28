use super::*;
use kv9_raft::grpc::checkpoint_history::{inspect_on_channel, HistoryTicket};
use settlement::{phase, planned, publish, register, Case};

pub(super) async fn committed(f: &Fixture, rivals: bool) -> (RootDescriptor, Vec<Case>) {
    assert!(matches!(
        f.client(0).activate().await.outcome,
        TtlOutcome::Success { .. }
    ));
    let root = f.backends[1].ttl_context().unwrap().root.clone();
    let expected = root.clone();
    let data = f.backends[0]
        .raw_directory
        .by_region(f.range.region)
        .unwrap();
    let engine = data.capture_parts().1.clone();
    let driver = data.capture_parts().0.clone();
    let range = f.range.clone();
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    let cases = tokio::task::spawn_blocking(move || {
        let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()));
        let loser = planned(&engine, &root, range.region);
        let mut cases = Vec::new();
        if rivals {
            cases.push(register(&root, &ledger, loser.manifest(), 0, Some(false)));
            let at = driver
                .propose(&kv9_raft::Command::RawTtl(
                    kv9_raft::command::RawTtlCommand::new(
                        &range,
                        kv9_raft::command::RawTtlAction::Observe {
                            observed: ClockSample::with_uncertainty(2000, 0)
                                .unwrap()
                                .advance(kv9_common::raw_ttl::ClockWatermark::default())
                                .unwrap(),
                        },
                    )
                    .unwrap(),
                ))
                .unwrap();
            driver.wait_applied(at, Duration::from_secs(3)).unwrap();
            cases.push(register(&root, &ledger, loser.manifest(), 1, None));
        }
        let winner = planned(&engine, &root, range.region);
        if rivals {
            assert_ne!(
                winner.manifest().change_id(0).unwrap(),
                loser.manifest().change_id(0).unwrap()
            );
        }
        cases.push(register(&root, &ledger, winner.manifest(), 0, Some(true)));
        publish(&driver, &uploader, winner, 0);
        cases
    })
    .await
    .unwrap();
    until(|| {
        f.data
            .iter()
            .all(|d| d.manifest_pair(f.range.region.0).unwrap().generation == 1)
    })
    .await;
    // Drivers remain alive in the fixture to run real Raft, but the metadata
    // leader's backend has no lookup handle for this group. This is not a
    // disjoint-placement or full NodeRuntime deployment fixture.
    assert!(!f.backends[1]
        .group_handles
        .lock()
        .unwrap()
        .contains_key(&f.range.region));
    f.backends[1].raw_directory.remove(f.range.region);
    (expected, cases)
}

fn protected(f: &Fixture, root: &RootDescriptor, case: &Case) {
    assert_eq!(phase(f, root, case.pending), Some(PinPhase::Published));
    assert_eq!(phase(f, root, case.version), None);
    assert!(matches!(
        observation(f, 1, root, case.pending),
        sweep::SweepObservation::Open
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remote_history_rotates_nodes_only_after_full_sweep_and_preserves_unknown_windows() {
    let f = Fixture::new().await;
    let (root, cases) = committed(&f, true).await;
    // The first candidate is an admitted node without this group handle.
    assert!(!f.backends[0]
        .group_handles
        .lock()
        .unwrap()
        .contains_key(&f.range.region));
    f.backends[0].raw_directory.remove(f.range.region);
    let mut state = Reconciler::default();
    for _ in &cases {
        let stats = pass(&mut state, &f, 1).await.unwrap();
        assert_eq!(
            (
                stats.remote_queries,
                stats.remote_errors,
                stats.history_unknown
            ),
            (1, 0, 1)
        );
        assert_eq!((stats.positive_handoffs, stats.negative_closures), (0, 0));
    }
    assert_eq!(f.hooks[0].history_reads.load(Ordering::SeqCst), cases.len());
    assert_eq!(f.hooks[2].history_reads.load(Ordering::SeqCst), 0);
    for case in &cases {
        protected(&f, &root, case);
    }
    assert_eq!(pass(&mut state, &f, 1).await.unwrap().wraps, 1);
    assert_eq!(f.data[2].status().role, Role::Follower);
    let (mut positives, mut negatives, mut unknowns) = (0, 0, 0);
    for _ in &cases {
        let stats = pass(&mut state, &f, 1).await.unwrap();
        assert_eq!((stats.remote_queries, stats.remote_errors), (1, 0));
        positives += stats.positive_handoffs;
        negatives += stats.negative_closures;
        unknowns += stats.history_unknown;
    }
    assert_eq!((positives, negatives, unknowns), (1, 1, 1));
    // The negative is independently re-read by the receiving metadata leader.
    assert_eq!(
        f.hooks[2].history_reads.load(Ordering::SeqCst),
        cases.len() + 1
    );
    for case in &cases {
        match case.positive {
            Some(true) => {
                assert_eq!(phase(&f, &root, case.pending), Some(PinPhase::Released));
                assert_eq!(phase(&f, &root, case.version), Some(PinPhase::Published));
            }
            Some(false) => {
                assert_eq!(phase(&f, &root, case.pending), Some(PinPhase::Published));
                assert_eq!(phase(&f, &root, case.version), None);
                assert!(!matches!(
                    observation(&f, 1, &root, case.pending),
                    sweep::SweepObservation::Open
                ));
            }
            None => protected(&f, &root, case),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remote_history_refusal_preserves_admission_and_does_not_starve_next_candidate() {
    let f = Fixture::new().await;
    let (root, cases) = committed(&f, false).await;
    f.hooks[0].history_refuse.store(true, Ordering::SeqCst);
    let mut state = Reconciler::default();
    let stats = pass(&mut state, &f, 1).await.unwrap();
    assert_eq!(
        (
            stats.remote_queries,
            stats.remote_errors,
            stats.history_unknown
        ),
        (1, 1, 1)
    );
    protected(&f, &root, &cases[0]);
    assert_eq!(pass(&mut state, &f, 1).await.unwrap().wraps, 1);
    let stats = pass(&mut state, &f, 1).await.unwrap();
    assert_eq!(
        (
            stats.remote_queries,
            stats.remote_errors,
            stats.positive_handoffs
        ),
        (1, 0, 1)
    );
    assert_eq!(phase(&f, &root, cases[0].pending), Some(PinPhase::Released));
    assert_eq!(f.hooks[0].history_reads.load(Ordering::SeqCst), 1);
    assert_eq!(f.hooks[2].history_reads.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remote_history_rechecks_endpoint_generation_before_any_owner_mutation() {
    let f = Fixture::new().await;
    let (root, cases) = committed(&f, false).await;
    let before = node_endpoint(
        &f.backends[1].node.meta_raft.store.begin().unwrap(),
        NodeId(1),
    )
    .unwrap()
    .unwrap();
    *f.hooks[0].history_change_endpoint.lock().unwrap() = Some(f.backends[1].clone());
    let stats = pass(&mut Reconciler::default(), &f, 1).await.unwrap();
    assert_eq!(
        (
            stats.remote_queries,
            stats.remote_errors,
            stats.history_unknown
        ),
        (1, 1, 1)
    );
    protected(&f, &root, &cases[0]);
    let after = node_endpoint(
        &f.backends[1].node.meta_raft.store.begin().unwrap(),
        NodeId(1),
    )
    .unwrap()
    .unwrap();
    assert_eq!(before.address, after.address);
    assert!(after.generation > before.generation);
    let stats = pass(&mut Reconciler::default(), &f, 1).await.unwrap();
    assert_eq!(
        (
            stats.remote_queries,
            stats.remote_errors,
            stats.positive_handoffs
        ),
        (1, 0, 1)
    );
    assert_eq!(phase(&f, &root, cases[0].pending), Some(PinPhase::Released));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remote_history_http2_requires_auth_and_exact_store_group_window() {
    let f = Fixture::new().await;
    let (root, _) = committed(&f, false).await;
    let identity = StoreIdentity::for_voter(&root, NodeId(1)).unwrap();
    let ticket = HistoryTicket::new(identity, f.range.region, 0).unwrap();
    for (caller, token) in [
        (NodeId(2), None),
        (NodeId(2), Some("wrong".into())),
        (NodeId(99), Some("cluster-test".into())),
    ] {
        assert!(inspect_on_channel(
            f.public[0].clone(),
            &ticket,
            caller,
            token,
            Duration::from_secs(1)
        )
        .await
        .is_err());
    }
    assert_eq!(f.hooks[0].history_reads.load(Ordering::SeqCst), 0);
    let reply = inspect_on_channel(
        f.public[0].clone(),
        &ticket,
        NodeId(2),
        Some("cluster-test".into()),
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    let expected = f.data[0]
        .manifest_pair(f.range.region.0)
        .unwrap()
        .last_change_id;
    assert_eq!(reply.winner_change_id().unwrap().as_slice(), expected);
    for (region, predecessor) in [(f.range.region, 1), (RegionId(f.range.region.0 + 100), 0)] {
        let ticket = HistoryTicket::new(identity, region, predecessor).unwrap();
        assert_eq!(
            inspect_on_channel(
                f.public[0].clone(),
                &ticket,
                NodeId(2),
                Some("cluster-test".into()),
                Duration::from_secs(1)
            )
            .await
            .unwrap()
            .winner_change_id(),
            None
        );
    }
    let wrong = StoreIdentity {
        store_incarnation: StoreIncarnation::mint().unwrap(),
        ..identity
    };
    let ticket = HistoryTicket::new(wrong, f.range.region, 0).unwrap();
    assert!(inspect_on_channel(
        f.public[0].clone(),
        &ticket,
        NodeId(2),
        Some("cluster-test".into()),
        Duration::from_secs(1)
    )
    .await
    .is_err());
    assert_eq!(f.hooks[0].history_reads.load(Ordering::SeqCst), 3);
}
