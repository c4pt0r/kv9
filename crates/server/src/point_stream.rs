//! Multiplex point calls on one bounded bidirectional gRPC stream per endpoint.
//! The bounded adapter delegates consistency and admission to the existing handlers.

use crate::grpc::{proto, Authenticator, Kv9Grpc};
use crate::point_wire::{
    Handler, WireMetadata, WireReply, WireRequest, CHANNEL_LIMIT, CONNECTION_LIMIT, FRAME_LIMIT,
};
use futures_util::{stream::FuturesUnordered, Stream, StreamExt};
use prost::Message;
#[cfg(any(test, feature = "rpc-experiment"))]
use std::io;
use std::{
    collections::HashMap,
    fmt,
    future::Future,
    net::SocketAddr,
    pin::Pin,
    sync::{Arc, Mutex, RwLock},
    task::{Context, Poll},
    time::Duration,
};
#[cfg(any(test, feature = "rpc-experiment"))]
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
};
use tokio::{
    sync::{mpsc, oneshot, OwnedSemaphorePermit, Semaphore},
    task::JoinHandle,
    time::{timeout_at, Instant},
};
#[cfg(any(test, feature = "rpc-experiment"))]
use tokio_stream::wrappers::TcpListenerStream;
use tokio_util::sync::{CancellationToken, WaitForCancellationFutureOwned};
#[cfg(any(test, feature = "rpc-experiment"))]
use tonic::transport::{server::Connected, Server};
use tonic::{transport::Endpoint, Request, Response, Status};

#[cfg(test)]
mod tests;

pub(super) mod wire {
    tonic::include_proto!("kv9.point.v1");
}

// Prost's generated Debug would print bearer credentials and application data.
macro_rules! redacted_debug {
    ($($ty:ty),+ $(,)?) => {$(
        impl fmt::Debug for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_struct(stringify!($ty)).finish_non_exhaustive()
            }
        }
    )+};
}
redacted_debug!(
    wire::PointRequest,
    wire::PointResponse,
    wire::Reply,
    wire::Metadata
);

impl From<WireReply> for wire::Reply {
    fn from(reply: WireReply) -> Self {
        Self {
            code: reply.code,
            message: reply.message,
            details: reply.details,
            payload: reply.payload,
            metadata: reply
                .metadata
                .into_iter()
                .map(|entry| wire::Metadata {
                    key: entry.key,
                    value: entry.value,
                    binary: entry.binary,
                })
                .collect(),
        }
    }
}
impl From<wire::Reply> for WireReply {
    fn from(reply: wire::Reply) -> Self {
        Self {
            code: reply.code,
            message: reply.message,
            details: reply.details,
            payload: reply.payload,
            metadata: reply
                .metadata
                .into_iter()
                .map(|entry| WireMetadata {
                    key: entry.key,
                    value: entry.value,
                    binary: entry.binary,
                })
                .collect(),
        }
    }
}

#[derive(Clone)]
pub(crate) struct Service {
    handler: Handler,
    streams: Arc<Semaphore>,
    shutdown: CancellationToken,
}

pub(crate) fn service(
    api: Kv9Grpc,
    authenticator: Arc<dyn Authenticator>,
    shutdown: CancellationToken,
) -> wire::point_stream_server::PointStreamServer<Service> {
    wire::point_stream_server::PointStreamServer::new(Service {
        handler: Handler { api, authenticator },
        streams: Arc::new(Semaphore::new(CONNECTION_LIMIT)),
        shutdown,
    })
    .max_decoding_message_size(FRAME_LIMIT)
    .max_encoding_message_size(FRAME_LIMIT)
}

