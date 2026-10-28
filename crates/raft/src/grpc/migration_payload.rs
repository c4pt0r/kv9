//! Bounded streaming of an already committed capture's exact durable bytes.
//! Local copy completion is not a PUT/GC settlement or replica admission.
use super::*;
use kv9_common::{RegionId, StoreIdentity};
use prost::Message as _;
use std::io::{Cursor, Read};
use std::time::Instant;

pub const MAX_CHUNK_BYTES: usize = 256 * 1024;
pub const MAX_TRANSFER_TIME: Duration = Duration::from_secs(120);
const MAX_REQUEST_BYTES: usize = 512;
const MAX_REPLY_BYTES: usize = MAX_CHUNK_BYTES + 256;
pub type PayloadStream =
    tokio_stream::wrappers::ReceiverStream<std::result::Result<pb::MigrationCaptureChunk, Status>>;

/// Own the actual local group and charged file reader until streaming drains.
pub struct PayloadExport {
    pub reader: Box<dyn Read + Send>,
    pub length: u64,
}
fn unavailable() -> Error {
    Error::MetaNotReady("exact migration payload unavailable".into())
}
fn io_error() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "migration payload stream invalid or unavailable",
    )
}
fn current(discovery: &dyn GrpcDiscoveryState, identity: StoreIdentity) -> bool {
    discovery.raft_receive_allowed()
        && discovery.root_identity().root_digest == identity.root_digest
        && discovery
            .raw_ttl_format_fence()
            .is_some_and(|f| f.identity() == identity)
}

impl RaftGrpcService {
    fn check_payload_request(
        &self,
        request: Request<pb::MigrationCaptureTransferRequest>,
        copying: bool,
    ) -> std::result::Result<(pb::MigrationCaptureTransferRequest, StoreIdentity, Instant), Status>
    {
        let started = Instant::now();
        let caller = *request
            .extensions()
            .get::<NodeId>()
            .filter(|n| n.0 != 0)
            .ok_or_else(|| Status::unauthenticated("authenticated node identity missing"))?;
        let value = request.into_inner();
        if value.encoded_len() > MAX_REQUEST_BYTES
            || value.store_identity.len() != 80
            || value.region_id < 100
            || value.operation_id.len() != 16
            || value.operation_id == [0; 16]
            || value.descriptor_digest.len() != 32
            || value.descriptor_digest == [0; 32]
            || value.challenge.len() != 16
            || value.challenge == [0; 16]
            || value.budget_micros == 0
            || value.budget_micros > MAX_TRANSFER_TIME.as_micros() as u64
            || (copying && value.source_store_identity.len() != 80)
            || (!copying && !value.source_store_identity.is_empty())
        {
            return Err(Status::invalid_argument("invalid capture transfer bounds"));
        }
        let identity = StoreIdentity::decode(&value.store_identity)
            .map_err(|_| Status::invalid_argument("invalid transfer store"))?;
        let mut query = Request::new(pb::RawTtlFormatRequest {
            root_digest: identity.root_digest.as_bytes().to_vec(),
            cluster_id: identity.cluster_id.as_bytes().to_vec(),
            node_id: identity.node_id.0,
            store_incarnation: identity.store_incarnation.as_bytes().to_vec(),
        });
        query.extensions_mut().insert(caller);
        self.confirm_raw_ttl_format_request(query)?;
        let deadline = started + Duration::from_micros(value.budget_micros);
        Ok((value, identity, deadline))
    }

