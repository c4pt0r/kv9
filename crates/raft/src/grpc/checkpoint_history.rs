//! Bounded, authenticated observations of one immutable checkpoint window.
//! A responder may be a follower. Unknown, unsupported or uncorrelated replies
//! never imply that an identity lost its CAS window.
use super::*;
use kv9_common::{RegionId, StoreIdentity};
use prost::Message as _;
use std::time::Instant;

const MAX_BYTES: usize = 512;
const MAX_MICROS: u64 = 2_000_000;
#[derive(Clone, Copy)]
enum Observation {
    History,
    Selection,
    BootstrapFence,
}

#[cfg(test)]
mod tests;

#[derive(Debug, Clone)]
pub struct HistoryTicket {
    identity: StoreIdentity,
    region: RegionId,
    predecessor: u64,
    challenge: [u8; 16],
}
impl HistoryTicket {
    pub fn new(
        identity: StoreIdentity,
        region: RegionId,
        predecessor: u64,
    ) -> kv9_common::Result<Self> {
        let ticket = Self {
            identity,
            region,
            predecessor,
            challenge: *kv9_common::ProcessIncarnation::mint()?.as_bytes(),
        };
        ticket.validate()?;
        Ok(ticket)
    }
    fn validate(&self) -> kv9_common::Result<()> {
        if self.identity.root_digest.as_bytes() == &[0; 32]
            || self.identity.cluster_id.as_bytes() == &[0; 16]
            || self.identity.store_incarnation.as_bytes() == &[0; 16]
            || self.identity.node_id.0 == 0
            || (self.region != kv9_common::META_REGION_0 && self.region.0 < 100)
            || self.predecessor == u64::MAX
            || self.challenge == [0; 16]
        {
            return Err(invalid());
        }
        Ok(())
    }
    fn request(&self, budget: Duration) -> pb::CheckpointHistoryRequest {
        pb::CheckpointHistoryRequest {
            root_digest: self.identity.root_digest.as_bytes().to_vec(),
            cluster_id: self.identity.cluster_id.as_bytes().to_vec(),
            node_id: self.identity.node_id.0,
            store_incarnation: self.identity.store_incarnation.as_bytes().to_vec(),
            region_id: self.region.0,
            predecessor: self.predecessor,
            challenge: self.challenge.to_vec(),
            budget_micros: budget.as_micros() as u64,
        }
    }
}

#[derive(Debug)]
pub struct ConfirmedHistory {
    winner: Option<[u8; 32]>,
}
impl ConfirmedHistory {
    pub fn winner_change_id(&self) -> Option<[u8; 32]> {
        self.winner
    }
    fn decode(
        ticket: &HistoryTicket,
        reply: pb::CheckpointHistoryResponse,
    ) -> kv9_common::Result<Self> {
        if reply.encoded_len() > MAX_BYTES
            || reply.root_digest.as_slice() != ticket.identity.root_digest.as_bytes()
            || reply.cluster_id.as_slice() != ticket.identity.cluster_id.as_bytes()
            || reply.node_id != ticket.identity.node_id.0
            || reply.store_incarnation.as_slice() != ticket.identity.store_incarnation.as_bytes()
            || reply.region_id != ticket.region.0
            || reply.predecessor != ticket.predecessor
            || reply.challenge.as_slice() != ticket.challenge
        {
            return Err(invalid());
        }
        let winner = match reply.observation {
            Some(pb::checkpoint_history_response::Observation::WinnerChangeId(bytes)) => {
                Some(bytes.try_into().map_err(|_| invalid())?)
            }
            Some(pb::checkpoint_history_response::Observation::Unknown(true)) => None,
            _ => return Err(invalid()),
        };
        Ok(Self { winner })
    }
}
fn invalid() -> Error {
    Error::MetaNotReady("unconfirmed checkpoint history".into())
}