pub(super) struct Replies {
    receiver: mpsc::Receiver<Result<wire::PointResponse, Status>>,
    task: JoinHandle<()>,
    // A service call returning headers has not finished its response stream.
    _permit: Arc<OwnedSemaphorePermit>,
}
impl Stream for Replies {
    type Item = Result<wire::PointResponse, Status>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.receiver.poll_recv(cx)
    }
}
impl Drop for Replies {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[tonic::async_trait]
impl wire::point_stream_server::PointStream for Service {
    type ExchangeStream = Replies;
    async fn exchange(
        &self,
        request: Request<tonic::Streaming<wire::PointRequest>>,
    ) -> Result<Response<Replies>, Status> {
        self.handler
            .authenticator
            .authenticate(request.metadata())?;
        let permit = Arc::new(
            self.streams
                .clone()
                .try_acquire_owned()
                .map_err(|_| Status::resource_exhausted("point stream limit"))?,
        );
        let (outgoing, receiver) = mpsc::channel(CHANNEL_LIMIT);
        let mut incoming = request.into_inner();
        let handler = self.handler.clone();
        let shutdown = self.shutdown.clone();
        let stream_permit = permit.clone();
        let task = tokio::spawn(async move {
            // Each stream polls its bounded concurrent handlers directly.
            // Their wakeups coalesce onto this stream task rather than creating
            // a separately scheduled Tokio task for every request.
            let mut jobs = FuturesUnordered::new();
            let mut last_id = 0;
            let mut input_closed = false;
            loop {
                if input_closed && jobs.is_empty() {
                    break;
                }
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    _ = outgoing.closed() => break,
                    _ = jobs.next(), if !jobs.is_empty() => {},
                    received = async {
                        // The reservation travels with the handler until its
                        // reply is queued. Running + buffered replies <= limit.
                        let reserved = outgoing.clone().reserve_owned().await.ok()?;
                        Some((reserved, incoming.message().await))
                    }, if !input_closed && jobs.len() < CHANNEL_LIMIT => {
                        let Some((reserved, received)) = received else { break; };
                        let frame = match received {
                            Ok(Some(frame)) => frame,
                            Ok(None) => { input_closed = true; continue; },
                            Err(_) => {
                                reserved.send(Err(Status::unavailable("point stream input failed")));
                                break;
                            }
                        };
                        if frame.id <= last_id || frame.operation > 4 ||
                            !(1..=30_000_000).contains(&frame.remaining_micros) {
                            reserved.send(Err(Status::invalid_argument("invalid point stream frame")));
                            break;
                        }
                        last_id = frame.id;
                        // Checked bounded duration; transit does not extend the
                        // client's independent absolute monotonic deadline.
                        let deadline = Instant::now() + Duration::from_micros(frame.remaining_micros);
                        let handler = handler.clone();
                        let handler_permit = stream_permit.clone();
                        jobs.push(async move {
                            // Abort is cooperative. Keep this stream slot until
                            // the handler is actually dropped, even if the
                            // response stream and its owner have already gone.
                            let _stream_permit = handler_permit;
                            let reply = match timeout_at(deadline, handler.dispatch( frame.operation as u8,
                                WireRequest { authorization: frame.authorization, payload: frame.payload }
                            )).await {
                                Ok(reply) => reply,
                                Err(_) => WireReply::error(Status::deadline_exceeded("point deadline")),
                            };
                            let response = wire::PointResponse { id: frame.id, reply: Some(reply.into()) };
                            if response.encoded_len() > FRAME_LIMIT {
                                reserved.send(Err(Status::data_loss("point response exceeds frame limit")));
                            } else {
                                reserved.send(Ok(response));
                            }
                        });
                    }
                }
            }
            // Closing or panicking this stream drops all concurrent futures.
            // Existing owned write completion retains its admission reservation
            // until the exact apply settles, independently of these observers.
        });
        Ok(Response::new(Replies {
            receiver,
            task,
            _permit: permit,
        }))
    }
}

