use super::*;
use std::sync::{atomic::AtomicUsize, Condvar};
fn root() -> RootDigest {
    RootDigest::from_bytes([1; 32])
}
fn owner() -> OwnerId {
    OwnerId::new([2; 16]).unwrap()
}
fn applied() -> LedgerReply {
    LedgerReply::Applied {
        revision: 1,
        changed: true,
        applied: AppliedPosition { term: 3, index: 9 },
    }
}

#[tokio::test]
async fn capture_plan_query_is_correlated_and_does_not_expand_apply_bounds() {
    let b = Arc::new(Backend::default());
    let service = service(b.clone());
    let bytes = vec![1; MAX_CAPTURE_BYTES];
    let query = LedgerQuery::CapturePlan(bytes.clone());
    let mut req = request();
    req.get_mut().query = Some(query.encode());
    let reply = service
        .checkpoint_retention(req)
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        LedgerReply::decode(reply.clone(), root(), &query).unwrap(),
        applied()
    );
    assert!(LedgerReply::decode(reply.clone(), root(), &LedgerQuery::Apply(bytes)).is_err());
    assert!(LedgerReply::decode(reply, root(), &LedgerQuery::CapturePlan(vec![2])).is_err());
    for query in [
        LedgerQuery::CapturePlan(vec![]),
        LedgerQuery::CapturePlan(vec![1; MAX_CAPTURE_BYTES + 1]),
        LedgerQuery::Apply(vec![1; MAX_COMMAND_BYTES + 1]),
    ] {
        let mut req = request();
        req.get_mut().query = Some(query.encode());
        assert!(service.checkpoint_retention(req).await.is_err());
    }
    assert_eq!(b.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn executor_queries_bind_pending_cursor_limit_and_reply_family() {
    let b = Arc::new(Backend::default());
    let service = service(b.clone());
    let query = LedgerQuery::Executors {
        pending: owner(),
        after: None,
        limit: 4,
    };
    let mut req = request();
    req.get_mut().query = Some(query.encode());
    let reply = service
        .checkpoint_retention_request(req)
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        LedgerReply::decode(reply.clone(), root(), &query).unwrap(),
        LedgerReply::Executors(vec![1])
    );
    for other in [
        LedgerQuery::Executors {
            pending: OwnerId::new([3; 16]).unwrap(),
            after: None,
            limit: 4,
        },
        LedgerQuery::Executors {
            pending: owner(),
            after: Some(owner()),
            limit: 4,
        },
        LedgerQuery::Executors {
            pending: owner(),
            after: None,
            limit: 3,
        },
        LedgerQuery::Intent(owner()),
    ] {
        assert!(LedgerReply::decode(reply.clone(), root(), &other).is_err());
    }
    assert!(LedgerReply::decode(reply.clone(), RootDigest::from_bytes([9; 32]), &query).is_err());
    for observation in [
        None,
        Some(vec![]),
        Some(vec![1; MAX_EXECUTOR_PAGE_BYTES + 1]),
    ] {
        let mut bad = reply.clone();
        bad.reply = Some(pb::checkpoint_retention_response::Reply::Executors(
            pb::CheckpointRetentionOwner { observation },
        ));
        assert!(LedgerReply::decode(bad, root(), &query).is_err());
    }
    for (pending, after, limit) in [
        (vec![0; 16], None, 1),
        (owner().as_bytes().to_vec(), Some(vec![]), 1),
        (owner().as_bytes().to_vec(), None, 0),
        (owner().as_bytes().to_vec(), None, 17),
    ] {
        let mut req = request();
        req.get_mut().query = Some(pb::checkpoint_retention_request::Query::Executors(
            pb::CheckpointExecutorsQuery {
                pending_owner_id: pending,
                after_owner_id: after,
                limit,
            },
        ));
        assert!(service.checkpoint_retention_request(req).await.is_err());
    }
    assert_eq!(b.calls.load(Ordering::SeqCst), 1);
}
fn request() -> Request<pb::CheckpointRetentionRequest> {
    let mut r = Request::new(pb::CheckpointRetentionRequest {
        root_digest: root().as_bytes().to_vec(),
        budget_micros: 5_000_000,
        query: Some(LedgerQuery::Apply(vec![1]).encode()),
    });
    r.extensions_mut().insert(NodeId(1));
    r
}
struct Discovery(bool);
impl GrpcDiscoveryState for Discovery {
    fn answer(&self) -> (NodeId, bool, u64) {
        (NodeId(2), true, 0)
    }
    fn raft_receive_allowed(&self) -> bool {
        self.0
    }
    fn root_identity(&self) -> RootWireIdentity {
        RootWireIdentity {
            root_digest: root(),
            bootstrap_generation: BootstrapGeneration::from_bytes([3; 16]),
        }
    }
}
#[derive(Default)]
struct Backend {
    calls: AtomicUsize,
    blocked: Mutex<bool>,
    wake: Condvar,
}
impl RegistrationBackend for Backend {
    fn checkpoint_retention(
        &self,
        _: RootDigest,
        query: &LedgerQuery,
        _: Instant,
    ) -> kv9_common::Result<LedgerReply> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut blocked = self.blocked.lock().unwrap();
        while *blocked {
            blocked = self.wake.wait(blocked).unwrap();
        }
        Ok(match query {
            LedgerQuery::Apply(_) | LedgerQuery::CapturePlan(_) => applied(),
            LedgerQuery::Owner(_) => LedgerReply::Owner(None),
            LedgerQuery::Intent(_) => LedgerReply::Intent(None),
            LedgerQuery::Executors { .. } => LedgerReply::Executors(vec![1]),
        })
    }
    fn register(
        &self,
        _: NodeId,
        _: &str,
        _: ClusterId,
        _: &[u8],
        _: StoreIncarnation,
    ) -> std::result::Result<RegistrationReceipt, RegistrationError> {
        unreachable!()
    }
}
fn service(b: Arc<Backend>) -> Arc<RaftGrpcService> {
    Arc::new(
        RaftGrpcService::new(
            NodeId(2),
            crate::work::RaftInbox::default(),
            Arc::new(Discovery(true)),
        )
        .with_registration(b),
    )
}
#[tokio::test]
async fn checkpoint_rpc_refuses_unauthenticated_wrong_root_and_invalid_bounds_before_backend() {
    let b = Arc::new(Backend::default());
    let s = service(b.clone());
    use pb::checkpoint_retention_request::Query;
    for mode in 0..10 {
        let mut r = request();
        match mode {
            0 => {
                r.extensions_mut().remove::<NodeId>();
            }
            1 => {
                r.extensions_mut().insert(NodeId(0));
            }
            2 => r.get_mut().root_digest[0] ^= 1,
            3 => r.get_mut().root_digest.pop().map(|_| ()).unwrap(),
            4 => r.get_mut().budget_micros = 0,
            5 => r.get_mut().budget_micros = MAX_BUDGET_MICROS + 1,
            6 => r.get_mut().query = None,
            7 => r.get_mut().query = Some(Query::Apply(vec![])),
            8 => r.get_mut().query = Some(Query::Apply(vec![1; MAX_COMMAND_BYTES + 1])),
            9 => r.get_mut().query = Some(Query::OwnerId(vec![0; 16])),
            _ => unreachable!(),
        }
        assert!(s.checkpoint_retention(r).await.is_err(), "mode {mode}");
    }
    assert_eq!(b.calls.load(Ordering::SeqCst), 0);
    let denied = RaftGrpcService::new(
        NodeId(2),
        crate::work::RaftInbox::default(),
        Arc::new(Discovery(false)),
    )
    .with_registration(b.clone());
    assert!(denied.checkpoint_retention(request()).await.is_err());
    assert_eq!(b.calls.load(Ordering::SeqCst), 0);
    let reply = s
        .checkpoint_retention(request())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        LedgerReply::decode(reply, root(), &LedgerQuery::Apply(vec![1])).unwrap(),
        applied()
    );
    let mut r = request();
    r.get_mut().query = Some(LedgerQuery::Owner(owner()).encode());
    assert_eq!(
        LedgerReply::decode(
            s.checkpoint_retention(r).await.unwrap().into_inner(),
            root(),
            &LedgerQuery::Owner(owner())
        )
        .unwrap(),
        LedgerReply::Owner(None)
    );
}

