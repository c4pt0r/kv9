use super::*;
use kv9_raft::grpc::data_membership::MembershipTicket;
use kv9_raft::storage::membership_history::{HistoricalMember, SEEN_LEARNER, SEEN_VOTER};

fn ticket(f: &Fixture, source: usize) -> MembershipTicket {
    let root = &f.backends[1].ttl_context().unwrap().root;
    MembershipTicket::new(
        StoreIdentity::for_voter(root, NodeId(source as u64 + 1)).unwrap(),
        f.range.region,
        f.range.creation,
    )
    .unwrap()
}
async fn inspect(f: &Fixture, ticket: MembershipTicket) -> Result<DataMembershipObservation> {
    let backend = f.backends[1].clone();
    let permit = backend
        .ttl_context()?
        .retention_capacity
        .clone()
        .try_acquire_owned()
        .unwrap();
    tokio::task::spawn_blocking(move || {
        backend.collect_data_membership(ticket, Instant::now() + Duration::from_secs(2), permit)
    })
    .await
    .unwrap()
}
async fn removed_learner(f: &Fixture, node: u64) {
    let at = f.data[0].add_learner(NodeId(node)).unwrap();
    until(|| {
        f.data.iter().all(|d| {
            d.peer().status_snapshot().conf_applied >= at.index.0
                && d.status().learners.contains(&node)
        })
    })
    .await;
    let at = f.data[0].remove_voter(NodeId(node)).unwrap();
    until(|| {
        f.data.iter().all(|d| {
            d.peer().status_snapshot().conf_applied >= at.index.0
                && !d.status().learners.contains(&node)
        })
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn independent_membership_pages_keep_removed_nodes_and_reject_changed_history() {
    let f = Fixture::new().await;
    for node in 4..=6 {
        removed_learner(&f, node).await;
    }
    // Metadata leader 2 inspects data follower 3, not its own local inventory.
    let first = inspect(&f, ticket(&f, 2)).await.unwrap();
    assert_eq!(first.identity(), ticket(&f, 2).identity());
    assert_eq!(first.endpoint().node, NodeId(3));
    assert_eq!(first.page().unwrap().known_nodes, 6);
    assert!(!first.page().unwrap().gaps.any());
    assert_eq!(first.page().unwrap().members.len(), 4);
    let next = first.next_ticket().unwrap().unwrap();
    let last = inspect(&f, next.clone()).await.unwrap();
    assert_eq!(
        last.page().unwrap().members,
        vec![
            HistoricalMember {
                node: NodeId(5),
                roles: SEEN_LEARNER
            },
            HistoricalMember {
                node: NodeId(6),
                roles: SEEN_LEARNER
            }
        ]
    );
    assert!(last.next_ticket().unwrap().is_none());
    removed_learner(&f, 7).await;
    assert!(inspect(&f, next).await.is_err());
    let first = inspect(&f, ticket(&f, 2)).await.unwrap();
    let last = inspect(&f, first.next_ticket().unwrap().unwrap())
        .await
        .unwrap();
    assert_eq!(last.page().unwrap().members.last().unwrap().node, NodeId(7));
    assert_eq!(f.hooks[0].membership_reads.load(Ordering::SeqCst), 0);
    assert_eq!(f.hooks[1].membership_reads.load(Ordering::SeqCst), 0);
    assert!(f.hooks[2].membership_reads.load(Ordering::SeqCst) >= 5);
    let expected = f.data[2].peer().membership_history_page(None).unwrap();
    let path = f.shutdown_keep().await;
    for node in 1..=3 {
        let storage = DiskRaftStorage::recover(&path.join(format!("n{node}/data-raft"))).unwrap();
        assert_eq!(storage.membership_history_page(None).unwrap(), expected);
    }
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn independent_membership_requires_exact_source_and_rechecks_endpoint_then_keeps_unknown() {
    let f = Fixture::new().await;
    let original = ticket(&f, 2);
    let before = f.hooks[2].membership_reads.load(Ordering::SeqCst);
    let mut wrong = original.identity();
    wrong.store_incarnation = StoreIncarnation::mint().unwrap();
    assert!(inspect(
        &f,
        MembershipTicket::new(wrong, f.range.region, f.range.creation).unwrap()
    )
    .await
    .is_err());
    assert_eq!(f.hooks[2].membership_reads.load(Ordering::SeqCst), before);
    assert!(inspect(
        &f,
        MembershipTicket::new(
            original.identity(),
            f.range.region,
            RootDigest::from_bytes([99; 32])
        )
        .unwrap()
    )
    .await
    .is_err());
    f.hooks[2].membership_refuse.store(true, Ordering::SeqCst);
    assert!(inspect(&f, original.clone()).await.is_err());
    f.hooks[2].membership_refuse.store(false, Ordering::SeqCst);
    *f.hooks[2].membership_change_endpoint.lock().unwrap() = Some(f.backends[1].clone());
    assert!(inspect(&f, original.clone()).await.is_err());
    let current = inspect(&f, original.clone()).await.unwrap();
    assert_eq!(current.endpoint().generation, 1);
    assert_eq!(current.page().unwrap().known_nodes, 3);
    f.hooks[2].membership_unknown.store(true, Ordering::SeqCst);
    let unknown = inspect(&f, original.clone()).await.unwrap();
    assert!(unknown.page().is_none());
    assert!(unknown.next_ticket().is_err());
    f.hooks[2].membership_unknown.store(false, Ordering::SeqCst);
    f.backends[2].raw_directory.remove(f.range.region);
    f.backends[2]
        .group_handles
        .lock()
        .unwrap()
        .remove(&f.range.region);
    let missing = inspect(&f, original).await.unwrap();
    assert!(
        missing.page().is_none(),
        "missing live owner must not become an empty history or reopen a live log"
    );
    assert!(missing.next_ticket().is_err());
    // Read-only observations create no retention authority.
    assert!(f
        .hooks
        .iter()
        .all(|h| h.retention_commits.load(Ordering::SeqCst) == 0));
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_membership_inventory_keeps_removed_learner_after_three_replica_restart() {
    let f = Fixture::new().await;
    let at = f.data[0].add_learner(NodeId(4)).unwrap();
    until(|| {
        f.data.iter().all(|d| {
            d.peer().status_snapshot().conf_applied >= at.index.0
                && d.status().learners.contains(&4)
        })
    })
    .await;
    let at = f.data[0].remove_voter(NodeId(4)).unwrap();
    until(|| {
        f.data.iter().all(|d| {
            d.peer().status_snapshot().conf_applied >= at.index.0
                && !d.status().learners.contains(&4)
        })
    })
    .await;
    let expected: Vec<_> = f
        .data
        .iter()
        .map(|driver| {
            let page = driver.peer().membership_history_page(None).unwrap();
            assert!(!page.gaps.any());
            assert!(page.next.is_none());
            assert_eq!(page.last_configuration_index, at.index.0);
            assert_eq!(
                page.members,
                vec![
                    HistoricalMember {
                        node: NodeId(1),
                        roles: SEEN_VOTER
                    },
                    HistoricalMember {
                        node: NodeId(2),
                        roles: SEEN_VOTER
                    },
                    HistoricalMember {
                        node: NodeId(3),
                        roles: SEEN_VOTER
                    },
                    HistoricalMember {
                        node: NodeId(4),
                        roles: SEEN_LEARNER
                    },
                ]
            );
            page
        })
        .collect();
    let path = f.shutdown_keep().await;
    for (i, expected) in expected.into_iter().enumerate() {
        let recovered =
            DiskRaftStorage::recover(&path.join(format!("n{}/data-raft", i + 1))).unwrap();
        assert_eq!(recovered.membership_history_page(None).unwrap(), expected);
    }
    std::fs::remove_dir_all(path).unwrap();
}