#[cfg(any(test, feature = "rpc-experiment"))]
struct Connection {
    stream: TcpStream,
    _permit: OwnedSemaphorePermit,
    cancelled: Pin<Box<WaitForCancellationFutureOwned>>,
}
#[cfg(any(test, feature = "rpc-experiment"))]
impl Connected for Connection {
    type ConnectInfo = ();
    fn connect_info(&self) {}
}
#[cfg(any(test, feature = "rpc-experiment"))]
impl AsyncRead for Connection {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.cancelled.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}
#[cfg(any(test, feature = "rpc-experiment"))]
impl AsyncWrite for Connection {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.cancelled.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
        Pin::new(&mut self.stream).poll_write(cx, bytes)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

#[cfg(any(test, feature = "rpc-experiment"))]
pub(crate) struct StreamServer {
    pub(crate) task: JoinHandle<io::Result<()>>,
    shutdown: CancellationToken,
}
#[cfg(any(test, feature = "rpc-experiment"))]
impl Drop for StreamServer {
    fn drop(&mut self) {
        self.shutdown.cancel();
        self.task.abort();
    }
}
#[cfg(any(test, feature = "rpc-experiment"))]
pub(crate) async fn start(
    address: SocketAddr,
    api: Kv9Grpc,
    authenticator: Arc<dyn Authenticator>,
) -> io::Result<StreamServer> {
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "streaming RPC experiment requires an explicit loopback address",
        ));
    }
    let listener = TcpListener::bind(address).await?;
    let shutdown = CancellationToken::new();
    let connections = Arc::new(Semaphore::new(CONNECTION_LIMIT));
    let service = Service {
        handler: Handler { api, authenticator },
        streams: Arc::new(Semaphore::new(CONNECTION_LIMIT)),
        shutdown: shutdown.clone(),
    };
    let stop = shutdown.clone();
    let incoming = TcpListenerStream::new(listener).filter_map(move |accepted| {
        let result = match accepted {
            Err(error) => Some(Err(error)),
            Ok(stream) => match connections.clone().try_acquire_owned() {
                Ok(permit) if stream.set_nodelay(true).is_ok() => Some(Ok(Connection {
                    stream,
                    _permit: permit,
                    cancelled: Box::pin(stop.clone().cancelled_owned()),
                })),
                _ => None,
            },
        };
        std::future::ready(result)
    });
    let task = tokio::spawn(async move {
        Server::builder()
            .max_concurrent_streams(CONNECTION_LIMIT as u32)
            .add_service(
                wire::point_stream_server::PointStreamServer::new(service)
                    .max_decoding_message_size(FRAME_LIMIT)
                    .max_encoding_message_size(FRAME_LIMIT),
            )
            .serve_with_incoming(incoming)
            .await
            .map_err(io::Error::other)
    });
    Ok(StreamServer { task, shutdown })
}

enum Decoded {
    Get(Response<proto::RawGetResponse>),
    BatchGet(Response<proto::RawBatchGetResponse>),
    Write(Response<proto::RawWriteResponse>),
}
impl Decoded {
    fn valid(&self, expected_items: usize) -> bool {
        match self {
            Self::Get(reply) => {
                !crate::client::has_control(reply.metadata())
                    && reply.get_ref().value.as_ref().is_some_and(|value| {
                        if value.found {
                            value.value.len() <= crate::client::MAX_VALUE_BYTES
                        } else {
                            value.value.is_empty()
                        }
                    })
            }
            Self::BatchGet(reply) => {
                !crate::client::has_control(reply.metadata())
                    && reply.get_ref().values.len() == expected_items
                    && reply
                        .get_ref()
                        .values
                        .iter()
                        .all(crate::client::valid_optional)
            }
            Self::Write(reply) => {
                !crate::client::has_control(reply.metadata())
                    && reply.get_ref().applied_term != 0
                    && reply.get_ref().applied_index != 0
            }
        }
    }
}
struct Pending {
    operation: u8,
    expected_items: usize,
    reply: oneshot::Sender<Result<Decoded, Status>>,
}
struct State {
    next_id: u64,
    closed: bool,
    pending: HashMap<u64, Pending>,
}
impl State {
    fn close(&mut self) {
        self.closed = true;
        for (_, pending) in self.pending.drain() {
            let _ = pending.reply.send(Err(unconfirmed()));
        }
    }
}
fn unconfirmed() -> Status {
    Status::unavailable("point stream outcome is unconfirmed")
}

struct StreamChannel {
    state: Arc<Mutex<State>>,
    outgoing: mpsc::Sender<wire::PointRequest>,
    dispatch: JoinHandle<()>,
    cancel: CancellationToken,
}
impl StreamChannel {
    fn close(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.close();
        }
        self.cancel.cancel();
        self.dispatch.abort();
    }
}
impl Drop for StreamChannel {
    fn drop(&mut self) {
        self.close();
    }
}
struct PendingCall {
    channel: Arc<StreamChannel>,
    armed: bool,
}
impl Drop for PendingCall {
    fn drop(&mut self) {
        if self.armed {
            self.channel.close();
        }
    }
}
struct ReaderState(Arc<Mutex<State>>, CancellationToken);
impl Drop for ReaderState {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.lock() {
            state.close();
        }
        self.1.cancel();
    }
}