#[tokio::test]
async fn intent_query_is_distinct_correlated_and_individually_bounded() {
    let s = service(Arc::new(Backend::default()));
    let query = LedgerQuery::Intent(owner());
    let mut r = request();
    r.get_mut().query = Some(query.encode());
    let response = s.checkpoint_retention(r).await.unwrap().into_inner();
    assert_eq!(
        LedgerReply::decode(response.clone(), root(), &query).unwrap(),
        LedgerReply::Intent(None)
    );
    assert!(LedgerReply::decode(response, root(), &LedgerQuery::Owner(owner())).is_err());
    for bytes in [vec![], vec![1; MAX_INTENT_BYTES + 1]] {
        let response = LedgerReply::Intent(Some(bytes)).encode(root(), &query);
        assert!(LedgerReply::decode(response, root(), &query).is_err());
    }
    let response = LedgerReply::Intent(Some(vec![1; MAX_INTENT_BYTES])).encode(root(), &query);
    assert!(LedgerReply::decode(response.clone(), root(), &query).is_ok());
    assert!(LedgerReply::decode(
        response,
        root(),
        &LedgerQuery::Intent(OwnerId::new([7; 16]).unwrap())
    )
    .is_err());
    let mut r = request();
    r.get_mut().query = Some(pb::checkpoint_retention_request::Query::IntentOwnerId(
        vec![0; 16],
    ));
    assert!(s.checkpoint_retention(r).await.is_err());
}
#[test]
fn checkpoint_receipt_correlates_root_command_kind_owner_and_exact_position() {
    let q = LedgerQuery::Apply(vec![1]);
    let good = applied().encode(root(), &q);
    for mode in 0..7 {
        let mut bad = good.clone();
        match mode {
            0 => bad.root_digest[0] ^= 1,
            1 => bad.query_digest[0] ^= 1,
            2 => bad.reply = None,
            3 => {
                bad.reply = Some(pb::checkpoint_retention_response::Reply::Owner(
                    pb::CheckpointRetentionOwner { observation: None },
                ))
            }
            4 | 5 => {
                let Some(pb::checkpoint_retention_response::Reply::Applied(at)) = &mut bad.reply
                else {
                    unreachable!()
                };
                if mode == 4 {
                    at.term = 0;
                } else {
                    at.index = 0;
                }
            }
            6 => {
                bad.query_digest = LedgerQuery::Apply(vec![2])
                    .digest(root())
                    .as_bytes()
                    .to_vec()
            }
            _ => unreachable!(),
        }
        assert!(LedgerReply::decode(bad, root(), &q).is_err());
    }
    let read = LedgerQuery::Owner(owner());
    for bytes in [Some(vec![]), Some(vec![1; MAX_OWNER_BYTES + 1])] {
        assert!(LedgerReply::decode(
            LedgerReply::Owner(bytes).encode(root(), &read),
            root(),
            &read
        )
        .is_err());
    }
    let absent = LedgerReply::Owner(None).encode(root(), &read);
    assert!(LedgerReply::decode(
        absent,
        root(),
        &LedgerQuery::Owner(OwnerId::new([4; 16]).unwrap())
    )
    .is_err());
    let bounded = LedgerReply::Owner(Some(vec![1; MAX_OWNER_BYTES])).encode(root(), &read);
    assert!(LedgerReply::decode(bounded, root(), &read).is_ok());
}
#[test]
fn only_typed_not_leader_is_reroutable() {
    assert!(matches!(
        decode_status(status(Error::NotLeader {
            leader: Some(NodeId(2))
        })),
        Error::NotLeader {
            leader: Some(NodeId(2))
        }
    ));
    for mut s in [
        Status::unavailable("not leader"),
        Status::failed_precondition("not leader"),
        Status::deadline_exceeded("not leader"),
    ] {
        assert!(!matches!(decode_status(s.clone()), Error::NotLeader { .. }));
        s.metadata_mut()
            .insert(NOT_LEADER_KEY, "true".parse().unwrap());
        s.metadata_mut()
            .insert(LEADER_NODE_ID_KEY, "0".parse().unwrap());
        assert!(!matches!(decode_status(s), Error::NotLeader { .. }));
    }
}
struct Release(Arc<Backend>);
impl Drop for Release {
    fn drop(&mut self) {
        *self.0.blocked.lock().unwrap() = false;
        self.0.wake.notify_all();
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checkpoint_cancelled_requests_hold_capacity_until_actual_blocking_jobs_exit() {
    let b = Arc::new(Backend::default());
    *b.blocked.lock().unwrap() = true;
    let release = Release(b.clone());
    let s = service(b.clone());
    let mut tasks = Vec::new();
    for _ in 0..2 {
        let s = s.clone();
        tasks.push(tokio::spawn(async move {
            s.checkpoint_retention(request()).await
        }));
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        while b.calls.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    for task in tasks {
        task.abort();
        let _ = task.await;
    }
    assert_eq!(
        s.checkpoint_retention(request()).await.unwrap_err().code(),
        tonic::Code::ResourceExhausted
    );
    assert_eq!(b.calls.load(Ordering::SeqCst), 2);
    drop(release);
    tokio::time::timeout(Duration::from_secs(3), async {
        while s.checkpoint_retention_capacity.available_permits() != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(s.checkpoint_retention(request()).await.is_ok());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn backend_completion_after_budget_is_not_reported_as_success() {
    let b = Arc::new(Backend::default());
    *b.blocked.lock().unwrap() = true;
    let release = Release(b.clone());
    let s = service(b.clone());
    let call = tokio::spawn(async move {
        let mut r = request();
        r.get_mut().budget_micros = 50_000;
        s.checkpoint_retention(r).await
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while b.calls.load(Ordering::SeqCst) < 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(75)).await;
    drop(release);
    assert_eq!(
        call.await.unwrap().unwrap_err().code(),
        tonic::Code::Unavailable
    );
}
