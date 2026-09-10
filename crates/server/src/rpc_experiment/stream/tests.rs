//! Real HTTP/2 stream tests with controlled peers and the shared instrumented backend.
//! These establish adapter ownership/correlation, not Raft or fault-matrix acceptance.

use super::*;
use crate::admission::PublicApiLimits;
use crate::client::{ClientConfig, Outcome, Peer, PersistentRawClient, RawOperation};
use crate::grpc::NOT_LEADER_KEY;
use crate::rpc_experiment::tests::{
    authenticator, context_message, deadline, get, put, request, Backend, WriteGate, APPLIED,
};
use crate::rpc_experiment::TransportKind;
use tokio_stream::wrappers::ReceiverStream;
use tonic::Code;

type WireClient = wire::point_stream_client::PointStreamClient<tonic::transport::Channel>;

async fn bounded<F: Future>(future: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(5), future)
        .await
        .expect("controlled transport step must finish")
}

async fn eventually(mut ready: impl FnMut() -> bool) {
    bounded(async {
        while !ready() {
            tokio::task::yield_now().await;
        }
    })
    .await;
}

async fn server(api: Kv9Grpc) -> (StreamServer, Arc<StreamClient>) {
    let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = probe.local_addr().unwrap();
    drop(probe);
    let server = start(address, api, authenticator()).await.unwrap();
    (server, Arc::new(StreamClient::new(address, 16)))
}

async fn stop(mut server: StreamServer) {
    server.shutdown.cancel();
    server.task.abort();
    let _ = bounded(&mut server.task).await;
}

async fn wire_client(address: SocketAddr) -> WireClient {
    let channel = Endpoint::from_shared(format!("http://{address}"))
        .unwrap()
        .tcp_nodelay(true)
        .connect()
        .await
        .unwrap();
    WireClient::new(channel)
        .max_decoding_message_size(FRAME_LIMIT)
        .max_encoding_message_size(FRAME_LIMIT)
}

fn frame(id: u64, operation: u32, payload: Vec<u8>) -> wire::PointRequest {
    wire::PointRequest {
        id,
        operation,
        remaining_micros: 4_000_000,
        authorization: "Bearer secret".into(),
        payload,
    }
}

fn get_reply(id: u64, value: &[u8]) -> wire::PointResponse {
    wire::PointResponse {
        id,
        reply: Some(
            WireReply::encode(Ok(Response::new(proto::RawGetResponse {
                value: Some(proto::OptionalValue {
                    found: true,
                    value: value.to_vec(),
                }),
            })))
            .into(),
        ),
    }
}

fn write_reply(id: u64) -> wire::PointResponse {
    wire::PointResponse {
        id,
        reply: Some(
            WireReply::encode(Ok(Response::new(proto::RawWriteResponse {
                applied_term: APPLIED.term,
                applied_index: APPLIED.index,
            })))
            .into(),
        ),
    }
}

struct Session {
    incoming: tonic::Streaming<wire::PointRequest>,
    replies: mpsc::Sender<Result<wire::PointResponse, Status>>,
}

#[derive(Clone)]
struct ControlledPeer(mpsc::UnboundedSender<Session>);

#[tonic::async_trait]
impl wire::point_stream_server::PointStream for ControlledPeer {
    type ExchangeStream = ReceiverStream<Result<wire::PointResponse, Status>>;

    async fn exchange(
        &self,
        request: Request<tonic::Streaming<wire::PointRequest>>,
    ) -> Result<Response<Self::ExchangeStream>, Status> {
        let (replies, receiver) = mpsc::channel(16);
        self.0
            .send(Session {
                incoming: request.into_inner(),
                replies,
            })
            .map_err(|_| Status::unavailable("test controller closed"))?;
        Ok(Response::new(ReceiverStream::new(receiver)))
    }
}

struct PeerServer {
    address: SocketAddr,
    sessions: mpsc::UnboundedReceiver<Session>,
    task: JoinHandle<()>,
}

impl Drop for PeerServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn controlled_peer() -> PeerServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (sessions, receiver) = mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        Server::builder()
            .add_service(wire::point_stream_server::PointStreamServer::new(
                ControlledPeer(sessions),
            ))
            .serve_with_incoming(TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    PeerServer {
        address,
        sessions: receiver,
        task,
    }
}

