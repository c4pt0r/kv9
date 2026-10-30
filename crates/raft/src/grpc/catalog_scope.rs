//! The catalog-scope stream on the node-authenticated service
//! (docs/META-REPLICA-MODES.md §3b). A full node (metadata voter or full learner)
//! streams one scoped node's catalog slice: a snapshot first when the caller's
//! position is not in the server's change ring, then deltas, each naming the
//! position it applies on, and idle position updates. The rows and their routing
//! are the backend's; this layer only authenticates, bounds and frames.

use std::time::Duration;

use kv9_common::{AppliedPosition, Error, NodeId, RootDigest};
use tonic::{Request, Response, Status};

use super::{attach_auth, pb, RaftGrpcService};
use pb::kv9_raft_client::Kv9RaftClient;

/// Frames the backend produces for one subscriber, in order.
pub type FrameReceiver = tokio::sync::mpsc::Receiver<std::result::Result<pb::CatalogScopeMessage, Status>>;
pub type FrameStream = tokio_stream::wrappers::ReceiverStream<std::result::Result<pb::CatalogScopeMessage, Status>>;

/// A delta: apply iff at `prev`, then move to the message position.
pub const KIND_DELTA: u32 = 1;
/// A snapshot part: buffer the ops.
pub const KIND_SNAPSHOT_PART: u32 = 2;
/// The snapshot end: replace the slice with the buffered parts plus these ops.
pub const KIND_SNAPSHOT_END: u32 = 3;
/// One frame's ops stay under this many bytes (keys plus values).
pub const MAX_FRAME_BYTES: usize = 1 << 20;
/// Decode bound on the subscriber side: one frame plus framing.
pub const MAX_DECODE_BYTES: usize = MAX_FRAME_BYTES + (64 << 10);

impl RaftGrpcService {
    pub(super) async fn catalog_scope_request(
        &self,
        request: Request<pb::CatalogScopeRequest>,
    ) -> std::result::Result<Response<FrameStream>, Status> {
        let caller = *request
            .extensions()
            .get::<NodeId>()
            .filter(|n| n.0 != 0)
            .ok_or_else(|| Status::unauthenticated("authenticated node identity missing"))?;
        let value = request.into_inner();
        if value.root_digest.len() != 32 || value.store_incarnation.len() != 16 || value.store_incarnation == [0; 16] {
            return Err(Status::invalid_argument("invalid catalog scope request"));
        }
        if (value.from_term == 0) != (value.from_index == 0) {
            return Err(Status::invalid_argument("a scope position names both term and index"));
        }
        let root = RootDigest::from_bytes(value.root_digest.as_slice().try_into().expect("length checked"));
        let store: [u8; 16] = value.store_incarnation.as_slice().try_into().expect("length checked");
        let from = (value.from_index != 0).then_some(AppliedPosition {
            term: value.from_term,
            index: value.from_index,
        });
        let backend = self
            .registration
            .clone()
            .ok_or_else(|| Status::unimplemented("catalog scope backend unavailable"))?;
        let frames = tokio::task::spawn_blocking(move || backend.catalog_scope(root, caller, store, from))
            .await
            .map_err(|_| Status::internal("catalog scope task failed"))?;
        match frames {
            Ok(frames) => Ok(Response::new(FrameStream::new(frames))),
            Err(Error::NotImplemented(what)) => Err(Status::unimplemented(what)),
            Err(Error::NotLeader { .. }) | Err(Error::MetaNotReady(_)) => {
                Err(Status::unavailable("this node does not serve catalog scopes now"))
            }
            Err(error) => Err(Status::failed_precondition(error.to_string())),
        }
    }
}

/// Open the scope stream at the full node behind `channel`.
pub async fn subscribe_on_channel(
    channel: tonic::transport::Channel,
    root: RootDigest,
    store: [u8; 16],
    from: Option<AppliedPosition>,
    caller: NodeId,
    token: Option<String>,
    connect_budget: Duration,
) -> kv9_common::Result<tonic::Streaming<pb::CatalogScopeMessage>> {
    if caller.0 == 0 || store == [0; 16] || connect_budget.is_zero() {
        return Err(Error::Config("invalid catalog scope subscription".into()));
    }
    let mut client = Kv9RaftClient::new(channel).max_decoding_message_size(MAX_DECODE_BYTES);
    let from = from.unwrap_or(AppliedPosition { term: 0, index: 0 });
    let mut request = Request::new(pb::CatalogScopeRequest {
        root_digest: root.as_bytes().to_vec(),
        store_incarnation: store.to_vec(),
        from_term: from.term,
        from_index: from.index,
    });
    attach_auth(&mut request, &token, caller);
    match tokio::time::timeout(connect_budget, client.catalog_scope(request)).await {
        Err(_) => Err(Error::MetaNotReady("catalog scope subscription timed out".into())),
        Ok(Ok(stream)) => Ok(stream.into_inner()),
        Ok(Err(status)) if status.code() == tonic::Code::Unavailable => {
            Err(Error::MetaNotReady(format!("catalog scope: {}", status.message())))
        }
        Ok(Err(status)) => Err(Error::Config(format!("catalog scope refused: {}", status.message()))),
    }
}
