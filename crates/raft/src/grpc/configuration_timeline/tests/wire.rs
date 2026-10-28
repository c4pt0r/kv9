use super::*;
use crate::grpc::data_membership::tests::old_service::{connect, serve_old_origin_fixture};

#[tokio::test]
async fn timeline_query_old_v11_refuses_without_fallback_and_http2_pages_are_authenticated() {
    let f = Fixture::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let (old, old_job) = serve_old_origin_fixture(f.ticket.identity(), calls.clone()).await;
    crate::grpc::ttl_format::confirm_on_channel(
        old.clone(),
        f.ticket.identity(),
        NodeId(1),
        Some("upgrade-test".into()),
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    let before = calls.load(Ordering::SeqCst);
    assert!(inspect_on_channel(
        old,
        &f.ticket,
        NodeId(1),
        Some("upgrade-test".into()),
        Duration::from_secs(1)
    )
    .await
    .is_err());
    assert_eq!(
        before,
        calls.load(Ordering::SeqCst),
        "no fallback to an old method"
    );
    let (channel, job) = connect(tonic::transport::Server::builder().add_service(
        pb::kv9_raft_server::Kv9RaftServer::with_interceptor(
            f.service(),
            cluster_token_interceptor("upgrade-test".into()),
        ),
    ))
    .await;
    assert!(inspect_on_channel(
        channel.clone(),
        &f.ticket,
        NodeId(1),
        Some("wrong".into()),
        Duration::from_secs(1)
    )
    .await
    .is_err());
    assert_eq!(f.backend.calls.load(Ordering::SeqCst), 0);
    let first = inspect_on_channel(
        channel.clone(),
        &f.ticket,
        NodeId(1),
        Some("upgrade-test".into()),
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    assert_eq!(first.page().unwrap().records.len(), 4);
    let last = inspect_on_channel(
        channel,
        &first.next_ticket().unwrap().unwrap(),
        NodeId(1),
        Some("upgrade-test".into()),
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    assert_eq!(
        last.page()
            .unwrap()
            .records
            .last()
            .unwrap()
            .origin
            .unwrap()
            .origin
            .node,
        NodeId(10)
    );
    assert!(last.next_ticket().unwrap().is_none());
    for task in [old_job, job] {
        task.abort();
        let _ = task.await;
    }
}