fn public_client(address: SocketAddr, deadline_ms: u64) -> PersistentRawClient {
    PersistentRawClient::new_with_transport(
        ClientConfig {
            version: 1,
            peers: vec![Peer {
                node_id: 1,
                address,
            }],
            keyspace_id: 7,
            epoch_conf_ver: 11,
            epoch_version: 13,
            max_in_flight: 16,
            max_attempts: 6,
            deadline_ms,
            retry_backoff_ms: 1,
        },
        "secret",
        TransportKind::TonicStream,
    )
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_stream_point_operations_and_per_frame_auth_share_handler_contracts() {
    let backend = Arc::new(Backend::default());
    let (server, client) = server(Kv9Grpc::new(backend.clone())).await;
    let at = bounded(client.raw_put(request(put(b"key", b"value")), deadline()))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        (at.applied_term, at.applied_index),
        (APPLIED.term, APPLIED.index)
    );
    let value = bounded(client.raw_get(request(get(b"key")), deadline()))
        .await
        .unwrap()
        .into_inner()
        .value
        .unwrap();
    assert!(value.found);
    assert_eq!(value.value, b"value");
    let bad = {
        let mut req = request(put(b"private", b"denied"));
        req.metadata_mut()
            .insert("authorization", "Bearer wrong".parse().unwrap());
        bounded(client.raw_put(req, deadline())).await.unwrap_err()
    };
    assert_eq!(bad.code(), Code::Unauthenticated);
    assert_eq!(backend.calls.lock().unwrap().len(), 2);
    let refused = bounded(client.raw_get(request(get(b"not-leader")), deadline()))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition);
    assert_eq!(refused.metadata().get(NOT_LEADER_KEY).unwrap(), "true");
    bounded(client.raw_delete(
        request(proto::RawDeleteRequest {
            context: Some(context_message()),
            key: b"key".to_vec(),
        }),
        deadline(),
    ))
    .await
    .unwrap();
    let missing = bounded(client.raw_get(request(get(b"key")), deadline()))
        .await
        .unwrap()
        .into_inner()
        .value
        .unwrap();
    assert!(!missing.found && missing.value.is_empty());
    {
        let calls = backend.calls.lock().unwrap();
        assert_eq!(calls.len(), 5);
        assert!(calls.iter().all(|ctx| ctx.keyspace.0 == 7
            && ctx.region_epoch.conf_ver == 11
            && ctx.region_epoch.version == 13));
    }
    stop(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn out_of_order_replies_correlate_to_exact_requests_on_one_generation() {
    let mut peer = controlled_peer().await;
    let client = Arc::new(StreamClient::new(peer.address, 16));
    let a = client.clone();
    let left = tokio::spawn(async move { a.raw_get(request(get(b"left")), deadline()).await });
    let mut session = bounded(peer.sessions.recv()).await.unwrap();
    let first = bounded(session.incoming.message()).await.unwrap().unwrap();
    let b = client.clone();
    let right = tokio::spawn(async move { b.raw_get(request(get(b"right")), deadline()).await });
    let second = bounded(session.incoming.message()).await.unwrap().unwrap();
    assert!(first.id > 0 && second.id > first.id);
    assert_eq!(
        proto::RawGetRequest::decode(first.payload.as_slice())
            .unwrap()
            .key,
        b"left"
    );
    assert_eq!(
        proto::RawGetRequest::decode(second.payload.as_slice())
            .unwrap()
            .key,
        b"right"
    );
    session
        .replies
        .send(Ok(get_reply(second.id, b"right-result")))
        .await
        .unwrap();
    let right = bounded(right)
        .await
        .unwrap()
        .unwrap()
        .into_inner()
        .value
        .unwrap();
    assert_eq!(right.value, b"right-result");
    assert!(
        !left.is_finished(),
        "another ID's success completed the held read"
    );
    session
        .replies
        .send(Ok(get_reply(first.id, b"left-result")))
        .await
        .unwrap();
    let left = bounded(left)
        .await
        .unwrap()
        .unwrap()
        .into_inner()
        .value
        .unwrap();
    assert_eq!(left.value, b"left-result");
    assert!(client
        .live_channel()
        .unwrap()
        .unwrap()
        .state
        .lock()
        .unwrap()
        .pending
        .is_empty());
    assert!(
        peer.sessions.try_recv().is_err(),
        "one call opened another stream"
    );
}

async fn held_write_interruption(use_deadline: bool) {
    let (entered_tx, mut entered) = mpsc::unbounded_channel();
    let (release, receiver) = oneshot::channel();
    let backend = Arc::new(Backend {
        write_gate: Some(WriteGate {
            entered: entered_tx,
            release: Mutex::new(Some(receiver)),
        }),
        ..Backend::default()
    });
    let api = Kv9Grpc::with_limits(
        backend.clone(),
        PublicApiLimits {
            max_requests: 1,
            max_encoded_bytes: 4096,
        },
    )
    .unwrap();
    let admission = api.admission();
    let (server, stream) = server(api).await;
    let client = public_client(stream.address, if use_deadline { 500 } else { 4_000 });
    let writer = client.clone();
    let task = tokio::spawn(async move {
        writer
            .call(RawOperation::Put {
                key: b"once".to_vec(),
                value: b"retained".to_vec(),
            })
            .await
    });
    bounded(entered.recv()).await.unwrap();
    let held = admission.snapshot();
    assert_eq!(held.in_flight, 1);
    assert!(held.encoded_bytes > 0);
    if use_deadline {
        let report = bounded(task).await.unwrap();
        assert!(matches!(report.outcome, Outcome::UnknownWrite { .. }));
        assert_eq!(report.attempts.len(), 1, "deadline replayed a write");
    } else {
        task.abort();
        assert!(bounded(task).await.unwrap_err().is_cancelled());
    }
    let refused = bounded(client.call(RawOperation::Get {
        key: b"once".to_vec(),
    }))
    .await;
    assert!(matches!(refused.outcome, Outcome::Refused { .. }));
    assert_eq!(refused.attempts.len(), 1);
    let after = admission.snapshot();
    assert_eq!(after.in_flight, held.in_flight);
    assert_eq!(
        after.encoded_bytes, held.encoded_bytes,
        "interruption lost the original byte reservation"
    );
    assert!(backend.values.lock().unwrap().is_empty());
    release.send(()).unwrap();
    eventually(|| admission.snapshot().in_flight == 0).await;
    assert_eq!(admission.snapshot().encoded_bytes, 0);
    let read = bounded(client.call(RawOperation::Get {
        key: b"once".to_vec(),
    }))
    .await;
    assert!(matches!(read.outcome, Outcome::Success { .. }));
    assert_eq!(
        backend.calls.lock().unwrap().len(),
        2,
        "one settled write and one new successful read, no replay"
    );
    stop(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn canceled_stream_write_keeps_its_original_admission_until_settlement() {
    held_write_interruption(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deadline_stream_write_is_unknown_and_keeps_its_original_admission() {
    held_write_interruption(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn canceling_one_call_closes_only_its_generation_and_all_collateral_waiters() {
    let mut peer = controlled_peer().await;
    let client = Arc::new(StreamClient::new(peer.address, 16));
    let a = client.clone();
    let first = tokio::spawn(async move { a.raw_put(request(put(b"a", b"v")), deadline()).await });
    let mut old_session = bounded(peer.sessions.recv()).await.unwrap();
    let _first_frame = bounded(old_session.incoming.message())
        .await
        .unwrap()
        .unwrap();
    let old = client.live_channel().unwrap().unwrap();
    let b = client.clone();
    let second = tokio::spawn(async move { b.raw_put(request(put(b"b", b"v")), deadline()).await });
    bounded(old_session.incoming.message())
        .await
        .unwrap()
        .unwrap();
    first.abort();
    assert!(bounded(first).await.unwrap_err().is_cancelled());
    let failure = bounded(second).await.unwrap().unwrap_err();
    assert_eq!(failure.code(), Code::Unavailable);
    assert!(failure.metadata().get(NOT_LEADER_KEY).is_none());
    eventually(|| old.dispatch.is_finished()).await;
    {
        let state = old.state.lock().unwrap();
        assert!(state.closed && state.pending.is_empty());
    }
    let c = client.clone();
    let next = tokio::spawn(async move { c.raw_get(request(get(b"new")), deadline()).await });
    let mut new_session = bounded(peer.sessions.recv()).await.unwrap();
    let frame = bounded(new_session.incoming.message())
        .await
        .unwrap()
        .unwrap();
    let new = client.live_channel().unwrap().unwrap();
    assert!(!Arc::ptr_eq(&old, &new));
    // A late old-generation guard must not reach through the cache into this stream.
    drop(PendingCall {
        channel: old.clone(),
        armed: true,
    });
    assert!(!new.state.lock().unwrap().closed);
    new_session
        .replies
        .send(Ok(get_reply(frame.id, b"new")))
        .await
        .unwrap();
    assert_eq!(
        bounded(next)
            .await
            .unwrap()
            .unwrap()
            .into_inner()
            .value
            .unwrap()
            .value,
        b"new"
    );
    let old_end = bounded(old_session.incoming.message()).await;
    assert!(
        !matches!(old_end, Ok(Some(_))),
        "old canceled payload was replayed"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_server_frames_close_the_stream_before_backend_dispatch() {
    let backend = Arc::new(Backend::default());
    let (server, client) = server(Kv9Grpc::new(backend.clone())).await;
    for (id, operation, remaining) in [
        (0, 1, 1_000_000),
        (1, 3, 1_000_000),
        (1, 257, 1_000_000),
        (1, 1, 0),
        (1, 1, 30_000_001),
        (1, 1, u64::MAX),
    ] {
        let mut raw = wire_client(client.address).await;
        let (send, receive) = mpsc::channel(2);
        let mut responses = bounded(raw.exchange(ReceiverStream::new(receive)))
            .await
            .unwrap()
            .into_inner();
        let mut request = frame(id, operation, put(b"invalid", b"v").encode_to_vec());
        request.remaining_micros = remaining;
        send.send(request).await.unwrap();
        let status = bounded(responses.message()).await.unwrap_err();
        assert_eq!(status.code(), Code::InvalidArgument);
        assert!(backend.calls.lock().unwrap().is_empty());
    }
    bounded(client.raw_get(request(get(b"healthy")), deadline()))
        .await
        .unwrap();
    assert_eq!(backend.calls.lock().unwrap().len(), 1);
    stop(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn duplicate_or_decreasing_request_id_never_reexecutes_a_write() {
    let backend = Arc::new(Backend::default());
    let (server, client) = server(Kv9Grpc::new(backend.clone())).await;
    for duplicate in [7, 6] {
        let mut raw = wire_client(client.address).await;
        let (send, receive) = mpsc::channel(2);
        let mut responses = bounded(raw.exchange(ReceiverStream::new(receive)))
            .await
            .unwrap()
            .into_inner();
        send.send(frame(7, 1, put(b"once", b"first").encode_to_vec()))
            .await
            .unwrap();
        assert_eq!(bounded(responses.message()).await.unwrap().unwrap().id, 7);
        let count = backend.calls.lock().unwrap().len();
        send.send(frame(duplicate, 1, put(b"once", b"second").encode_to_vec()))
            .await
            .unwrap();
        assert_eq!(
            bounded(responses.message()).await.unwrap_err().code(),
            Code::InvalidArgument
        );
        assert_eq!(backend.calls.lock().unwrap().len(), count);
        assert_eq!(
            backend
                .values
                .lock()
                .unwrap()
                .get(b"once".as_slice())
                .unwrap(),
            b"first"
        );
    }
    stop(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_or_unknown_reply_closes_all_pending_calls_without_retry() {
    for kind in 0..5 {
        let mut peer = controlled_peer().await;
        let client = public_client(peer.address, 4_000);
        let a = client.clone();
        let first = tokio::spawn(async move {
            a.call(RawOperation::Put {
                key: b"a".to_vec(),
                value: b"v".to_vec(),
            })
            .await
        });
        let mut session = bounded(peer.sessions.recv()).await.unwrap();
        let frame = bounded(session.incoming.message()).await.unwrap().unwrap();
        let b = client.clone();
        let second = tokio::spawn(async move {
            b.call(RawOperation::Put {
                key: b"b".to_vec(),
                value: b"v".to_vec(),
            })
            .await
        });
        bounded(session.incoming.message()).await.unwrap().unwrap();
        let mut reply = write_reply(frame.id);
        match kind {
            0 => reply.id = u64::MAX,
            1 => reply.reply = None,
            2 => reply.reply.as_mut().unwrap().payload = vec![0xff],
            3 => reply.reply.as_mut().unwrap().code = 17,
            _ => reply.reply.as_mut().unwrap().metadata.push(wire::Metadata {
                key: "bad key".into(),
                value: b"true".to_vec(),
                binary: false,
            }),
        }
        session.replies.send(Ok(reply)).await.unwrap();
        for task in [first, second] {
            let report = bounded(task).await.unwrap();
            assert!(
                matches!(report.outcome, Outcome::UnknownWrite { .. }),
                "malformed response proved a refusal or success"
            );
            assert_eq!(report.attempts.len(), 1, "malformed response caused replay");
        }
        assert!(
            peer.sessions.try_recv().is_err(),
            "failure opened an automatic retry stream"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn duplicate_reply_does_not_complete_a_different_pending_request() {
    let mut peer = controlled_peer().await;
    let client = Arc::new(StreamClient::new(peer.address, 16));
    let a = client.clone();
    let first = tokio::spawn(async move { a.raw_get(request(get(b"first")), deadline()).await });
    let mut session = bounded(peer.sessions.recv()).await.unwrap();
    let one = bounded(session.incoming.message()).await.unwrap().unwrap();
    session
        .replies
        .send(Ok(get_reply(one.id, b"one")))
        .await
        .unwrap();
    bounded(first).await.unwrap().unwrap();
    let b = client.clone();
    let second = tokio::spawn(async move { b.raw_get(request(get(b"second")), deadline()).await });
    let two = bounded(session.incoming.message()).await.unwrap().unwrap();
    assert!(two.id > one.id);
    session
        .replies
        .send(Ok(get_reply(one.id, b"wrong")))
        .await
        .unwrap();
    assert_eq!(
        bounded(second).await.unwrap().unwrap_err().code(),
        Code::Unavailable
    );
    eventually(|| client.live_channel().unwrap().is_none()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn id_exhaustion_closes_generation_instead_of_reusing_an_identifier() {
    let backend = Arc::new(Backend::default());
    let (server, client) = server(Kv9Grpc::new(backend.clone())).await;
    bounded(client.raw_get(request(get(b"seed")), deadline()))
        .await
        .unwrap();
    let old = client.live_channel().unwrap().unwrap();
    old.state.lock().unwrap().next_id = u64::MAX;
    let result = bounded(client.raw_put(request(put(b"unissued", b"v")), deadline())).await;
    assert_eq!(result.unwrap_err().code(), Code::Unavailable);
    assert_eq!(backend.calls.lock().unwrap().len(), 1);
    assert!(old.state.lock().unwrap().closed);
    bounded(client.raw_get(request(get(b"fresh")), deadline()))
        .await
        .unwrap();
    assert!(!Arc::ptr_eq(&old, &client.live_channel().unwrap().unwrap()));
    assert_eq!(backend.calls.lock().unwrap().len(), 2);
    stop(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_generation_restart_allows_new_calls_without_replay() {
    let backend = Arc::new(Backend::default());
    let (old_server, client) = server(Kv9Grpc::new(backend.clone())).await;
    bounded(client.raw_put(request(put(b"once", b"v")), deadline()))
        .await
        .unwrap();
    let old = client.live_channel().unwrap().unwrap();
    stop(old_server).await;
    eventually(|| old.dispatch.is_finished()).await;
    let server = start(
        client.address,
        Kv9Grpc::new(backend.clone()),
        authenticator(),
    )
    .await
    .unwrap();
    bounded(client.raw_get(request(get(b"once")), deadline()))
        .await
        .unwrap();
    assert!(!Arc::ptr_eq(&old, &client.live_channel().unwrap().unwrap()));
    assert_eq!(backend.calls.lock().unwrap().len(), 2);
    stop(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropping_client_releases_cached_generation_and_pending_transport() {
    let mut peer = controlled_peer().await;
    let client = StreamClient::new(peer.address, 16);
    let channel = bounded(client.connect()).await.unwrap();
    let mut session = bounded(peer.sessions.recv()).await.unwrap();
    let weak = Arc::downgrade(&channel);
    drop(channel);
    drop(client);
    eventually(|| weak.upgrade().is_none()).await;
    assert!(!matches!(
        bounded(session.incoming.message()).await,
        Ok(Some(_))
    ));
    eventually(|| session.replies.is_closed()).await;
}

#[test]
fn stream_wire_debug_redacts_authentication_and_payloads() {
    let request = frame(1, 1, b"distinct-private-payload".to_vec());
    let reply = get_reply(1, b"distinct-private-reply");
    assert!(!format!("{request:?}").contains("Bearer"));
    assert!(!format!("{request:?}").contains("distinct-private"));
    assert!(!format!("{reply:?}").contains("distinct-private"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stream_limit_is_held_after_headers_until_response_stream_drop() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let permits = Arc::new(Semaphore::new(2));
    let service = Service {
        handler: Handler {
            api: Kv9Grpc::new(Arc::new(Backend::default())),
            authenticator: authenticator(),
        },
        streams: permits.clone(),
        shutdown: CancellationToken::new(),
    };
    let task = tokio::spawn(async move {
        Server::builder()
            .add_service(wire::point_stream_server::PointStreamServer::new(service))
            .serve_with_incoming(TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    let mut client = wire_client(address).await;
    let (one_send, one_receive) = mpsc::channel(1);
    let one = bounded(client.exchange(ReceiverStream::new(one_receive)))
        .await
        .unwrap()
        .into_inner();
    let (two_send, two_receive) = mpsc::channel(1);
    let two = bounded(client.exchange(ReceiverStream::new(two_receive)))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        permits.available_permits(),
        0,
        "returning response headers released a live stream"
    );
    let (third_send, third_receive) = mpsc::channel(1);
    let error = bounded(client.exchange(ReceiverStream::new(third_receive)))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::ResourceExhausted);
    drop(one);
    drop(one_send);
    eventually(|| permits.available_permits() == 1).await;
    let (new_send, new_receive) = mpsc::channel(1);
    let new = bounded(client.exchange(ReceiverStream::new(new_receive)))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(permits.available_permits(), 0);
    drop((two, two_send, new, new_send, third_send));
    eventually(|| permits.available_permits() == 2).await;
    task.abort();
    let _ = bounded(task).await;
}

// Hold polling *before* tonic can drain Replies into HTTP/2 flow-control buffers.
// This observes the real service's reply reservations without guessing socket capacity.
struct HeldReplies {
    inner: Replies,
    release: Pin<Box<WaitForCancellationFutureOwned>>,
}

impl Stream for HeldReplies {
    type Item = Result<wire::PointResponse, Status>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.release.as_mut().poll(cx).is_pending() {
            return Poll::Pending;
        }
        Pin::new(&mut self.inner).poll_next(cx)
    }
}

#[derive(Clone)]
struct HeldService {
    inner: Service,
    release: CancellationToken,
}

#[tonic::async_trait]
impl wire::point_stream_server::PointStream for HeldService {
    type ExchangeStream = HeldReplies;
    async fn exchange(
        &self,
        request: Request<tonic::Streaming<wire::PointRequest>>,
    ) -> Result<Response<HeldReplies>, Status> {
        let response = self.inner.exchange(request).await?;
        Ok(Response::new(HeldReplies {
            inner: response.into_inner(),
            release: Box::pin(self.release.clone().cancelled_owned()),
        }))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completed_but_unconsumed_replies_keep_pending_work_bounded() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let backend = Arc::new(Backend::default());
    let api = Kv9Grpc::with_limits(
        backend.clone(),
        PublicApiLimits {
            max_requests: CHANNEL_LIMIT,
            max_encoded_bytes: 16_777_216,
        },
    )
    .unwrap();
    let admission = api.admission();
    let permits = Arc::new(Semaphore::new(1));
    let release = CancellationToken::new();
    let service = HeldService {
        inner: Service {
            handler: Handler {
                api,
                authenticator: authenticator(),
            },
            streams: permits.clone(),
            shutdown: CancellationToken::new(),
        },
        release: release.clone(),
    };
    let task = tokio::spawn(async move {
        Server::builder()
            .add_service(wire::point_stream_server::PointStreamServer::new(service))
            .serve_with_incoming(TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    let mut client = wire_client(address).await;
    let total = CHANNEL_LIMIT * 2 + 1;
    let (send, receive) = mpsc::channel(total);
    let mut responses = bounded(client.exchange(ReceiverStream::new(receive)))
        .await
        .unwrap()
        .into_inner();
    for id in 1..=total {
        send.send(frame(id as u64, 0, get(b"bounded").encode_to_vec()))
            .await
            .unwrap();
    }
    eventually(|| backend.calls.lock().unwrap().len() >= CHANNEL_LIMIT).await;
    eventually(|| admission.snapshot().in_flight == 0).await;
    // The complete response consumer is gated, not merely delayed by a scheduler.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        backend.calls.lock().unwrap().len(),
        CHANNEL_LIMIT,
        "completed replies escaped the pending-work budget"
    );
    assert_eq!(permits.available_permits(), 0);
    release.cancel();
    let mut ids = std::collections::BTreeSet::new();
    for _ in 0..total {
        let response = bounded(responses.message()).await.unwrap().unwrap();
        assert!(ids.insert(response.id), "duplicate response ID");
        assert_eq!(response.reply.unwrap().code, 0);
    }
    assert_eq!(ids, (1..=total as u64).collect());
    assert_eq!(backend.calls.lock().unwrap().len(), total);
    drop((send, responses));
    eventually(|| permits.available_permits() == 1).await;
    assert_eq!(admission.snapshot().encoded_bytes, 0);
    task.abort();
    let _ = bounded(task).await;
}
