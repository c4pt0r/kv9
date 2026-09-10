//! Opt-in RPC framework comparison using the existing authenticated handlers.
//! Protobuf point payloads and typed metadata stay unchanged. Only the public
//! transport changes; Raft transport, quorum reads and write receipts do not.

use crate::grpc::{proto, proto::kv9_server::Kv9, Authenticator, Kv9Grpc};
use futures_util::StreamExt;
use prost::Message;
use serde::{Deserialize, Serialize};
use std::{
    fmt,
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
use tonic::{
    metadata::{KeyAndValueRef, MetadataMap},
    Request, Response, Status,
};

const FRAME_LIMIT: usize = crate::client::MAX_MESSAGE_BYTES + 8192;
const CONNECTION_LIMIT: usize = 8;
const CHANNEL_LIMIT: usize = crate::client::MAX_IN_FLIGHT;

pub(crate) mod stream;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportKind {
    #[default]
    TonicUnary,
    TarpcTcp,
    TonicStream,
}

#[derive(Serialize, Deserialize)]
struct WireMetadata {
    key: String,
    value: Vec<u8>,
    binary: bool,
}

fn encode_metadata(map: &MetadataMap) -> Result<Vec<WireMetadata>, Status> {
    map.iter()
        .map(|entry| match entry {
            KeyAndValueRef::Ascii(key, value) => Ok(WireMetadata {
                key: key.as_str().into(),
                value: value.as_encoded_bytes().to_vec(),
                binary: false,
            }),
            KeyAndValueRef::Binary(key, value) => Ok(WireMetadata {
                key: key.as_str().into(),
                value: value
                    .to_bytes()
                    .map_err(|_| Status::internal("invalid binary metadata"))?
                    .to_vec(),
                binary: true,
            }),
        })
        .collect()
}

fn decode_metadata(entries: Vec<WireMetadata>) -> Result<MetadataMap, Status> {
    let mut map = MetadataMap::new();
    for entry in entries {
        if entry.binary {
            let key = tonic::metadata::MetadataKey::<tonic::metadata::Binary>::from_bytes(
                entry.key.as_bytes(),
            )
            .map_err(|_| Status::data_loss("invalid binary metadata key"))?;
            map.append_bin(
                key,
                tonic::metadata::MetadataValue::from_bytes(&entry.value),
            );
        } else {
            let key = tonic::metadata::MetadataKey::<tonic::metadata::Ascii>::from_bytes(
                entry.key.as_bytes(),
            )
            .map_err(|_| Status::data_loss("invalid metadata key"))?;
            let value = tonic::metadata::MetadataValue::<tonic::metadata::Ascii>::try_from(
                entry.value.as_slice(),
            )
            .map_err(|_| Status::data_loss("invalid metadata value"))?;
            map.append(key, value);
        }
    }
    Ok(map)
}

#[derive(Serialize, Deserialize)]
pub struct WireReply {
    code: i32,
    message: String,
    details: Vec<u8>,
    metadata: Vec<WireMetadata>,
    payload: Vec<u8>,
}

impl fmt::Debug for WireReply {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WireReply")
            .field("code", &self.code)
            .field("payload_bytes", &self.payload.len())
            .finish_non_exhaustive()
    }
}

impl WireReply {
    fn encode<T: Message>(reply: Result<Response<T>, Status>) -> Self {
        match reply {
            Ok(response) => match encode_metadata(response.metadata()) {
                Ok(metadata) => Self {
                    code: 0,
                    message: String::new(),
                    details: Vec::new(),
                    metadata,
                    payload: response.into_inner().encode_to_vec(),
                },
                Err(error) => Self::error(error),
            },
            Err(error) => Self::error(error),
        }
    }
    fn error(error: Status) -> Self {
        match encode_metadata(error.metadata()) {
            Ok(metadata) => Self {
                code: error.code() as i32,
                message: error.message().into(),
                details: error.details().to_vec(),
                metadata,
                payload: Vec::new(),
            },
            Err(_) => Self {
                code: tonic::Code::Internal as i32,
                message: "invalid status metadata".into(),
                details: Vec::new(),
                metadata: Vec::new(),
                payload: Vec::new(),
            },
        }
    }
    fn decode<T: Message + Default>(self) -> Result<Response<T>, Status> {
        if self.payload.len() > crate::client::MAX_MESSAGE_BYTES || self.code < 0 || self.code > 16
        {
            return Err(Status::data_loss("invalid experimental response"));
        }
        let metadata = decode_metadata(self.metadata)?;
        if self.code != 0 {
            if !self.payload.is_empty() {
                return Err(Status::data_loss("error response contains success payload"));
            }
            return Err(Status::with_details_and_metadata(
                tonic::Code::from_i32(self.code),
                self.message,
                self.details.into(),
                metadata,
            ));
        }
        if !self.message.is_empty() || !self.details.is_empty() {
            return Err(Status::data_loss("success response contains error fields"));
        }
        let value = T::decode(self.payload.as_slice())
            .map_err(|_| Status::data_loss("invalid point response"))?;
        let mut response = Response::new(value);
        *response.metadata_mut() = metadata;
        Ok(response)
    }
}

#[derive(Serialize, Deserialize)]
pub struct WireRequest {
    authorization: String,
    payload: Vec<u8>,
}

impl fmt::Debug for WireRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WireRequest")
            .field("payload_bytes", &self.payload.len())
            .finish_non_exhaustive()
    }
}

#[tarpc::service]
pub trait PointRpc {
    async fn point(operation: u8, request: WireRequest) -> WireReply;
}

#[derive(Clone)]
struct Handler {
    api: Kv9Grpc,
    authenticator: Arc<dyn Authenticator>,
}

impl Handler {
    fn request<T: Message + Default>(
        &self,
        authorization: &str,
        payload: &[u8],
    ) -> Result<Request<T>, Status> {
        if authorization.len() > 4103 || payload.len() > crate::client::MAX_MESSAGE_BYTES {
            return Err(Status::resource_exhausted(
                "experimental frame exceeds point limits",
            ));
        }
        let mut metadata = MetadataMap::new();
        metadata.insert(
            "authorization",
            authorization
                .parse()
                .map_err(|_| Status::unauthenticated("invalid authorization"))?,
        );
        let auth = self.authenticator.authenticate(&metadata)?;
        let message =
            T::decode(payload).map_err(|_| Status::invalid_argument("invalid point request"))?;
        let mut request = Request::new(message);
        *request.metadata_mut() = metadata;
        request.extensions_mut().insert(auth);
        Ok(request)
    }
}

impl PointRpc for Handler {
    async fn point(self, _: context::Context, operation: u8, request: WireRequest) -> WireReply {
        let WireRequest {
            authorization,
            payload,
        } = request;
        match operation {
            0 => WireReply::encode(match self.request(&authorization, &payload) {
                Ok(request) => self.api.raw_get(request).await,
                Err(error) => Err(error),
            }),
            1 => WireReply::encode(match self.request(&authorization, &payload) {
                Ok(request) => self.api.raw_put(request).await,
                Err(error) => Err(error),
            }),
            2 => WireReply::encode(match self.request(&authorization, &payload) {
                Ok(request) => self.api.raw_delete(request).await,
                Err(error) => Err(error),
            }),
            _ => WireReply::error(Status::unimplemented("unknown point operation")),
        }
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
}
