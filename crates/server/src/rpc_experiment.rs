//! Opt-in tarpc reference control for the selected streaming-gRPC transport.
pub use crate::client::TransportKind;
use crate::grpc::{proto, Authenticator, Kv9Grpc};
use crate::point_wire::*;
use futures_util::StreamExt;
use prost::Message;
use std::{
    net::SocketAddr,
    sync::{Arc, RwLock},
};
use tarpc::{
    context,
    server::{BaseChannel, Channel},
    tokio_serde::formats::Bincode,
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::Mutex,
    task::{JoinHandle, JoinSet},
    time::Instant,
};
use tokio_util::codec::LengthDelimitedCodec;
use tonic::{Request, Response, Status};

#[cfg(test)]
mod tests;

#[tarpc::service]
pub(crate) trait PointRpc {
    async fn point(operation: u8, request: WireRequest) -> WireReply;
}

impl PointRpc for Handler {
    async fn point(self, _: context::Context, operation: u8, request: WireRequest) -> WireReply {
        self.dispatch(operation, request).await
    }
}

pub(crate) struct ExperimentalServer {
    pub(crate) task: JoinHandle<std::io::Result<()>>,
}

impl Drop for ExperimentalServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(crate) async fn start(
    address: SocketAddr,
    api: Kv9Grpc,
    authenticator: Arc<dyn Authenticator>,
) -> std::io::Result<ExperimentalServer> {
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "RPC experiment requires an explicit loopback address",
        ));
    }
    let listener = TcpListener::bind(address).await?;
    let task = tokio::spawn(async move {
        let mut connections = JoinSet::new();
        loop {
            tokio::select! {
                finished = connections.join_next(), if !connections.is_empty() => {
                    if finished.is_some_and(|result| result.is_err()) {
                        // Deserialization runs before authentication. A malformed
                        // peer may kill its own channel, never the node listener.
                        eprintln!("experimental RPC connection terminated abnormally");
                    }
                }
                accepted = listener.accept() => {
                    let (stream, _) = accepted?;
                    if connections.len() >= CONNECTION_LIMIT { continue; }
                    if stream.set_nodelay(true).is_err() { continue; }
                    let transport = tarpc::serde_transport::new(
                        LengthDelimitedCodec::builder().max_frame_length(FRAME_LIMIT).new_framed(stream), Bincode::default());
                    let handler = Handler { api: api.clone(), authenticator: authenticator.clone() };
                    connections.spawn(async move {
                        BaseChannel::with_defaults(transport).max_concurrent_requests(CHANNEL_LIMIT)
                            .execute(handler.serve()).for_each_concurrent(Some(CHANNEL_LIMIT), |request| request).await;
                    });
                }
            }
        }
    });
    Ok(ExperimentalServer { task })
}

pub(crate) struct ExperimentClient {
    address: SocketAddr,
    max_in_flight: usize,
    channel: RwLock<Option<Arc<ClientChannel>>>,
    connecting: Mutex<()>,
}

struct ClientChannel {
    client: PointRpcClient,
    dispatch: JoinHandle<()>,
}

impl Drop for ClientChannel {
    fn drop(&mut self) {
        self.dispatch.abort();
    }
}

impl ExperimentClient {
    pub(crate) fn new(address: SocketAddr, max_in_flight: usize) -> Self {
        Self {
            address,
            max_in_flight,
            channel: RwLock::new(None),
            connecting: Mutex::new(()),
        }
    }
    fn live_channel(&self) -> Result<Option<Arc<ClientChannel>>, Status> {
        let guard = self
            .channel
            .read()
            .map_err(|_| Status::unavailable("experimental channel cache failed"))?;
        Ok(guard
            .as_ref()
            .filter(|channel| !channel.dispatch.is_finished())
            .cloned())
    }

    async fn connect(&self) -> Result<Arc<ClientChannel>, Status> {
        if let Some(channel) = self.live_channel()? {
            return Ok(channel);
        }
        let _connecting = self.connecting.lock().await;
        // Only a new logical operation enters here. Calls already holding an old
        // generation keep it and report its uncertain result without replay.
        if let Some(channel) = self.live_channel()? {
            return Ok(channel);
        }
        let stream = TcpStream::connect(self.address)
            .await
            .map_err(|_| Status::unavailable("experimental transport connect failed"))?;
        stream
            .set_nodelay(true)
            .map_err(|_| Status::unavailable("experimental TCP configuration failed"))?;
        let transport = tarpc::serde_transport::new(
            LengthDelimitedCodec::builder()
                .max_frame_length(FRAME_LIMIT)
                .new_framed(stream),
            Bincode::default(),
        );
        let mut config = tarpc::client::Config::default();
        config.max_in_flight_requests = self.max_in_flight;
        config.pending_request_buffer = self.max_in_flight;
        let new = PointRpcClient::new(config, transport);
        let channel = Arc::new(ClientChannel {
            client: new.client,
            dispatch: tokio::spawn(async move {
                let _ = new.dispatch.await;
            }),
        });
        *self
            .channel
            .write()
            .map_err(|_| Status::unavailable("experimental channel cache failed"))? =
            Some(channel.clone());
        Ok(channel)
    }
    async fn call<Q: Message, A: Message + Default>(
        &self,
        operation: u8,
        request: Request<Q>,
        deadline: Instant,
    ) -> Result<Response<A>, Status> {
        let channel = self.connect().await?;
        let authorization = request
            .metadata()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| Status::unauthenticated("missing authorization"))?
            .to_owned();
        let mut context = context::current();
        context.deadline = deadline.into_std();
        channel
            .client
            .point(
                context,
                operation,
                WireRequest {
                    authorization,
                    payload: request.into_inner().encode_to_vec(),
                },
            )
            .await
            .map_err(|_| Status::unavailable("experimental RPC outcome is unconfirmed"))?
            .decode()
    }
    pub(crate) async fn raw_get(
        &self,
        request: Request<proto::RawGetRequest>,
        deadline: Instant,
    ) -> Result<Response<proto::RawGetResponse>, Status> {
        self.call(0, request, deadline).await
    }
    pub(crate) async fn raw_put(
        &self,
        request: Request<proto::RawPutRequest>,
        deadline: Instant,
    ) -> Result<Response<proto::RawWriteResponse>, Status> {
        self.call(1, request, deadline).await
    }
    pub(crate) async fn raw_delete(
        &self,
        request: Request<proto::RawDeleteRequest>,
        deadline: Instant,
    ) -> Result<Response<proto::RawWriteResponse>, Status> {
        self.call(2, request, deadline).await
    }
    pub(crate) async fn raw_batch_get(
        &self,
        request: Request<proto::RawBatchGetRequest>,
        deadline: Instant,
    ) -> Result<Response<proto::RawBatchGetResponse>, Status> {
        self.call(3, request, deadline).await
    }
    pub(crate) async fn raw_batch_put(
        &self,
        request: Request<proto::RawBatchPutRequest>,
        deadline: Instant,
    ) -> Result<Response<proto::RawWriteResponse>, Status> {
        self.call(4, request, deadline).await
    }
}