impl RaftGrpcService {
    pub(super) async fn inspect_checkpoint_history_request(
        &self,
        request: Request<pb::CheckpointHistoryRequest>,
    ) -> std::result::Result<Response<pb::CheckpointHistoryResponse>, Status> {
        self.inspect_checkpoint_request(request, Observation::History)
            .await
    }
    pub(super) async fn inspect_checkpoint_selection_request(
        &self,
        request: Request<pb::CheckpointHistoryRequest>,
    ) -> std::result::Result<Response<pb::CheckpointHistoryResponse>, Status> {
        self.inspect_checkpoint_request(request, Observation::Selection)
            .await
    }
    pub(super) async fn inspect_bootstrap_read_fence_request(
        &self,
        request: Request<pb::CheckpointHistoryRequest>,
    ) -> std::result::Result<Response<pb::CheckpointHistoryResponse>, Status> {
        self.inspect_checkpoint_request(request, Observation::BootstrapFence)
            .await
    }
    async fn inspect_checkpoint_request(
        &self,
        request: Request<pb::CheckpointHistoryRequest>,
        observation: Observation,
    ) -> std::result::Result<Response<pb::CheckpointHistoryResponse>, Status> {
        let started = Instant::now();
        let caller = *request
            .extensions()
            .get::<NodeId>()
            .filter(|n| n.0 != 0)
            .ok_or_else(|| Status::unauthenticated("authenticated node identity missing"))?;
        let value = request.into_inner();
        if value.encoded_len() > MAX_BYTES
            || value.challenge.len() != 16
            || value.challenge == [0; 16]
            || !(1..=MAX_MICROS).contains(&value.budget_micros)
            || (value.region_id != kv9_common::META_REGION_0.0 && value.region_id < 100)
            || value.predecessor == u64::MAX
            || (matches!(observation, Observation::BootstrapFence)
                && value.region_id != kv9_common::META_REGION_0.0)
        {
            return Err(Status::invalid_argument(
                "invalid checkpoint history query bounds",
            ));
        }
        let confirm = || {
            let mut request = Request::new(pb::RawTtlFormatRequest {
                root_digest: value.root_digest.clone(),
                cluster_id: value.cluster_id.clone(),
                node_id: value.node_id,
                store_incarnation: value.store_incarnation.clone(),
            });
            request.extensions_mut().insert(caller);
            self.confirm_raw_ttl_format_request(request)
        };
        let identity = confirm()?.into_inner();
        let root = RootDigest::from_bytes(
            identity
                .root_digest
                .as_slice()
                .try_into()
                .map_err(|_| Status::internal("invalid local checkpoint history identity"))?,
        );
        let backend = self
            .registration
            .clone()
            .ok_or_else(|| Status::unimplemented("checkpoint history backend unavailable"))?;
        let permit = self
            .checkpoint_history_capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("checkpoint history reader busy"))?;
        let region = RegionId(value.region_id);
        let predecessor = value.predecessor;
        let deadline = started + Duration::from_micros(value.budget_micros);
        let job = tokio::task::spawn_blocking(move || {
            let _permit = permit; // Cancellation never releases an actual blocked reader.
            if Instant::now() >= deadline {
                return Err(invalid());
            }
            let winner = match observation {
                Observation::Selection => {
                    backend.checkpoint_selection(root, region, predecessor, deadline)?
                }
                Observation::History => {
                    backend.checkpoint_history(root, region, predecessor, deadline)?
                }
                Observation::BootstrapFence => {
                    backend.bootstrap_read_fence(root, predecessor, deadline)?
                }
            };
            if Instant::now() >= deadline {
                return Err(invalid());
            }
            Ok(winner)
        });
        let winner = tokio::time::timeout(deadline.saturating_duration_since(Instant::now()), job)
            .await
            .map_err(|_| Status::deadline_exceeded("checkpoint history deadline elapsed"))?
            .map_err(|_| Status::unavailable("checkpoint history worker failed"))?
            .map_err(|_| Status::unavailable("checkpoint history remains unknown"))?;
        confirm()?; // Store authority may have been withdrawn during the read.
        Ok(Response::new(pb::CheckpointHistoryResponse {
            root_digest: identity.root_digest,
            cluster_id: identity.cluster_id,
            node_id: identity.node_id,
            store_incarnation: identity.store_incarnation,
            region_id: value.region_id,
            predecessor: value.predecessor,
            challenge: value.challenge,
            observation: Some(match winner {
                Some(id) => {
                    pb::checkpoint_history_response::Observation::WinnerChangeId(id.to_vec())
                }
                None => pb::checkpoint_history_response::Observation::Unknown(true),
            }),
        }))
    }
}