    pub(super) async fn fetch_migration_payload(
        &self,
        request: Request<pb::MigrationCaptureTransferRequest>,
    ) -> std::result::Result<Response<PayloadStream>, Status> {
        let (value, identity, deadline) = self.check_payload_request(request, false)?;
        let backend = self
            .registration
            .clone()
            .ok_or_else(|| Status::unimplemented("capture transfer backend unavailable"))?;
        let permit = self
            .checkpoint_history_capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("capture transfer busy"))?;
        let discovery = self.discovery.clone();
        let runtime = tokio::runtime::Handle::current();
        let (send, receive) = mpsc::channel(1);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let send_frame = |frame| {
                runtime.block_on(async {
                    tokio::time::timeout_at(
                        tokio::time::Instant::from_std(deadline),
                        send.send(frame),
                    )
                    .await
                    .map_err(|_| unavailable())?
                    .map_err(|_| unavailable())
                })
            };
            let result = (|| -> kv9_common::Result<()> {
                let mut export = backend.export_migration_capture(
                    identity.root_digest,
                    RegionId(value.region_id),
                    value.operation_id.as_slice().try_into().unwrap(),
                    RootDigest::from_bytes(value.descriptor_digest.as_slice().try_into().unwrap()),
                    deadline,
                )?;
                if export.length == 0 {
                    return Err(unavailable());
                }
                let mut offset = 0;
                while offset < export.length {
                    if Instant::now() >= deadline || !current(discovery.as_ref(), identity) {
                        return Err(unavailable());
                    }
                    let n = (export.length - offset).min(MAX_CHUNK_BYTES as u64) as usize;
                    let mut payload = vec![0; n];
                    export
                        .reader
                        .read_exact(&mut payload)
                        .map_err(|_| unavailable())?;
                    let last = offset + n as u64 == export.length;
                    if last {
                        let mut tail = [0];
                        if export.reader.read(&mut tail).map_err(|_| unavailable())? != 0
                            || !current(discovery.as_ref(), identity)
                        {
                            return Err(unavailable());
                        }
                    }
                    send_frame(Ok(pb::MigrationCaptureChunk {
                        challenge: value.challenge.clone(),
                        descriptor_digest: value.descriptor_digest.clone(),
                        offset,
                        total_bytes: export.length,
                        payload,
                        last,
                    }))?;
                    offset += n as u64;
                }
                Ok(())
            })();
            if result.is_err() {
                let _ = send_frame(Err(Status::unavailable(
                    "capture transfer did not complete",
                )));
            }
        });
        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            receive,
        )))
    }

    pub(super) async fn replicate_migration_payload(
        &self,
        request: Request<pb::MigrationCaptureTransferRequest>,
    ) -> std::result::Result<Response<pb::MigrationCaptureResponse>, Status> {
        let (value, identity, deadline) = self.check_payload_request(request, true)?;
        let source = StoreIdentity::decode(&value.source_store_identity)
            .map_err(|_| Status::invalid_argument("invalid payload source"))?;
        if source.node_id == identity.node_id
            || source.root_digest != identity.root_digest
            || source.cluster_id != identity.cluster_id
        {
            return Err(Status::invalid_argument(
                "payload source differs from transfer scope",
            ));
        }
        let backend = self
            .registration
            .clone()
            .ok_or_else(|| Status::unimplemented("capture copy backend unavailable"))?;
        let permit = self
            .checkpoint_history_capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("capture copy busy"))?;
        let operation = value.operation_id.as_slice().try_into().unwrap();
        let digest = RootDigest::from_bytes(value.descriptor_digest.as_slice().try_into().unwrap());
        let region = RegionId(value.region_id);
        let job = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            backend.replicate_migration_capture(
                identity.root_digest,
                region,
                operation,
                digest,
                source,
                deadline,
            )
        });
        tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), job)
            .await
            .map_err(|_| Status::deadline_exceeded("capture copy deadline"))?
            .map_err(|_| Status::unavailable("capture copy worker stopped"))?
            .map_err(|_| Status::unavailable("capture copy incomplete"))?;
        if !current(self.discovery.as_ref(), identity) {
            return Err(Status::unavailable("capture store authority changed"));
        }
        Ok(Response::new(pb::MigrationCaptureResponse {
            store_identity: value.store_identity,
            region_id: value.region_id,
            operation_id: value.operation_id,
            challenge: value.challenge,
            descriptor_digest: Some(value.descriptor_digest),
        }))
    }
}