struct Outgoing {
    receiver: mpsc::Receiver<wire::PointRequest>,
    cancelled: Pin<Box<WaitForCancellationFutureOwned>>,
}
impl Stream for Outgoing {
    type Item = wire::PointRequest;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.cancelled.as_mut().poll(cx).is_ready() {
            return Poll::Ready(None);
        }
        self.receiver.poll_recv(cx)
    }
}

pub(crate) struct StreamClient {
    address: SocketAddr,
    max_in_flight: usize,
    channel: RwLock<Option<Arc<StreamChannel>>>,
    connecting: tokio::sync::Mutex<()>,
}
impl StreamClient {
    pub(crate) fn new(address: SocketAddr, max_in_flight: usize) -> Self {
        Self {
            address,
            max_in_flight,
            channel: RwLock::new(None),
            connecting: tokio::sync::Mutex::new(()),
        }
    }
    fn live_channel(&self) -> Result<Option<Arc<StreamChannel>>, Status> {
        let channel = self.channel.read().map_err(|_| unconfirmed())?.clone();
        match channel {
            Some(channel)
                if !channel.dispatch.is_finished()
                    && !channel.state.lock().map_err(|_| unconfirmed())?.closed =>
            {
                Ok(Some(channel))
            }
            _ => Ok(None),
        }
    }
    async fn connect(&self, authorization: &str) -> Result<Arc<StreamChannel>, Status> {
        if let Some(channel) = self.live_channel()? {
            return Ok(channel);
        }
        let _connecting = self.connecting.lock().await;
        if let Some(channel) = self.live_channel()? {
            return Ok(channel);
        }
        let transport = Endpoint::from_shared(format!("http://{}", self.address))
            .map_err(|_| unconfirmed())?
            .tcp_nodelay(true)
            .connect_timeout(Duration::from_secs(1))
            .connect()
            .await
            .map_err(|_| unconfirmed())?;
        let mut client = wire::point_stream_client::PointStreamClient::new(transport)
            .max_decoding_message_size(FRAME_LIMIT)
            .max_encoding_message_size(FRAME_LIMIT);
        let (outgoing, incoming) = mpsc::channel(self.max_in_flight);
        let cancel = CancellationToken::new();
        let request_stream = Outgoing {
            receiver: incoming,
            cancelled: Box::pin(cancel.clone().cancelled_owned()),
        };
        let mut request = Request::new(request_stream);
        let mut header = authorization
            .parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>()
            .map_err(|_| Status::unauthenticated("invalid authorization"))?;
        header.set_sensitive(true);
        request.metadata_mut().insert("authorization", header);
        let mut responses = client
            .exchange(request)
            .await
            .map_err(|_| unconfirmed())?
            .into_inner();
        let state = Arc::new(Mutex::new(State {
            next_id: 1,
            closed: false,
            pending: HashMap::new(),
        }));
        let reader = ReaderState(state.clone(), cancel.clone());
        let dispatch = tokio::spawn(async move {
            // This guard is constructed before spawning: abort-before-first-poll
            // also closes every registered waiter.
            let reader = reader;
            while let Ok(Some(frame)) = responses.message().await {
                let Ok(mut state) = reader.0.lock() else {
                    break;
                };
                if state.closed {
                    break;
                }
                let Some(pending) = state.pending.remove(&frame.id) else {
                    state.close();
                    break;
                };
                let Some(reply) = frame.reply else {
                    state.close();
                    let _ = pending.reply.send(Err(unconfirmed()));
                    break;
                };
                let reply = WireReply::from(reply);
                let decoded = match pending.operation {
                    0 => reply.decode().map(Decoded::Get),
                    3 => reply.decode().map(Decoded::BatchGet),
                    _ => reply.decode().map(Decoded::Write),
                };
                let invalid = match &decoded {
                    Ok(reply) => !reply.valid(pending.expected_items),
                    Err(status) => {
                        status.code() == tonic::Code::DataLoss
                            || matches!(
                                crate::client::classify_status(
                                    status,
                                    matches!(pending.operation, 0 | 3)
                                ),
                                crate::client::Reason::Protocol
                            )
                    }
                };
                if invalid {
                    state.close();
                    let _ = pending.reply.send(Err(unconfirmed()));
                    break;
                }
                let _ = pending.reply.send(decoded);
            }
        });
        let channel = Arc::new(StreamChannel {
            state,
            outgoing,
            dispatch,
            cancel,
        });
        *self.channel.write().map_err(|_| unconfirmed())? = Some(channel.clone());
        Ok(channel)
    }
    async fn call<Q: Message>(
        &self,
        operation: u8,
        expected_items: usize,
        request: Request<Q>,
        deadline: Instant,
    ) -> Result<Decoded, Status> {
        let authorization = request
            .metadata()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| Status::unauthenticated("missing authorization"))?
            .to_owned();
        let channel = self.connect(&authorization).await?;
        let remaining = deadline
            .saturating_duration_since(Instant::now())
            .as_micros();
        if remaining == 0 || remaining > 30_000_000 {
            return Err(Status::deadline_exceeded("point deadline"));
        }
        let mut frame = wire::PointRequest {
            id: 0,
            operation: operation.into(),
            authorization,
            remaining_micros: remaining as u64,
            payload: request.into_inner().encode_to_vec(),
        };
        let (reply, receive) = oneshot::channel();
        // The guard owns this generation only; it cannot invalidate a later one.
        let mut guard = PendingCall {
            channel: channel.clone(),
            armed: true,
        };
        {
            let mut state = channel.state.lock().map_err(|_| unconfirmed())?;
            if state.closed || state.pending.len() >= self.max_in_flight {
                return Err(unconfirmed());
            }
            let Some(next_id) = state.next_id.checked_add(1) else {
                state.close();
                return Err(unconfirmed());
            };
            frame.id = state.next_id;
            state.next_id = next_id;
            let id = frame.id;
            state.pending.insert(
                id,
                Pending {
                    operation,
                    expected_items,
                    reply,
                },
            );
            // ID allocation, registration and FIFO enqueue are one linearization
            // point. No await or cancellation can split those actions.
            if channel.outgoing.try_send(frame).is_err() {
                state.close();
                return Err(unconfirmed());
            }
        }
        let result = receive.await.map_err(|_| unconfirmed())?;
        guard.armed = false;
        result
    }
    pub(crate) async fn raw_get(
        &self,
        request: Request<proto::RawGetRequest>,
        deadline: Instant,
    ) -> Result<Response<proto::RawGetResponse>, Status> {
        match self.call(0, 0, request, deadline).await? {
            Decoded::Get(reply) => Ok(reply),
            _ => Err(unconfirmed()),
        }
    }
    pub(crate) async fn raw_put(
        &self,
        request: Request<proto::RawPutRequest>,
        deadline: Instant,
    ) -> Result<Response<proto::RawWriteResponse>, Status> {
        match self.call(1, 0, request, deadline).await? {
            Decoded::Write(reply) => Ok(reply),
            _ => Err(unconfirmed()),
        }
    }
    pub(crate) async fn raw_delete(
        &self,
        request: Request<proto::RawDeleteRequest>,
        deadline: Instant,
    ) -> Result<Response<proto::RawWriteResponse>, Status> {
        match self.call(2, 0, request, deadline).await? {
            Decoded::Write(reply) => Ok(reply),
            _ => Err(unconfirmed()),
        }
    }
    pub(crate) async fn raw_batch_get(
        &self,
        request: Request<proto::RawBatchGetRequest>,
        deadline: Instant,
    ) -> Result<Response<proto::RawBatchGetResponse>, Status> {
        let count = request.get_ref().keys.len();
        match self.call(3, count, request, deadline).await? {
            Decoded::BatchGet(reply) => Ok(reply),
            _ => Err(unconfirmed()),
        }
    }
    pub(crate) async fn raw_batch_put(
        &self,
        request: Request<proto::RawBatchPutRequest>,
        deadline: Instant,
    ) -> Result<Response<proto::RawWriteResponse>, Status> {
        match self.call(4, 0, request, deadline).await? {
            Decoded::Write(reply) => Ok(reply),
            _ => Err(unconfirmed()),
        }
    }
}