pub async fn inspect_on_channel(
    channel: tonic::transport::Channel,
    ticket: &HistoryTicket,
    caller: NodeId,
    token: Option<String>,
    budget: Duration,
) -> kv9_common::Result<ConfirmedHistory> {
    inspect(channel, ticket, caller, token, budget, Observation::History).await
}

/// Confirmed only through the distinct selection RPC. History replies cannot
/// be converted into this observation, and Unknown never means superseded.
/// ```compile_fail
/// fn convert(h: kv9_raft::grpc::checkpoint_history::ConfirmedHistory) {
///     let _ = kv9_raft::grpc::checkpoint_history::ConfirmedSelection(h);
/// }
/// ```
#[derive(Debug)]
pub struct ConfirmedSelection(ConfirmedHistory);
/// Only the distinct bootstrap-fence RPC can mint this observation. The value
/// is an exact image digest, never a claim from ordinary committed history.
/// ```compile_fail
/// fn forge(h: kv9_raft::grpc::checkpoint_history::ConfirmedHistory) {
///     let _ = kv9_raft::grpc::checkpoint_history::ConfirmedBootstrapFence(h);
/// }
/// ```
#[derive(Debug)]
pub struct ConfirmedBootstrapFence(ConfirmedHistory);
impl ConfirmedBootstrapFence {
    pub fn image_digest(&self) -> Option<[u8; 32]> {
        self.0.winner_change_id()
    }
}
pub async fn inspect_bootstrap_fence_on_channel(
    channel: tonic::transport::Channel,
    ticket: &HistoryTicket,
    caller: NodeId,
    token: Option<String>,
    budget: Duration,
) -> kv9_common::Result<ConfirmedBootstrapFence> {
    if ticket.region != kv9_common::META_REGION_0 {
        return Err(invalid());
    }
    inspect(
        channel,
        ticket,
        caller,
        token,
        budget,
        Observation::BootstrapFence,
    )
    .await
    .map(ConfirmedBootstrapFence)
}
impl ConfirmedSelection {
    pub fn selected_change_id(&self) -> Option<[u8; 32]> {
        self.0.winner_change_id()
    }
}
pub async fn inspect_selection_on_channel(
    channel: tonic::transport::Channel,
    ticket: &HistoryTicket,
    caller: NodeId,
    token: Option<String>,
    budget: Duration,
) -> kv9_common::Result<ConfirmedSelection> {
    inspect(
        channel,
        ticket,
        caller,
        token,
        budget,
        Observation::Selection,
    )
    .await
    .map(ConfirmedSelection)
}
async fn inspect(
    channel: tonic::transport::Channel,
    ticket: &HistoryTicket,
    caller: NodeId,
    token: Option<String>,
    budget: Duration,
    observation: Observation,
) -> kv9_common::Result<ConfirmedHistory> {
    ticket.validate()?;
    if caller.0 == 0 || budget.as_micros() == 0 || budget.as_micros() > u128::from(MAX_MICROS) {
        return Err(invalid());
    }
    let mut request = Request::new(ticket.request(budget));
    request.set_timeout(budget);
    attach_auth(&mut request, &token, caller);
    let mut client = Kv9RaftClient::new(channel)
        .max_encoding_message_size(MAX_BYTES)
        .max_decoding_message_size(MAX_BYTES);
    let query = async {
        match observation {
            Observation::Selection => client.inspect_checkpoint_selection(request).await,
            Observation::History => client.inspect_checkpoint_history(request).await,
            Observation::BootstrapFence => client.inspect_bootstrap_read_fence(request).await,
        }
    };
    let reply = tokio::time::timeout(budget, query)
        .await
        .map_err(|_| invalid())?
        .map_err(|_| invalid())?
        .into_inner();
    ConfirmedHistory::decode(ticket, reply)
}