fn request(
    store: StoreIdentity,
    descriptor: &crate::snapshot_install::capture::CaptureDescriptor,
    budget: Duration,
) -> kv9_common::Result<pb::MigrationCaptureTransferRequest> {
    if budget.as_micros() == 0
        || budget > MAX_TRANSFER_TIME
        || store.root_digest != descriptor.request().range.root
        || store.cluster_id != descriptor.request().source.cluster_id
    {
        return Err(unavailable());
    }
    Ok(pb::MigrationCaptureTransferRequest {
        store_identity: store.canonical_bytes(),
        region_id: descriptor.request().range.region.0,
        operation_id: descriptor.request().operation.to_vec(),
        descriptor_digest: descriptor.digest().as_bytes().to_vec(),
        challenge: kv9_common::ProcessIncarnation::mint()?.as_bytes().to_vec(),
        budget_micros: budget.as_micros() as u64,
        source_store_identity: vec![],
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn replicate_on_channel(
    channel: tonic::transport::Channel,
    store: StoreIdentity,
    source: StoreIdentity,
    descriptor: &crate::snapshot_install::capture::CaptureDescriptor,
    caller: NodeId,
    token: Option<String>,
    budget: Duration,
) -> kv9_common::Result<()> {
    let mut value = request(store, descriptor, budget)?;
    value.source_store_identity = source.canonical_bytes();
    let mut req = Request::new(value.clone());
    req.set_timeout(budget);
    attach_auth(&mut req, &token, caller);
    let mut client = Kv9RaftClient::new(channel)
        .max_encoding_message_size(MAX_REQUEST_BYTES)
        .max_decoding_message_size(MAX_REQUEST_BYTES);
    let reply = tokio::time::timeout(budget, client.replicate_migration_capture(req))
        .await
        .map_err(|_| unavailable())?
        .map_err(|_| unavailable())?
        .into_inner();
    if reply.store_identity != value.store_identity
        || reply.region_id != value.region_id
        || reply.operation_id != value.operation_id
        || reply.challenge != value.challenge
        || reply.descriptor_digest.as_deref() != Some(value.descriptor_digest.as_slice())
    {
        return Err(unavailable());
    }
    Ok(())
}

/// A synchronous receiver used only in a blocking worker. Bounds, sequence,
/// challenge, exact descriptor and terminal EOF are checked for every frame.
pub struct PayloadReader {
    stream: Streaming<pb::MigrationCaptureChunk>,
    runtime: tokio::runtime::Handle,
    sequence: PayloadSequence,
    buffered: Cursor<Vec<u8>>,
    deadline: Instant,
}
struct PayloadSequence {
    request: pb::MigrationCaptureTransferRequest,
    expected: u64,
    offset: u64,
    last: bool,
    done: bool,
}
impl Read for PayloadReader {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let n = self.buffered.read(bytes)?;
        if n != 0 {
            return Ok(n);
        }
        if self.sequence.done {
            return Ok(0);
        }
        let frame = self
            .runtime
            .block_on(async {
                tokio::time::timeout_at(
                    tokio::time::Instant::from_std(self.deadline),
                    self.stream.message(),
                )
                .await
            })
            .map_err(|_| io_error())?
            .map_err(|_| io_error())?;
        self.buffered = Cursor::new(self.sequence.accept(frame)?);
        self.buffered.read(bytes)
    }
}
impl PayloadSequence {
    fn new(request: pb::MigrationCaptureTransferRequest, expected: u64) -> Self {
        Self {
            request,
            expected,
            offset: 0,
            last: false,
            done: false,
        }
    }
    fn accept(&mut self, frame: Option<pb::MigrationCaptureChunk>) -> std::io::Result<Vec<u8>> {
        let Some(frame) = frame else {
            if !self.last || self.offset != self.expected {
                return Err(io_error());
            }
            self.done = true;
            return Ok(vec![]);
        };
        let len = frame.payload.len();
        if self.last
            || frame.encoded_len() > MAX_REPLY_BYTES
            || len == 0
            || len > MAX_CHUNK_BYTES
            || frame.challenge != self.request.challenge
            || frame.descriptor_digest != self.request.descriptor_digest
            || frame.offset != self.offset
            || frame.total_bytes != self.expected
            || self
                .offset
                .checked_add(len as u64)
                .is_none_or(|end| end > self.expected || frame.last != (end == self.expected))
        {
            return Err(io_error());
        }
        self.offset += len as u64;
        self.last = frame.last;
        Ok(frame.payload)
    }
}
pub async fn reader_on_channel(
    channel: tonic::transport::Channel,
    store: StoreIdentity,
    descriptor: &crate::snapshot_install::capture::CaptureDescriptor,
    caller: NodeId,
    token: Option<String>,
    budget: Duration,
) -> kv9_common::Result<PayloadReader> {
    let deadline = Instant::now() + budget;
    let value = request(store, descriptor, budget)?;
    let expected = kv9_engine::checkpoint::TransferJournal::encoded_len(
        descriptor.encode(),
        descriptor.manifest(),
    )?;
    let mut req = Request::new(value.clone());
    req.set_timeout(budget);
    attach_auth(&mut req, &token, caller);
    let mut client = Kv9RaftClient::new(channel)
        .max_encoding_message_size(MAX_REQUEST_BYTES)
        .max_decoding_message_size(MAX_REPLY_BYTES);
    let stream = tokio::time::timeout(budget, client.fetch_migration_capture(req))
        .await
        .map_err(|_| unavailable())?
        .map_err(|_| unavailable())?
        .into_inner();
    Ok(PayloadReader {
        stream,
        runtime: tokio::runtime::Handle::current(),
        sequence: PayloadSequence::new(value, expected),
        buffered: Cursor::new(vec![]),
        deadline,
    })
}

#[cfg(test)]
mod tests;
