use super::*;
use crate::runtime::checkpoint_retention::origins::ConfigurationOriginObservation;
use kv9_raft::grpc::configuration_origins::OriginTicket;

fn ticket(f: &Fixture) -> OriginTicket {
    let root = &f.backends[1].ttl_context().unwrap().root;
    OriginTicket::new(
        StoreIdentity::for_voter(root, NodeId(3)).unwrap(),
        f.range.region,
        f.range.creation,
    )
    .unwrap()
}
async fn inspect(f: &Fixture, ticket: OriginTicket) -> Result<ConfigurationOriginObservation> {
    let backend = f.backends[1].clone();
    let permit = backend
        .ttl_context()?
        .retention_capacity
        .clone()
        .try_acquire_owned()
        .unwrap();
    tokio::task::spawn_blocking(move || {
        backend.collect_configuration_origins(
            ticket,
            Instant::now() + Duration::from_secs(2),
            permit,
        )
    })
    .await
    .unwrap()
}
pub(super) async fn detach(f: &Fixture, operation: [u8; 16]) {
    let mut t = f.backends[1].node.meta_raft.store.begin().unwrap();
    kv9_meta::data_groups::abort::plan_abort(&mut t, operation, [29; 32]).unwrap();
    let at = f.meta[1]
        .propose(&Command::from_batch(&t.into_batch()))
        .unwrap();
    applied(&f.meta, at).await;
    let backend = f.backends[0].clone();
    let root = f.range.root;
    tokio::task::spawn_blocking(move || backend.detach_aborted_learner("test", root, operation))
        .await
        .unwrap()
        .unwrap();
    recorded(f, Action::Detach).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn origin_query_remote_pages_keep_exact_stores_and_reject_endpoint_changes_and_stale_cursor()
{
    let f = Fixture::new().await;
    let root = f.backends[0].ttl_context().unwrap().root.clone();
    let (identity, guard, creation) = prepare_migration(&f, &root, [120; 16]).await;
    recorded(&f, Action::Attach).await;
    detach(&f, [120; 16]).await;
    for operation in [[121; 16], [122; 16]] {
        let mut t = f.backends[1].node.meta_raft.store.begin().unwrap();
        kv9_meta::data_groups::migration::plan_migration(&mut t, operation, creation, NodeId(4))
            .unwrap();
        let at = f.meta[1]
            .propose(&Command::from_batch(&t.into_batch()))
            .unwrap();
        applied(&f.meta, at).await;
        let backend = f.backends[0].clone();
        let digest = root.digest();
        tokio::task::spawn_blocking(move || {
            backend.attach_migration_learner("test", digest, operation)
        })
        .await
        .unwrap()
        .unwrap();
        recorded(&f, Action::Attach).await;
        detach(&f, operation).await;
    }
    let first = inspect(&f, ticket(&f)).await.unwrap();
    let page = first.page().unwrap();
    assert_eq!(page.total_known, 6);
    assert_eq!(page.entries.len(), 4);
    assert!(page
        .entries
        .iter()
        .all(|e| e.origin.store == identity.store_incarnation));
    let next = first.next_ticket().unwrap().unwrap();
    let last = inspect(&f, next.clone()).await.unwrap();
    assert_eq!(last.page().unwrap().entries.len(), 2);
    assert!(last.next_ticket().unwrap().is_none());
    let before = f.hooks[2].membership_reads.load(Ordering::SeqCst);
    let mut wrong = ticket(&f).identity();
    wrong.store_incarnation = StoreIncarnation::from_bytes([99; 16]);
    assert!(inspect(
        &f,
        OriginTicket::new(wrong, f.range.region, f.range.creation).unwrap()
    )
    .await
    .is_err());
    assert_eq!(f.hooks[2].membership_reads.load(Ordering::SeqCst), before);
    f.hooks[2].membership_unknown.store(true, Ordering::SeqCst);
    let unknown = inspect(&f, ticket(&f)).await.unwrap();
    assert!(unknown.page().is_none());
    assert!(unknown.next_ticket().is_err());
    f.hooks[2].membership_unknown.store(false, Ordering::SeqCst);
    *f.hooks[2].membership_change_endpoint.lock().unwrap() = Some(f.backends[1].clone());
    assert!(inspect(&f, ticket(&f)).await.is_err());
    let at = f.data[0].add_learner(NodeId(5)).unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while f.data[2].peer().status_snapshot().conf_applied < at.index.0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(inspect(&f, next).await.is_err());
    assert!(
        inspect(&f, ticket(&f))
            .await
            .unwrap()
            .page()
            .unwrap()
            .legacy_contexts_unknown
    );
    drop(guard);
    let path = f.shutdown_keep().await;
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn origin_query_rejects_remote_applied_origin_without_independent_catalog_source() {
    let f = Fixture::new().await;
    let root = f.backends[0].ttl_context().unwrap().root.clone();
    let (_, guard, _) = prepare_migration(&f, &root, [123; 16]).await;
    let known = recorded(&f, Action::Attach).await[0];
    let forged = kv9_raft::configuration_origin::ConfigurationOrigin {
        task: 9999,
        node: NodeId(5),
        store: StoreIncarnation::from_bytes([5; 16]),
        source_digest: RootDigest::sha256(b"missing catalog task"),
        ..known.origin
    };
    let at = f.data[0].change_data_member(forged).unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while f.data[2].peer().status_snapshot().conf_applied < at.index.0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        inspect(&f, ticket(&f)).await.is_err(),
        "remote origin requires independent catalog source verification"
    );
    drop(guard);
    let path = f.shutdown_keep().await;
    std::fs::remove_dir_all(path).unwrap();
}
