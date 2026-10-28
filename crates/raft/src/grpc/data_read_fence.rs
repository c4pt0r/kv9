//! A separate authenticated observation path for prospective DATA read floors.
//! Ordinary checkpoint selection/history replies cannot mint this result.
use super::*;
use kv9_common::data_read_fence::ReadFloor;
use kv9_common::{RegionId, StoreIdentity};
use prost::Message as _;
use std::time::Instant;

const MAX_BYTES: usize = 512;
const MAX_MICROS: u64 = 2_000_000;
#[derive(Debug, Clone)]
pub struct FenceTicket {
    pub(super) identity: StoreIdentity,
    pub(super) region: RegionId,
    pub(super) creation: RootDigest,
    challenge: [u8; 16],
}
impl FenceTicket {
    pub fn new(
        identity: StoreIdentity,
        region: RegionId,
        creation: RootDigest,
    ) -> kv9_common::Result<Self> {
        let t = Self {
            identity,
            region,
            creation,
            challenge: *kv9_common::ProcessIncarnation::mint()?.as_bytes(),
        };
        t.validate()?;
        Ok(t)
    }
    pub(super) fn validate(&self) -> kv9_common::Result<()> {
        if self.identity.root_digest.as_bytes() == &[0; 32]
            || self.identity.cluster_id.as_bytes() == &[0; 16]
            || self.identity.store_incarnation.as_bytes() == &[0; 16]
            || self.identity.node_id.0 == 0
            || self.region.0 < 100
            || self.creation.as_bytes() == &[0; 32]
            || self.challenge == [0; 16]
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub(super) fn request(&self, budget: Duration) -> pb::DataReadFenceRequest {
        pb::DataReadFenceRequest {
            root_digest: self.identity.root_digest.as_bytes().to_vec(),
            cluster_id: self.identity.cluster_id.as_bytes().to_vec(),
            node_id: self.identity.node_id.0,
            store_incarnation: self.identity.store_incarnation.as_bytes().to_vec(),
            region_id: self.region.0,
            creation: self.creation.as_bytes().to_vec(),
            challenge: self.challenge.to_vec(),
            budget_micros: budget.as_micros() as u64,
        }
    }
    pub(super) fn from_request(value: &pb::DataReadFenceRequest) -> kv9_common::Result<Self> {
        let t = Self {
            identity: StoreIdentity {
                root_digest: RootDigest::from_bytes(
                    value
                        .root_digest
                        .as_slice()
                        .try_into()
                        .map_err(|_| invalid())?,
                ),
                cluster_id: ClusterId::from_bytes(
                    value
                        .cluster_id
                        .as_slice()
                        .try_into()
                        .map_err(|_| invalid())?,
                ),
                node_id: NodeId(value.node_id),
                store_incarnation: StoreIncarnation::from_bytes(
                    value
                        .store_incarnation
                        .as_slice()
                        .try_into()
                        .map_err(|_| invalid())?,
                ),
            },
            region: RegionId(value.region_id),
            creation: RootDigest::from_bytes(
                value
                    .creation
                    .as_slice()
                    .try_into()
                    .map_err(|_| invalid())?,
            ),
            challenge: value
                .challenge
                .as_slice()
                .try_into()
                .map_err(|_| invalid())?,
        };
        t.validate()?;
        Ok(t)
    }
    fn accepts(&self, floor: &ReadFloor) -> bool {
        floor.validate().is_ok()
            && floor.identity == self.identity
            && floor.creation == self.creation
            && floor.image.region == self.region
    }
}
/// This result is created only by the independent inspection RPC. A decoded
/// disk descriptor or an ordinary selection observation is not sufficient.
#[derive(Debug)]
pub struct ConfirmedDataReadFence(Option<ReadFloor>);
impl ConfirmedDataReadFence {
    pub fn floor(&self) -> Option<ReadFloor> {
        self.0
    }
    fn decode(
        ticket: &FenceTicket,
        query: &pb::DataReadFenceRequest,
        reply: pb::DataReadFenceResponse,
    ) -> kv9_common::Result<Self> {
        if reply.encoded_len() > MAX_BYTES || reply.query.as_ref() != Some(query) {
            return Err(invalid());
        }
        let floor = match reply.observation {
            Some(pb::data_read_fence_response::Observation::Floor(b)) => {
                let floor = ReadFloor::decode(&b)?;
                if !ticket.accepts(&floor) {
                    return Err(invalid());
                }
                Some(floor)
            }
            Some(pb::data_read_fence_response::Observation::Unknown(true)) => None,
            _ => return Err(invalid()),
        };
        Ok(Self(floor))
    }
}
fn invalid() -> Error {
    Error::MetaNotReady("unconfirmed durable DATA read fence".into())
}

impl RaftGrpcService {
    pub(super) async fn inspect_data_read_fence_request(
        &self,
        request: Request<pb::DataReadFenceRequest>,
    ) -> std::result::Result<Response<pb::DataReadFenceResponse>, Status> {
        let started = Instant::now();
        let caller = *request
            .extensions()
            .get::<NodeId>()
            .filter(|n| n.0 != 0)
            .ok_or_else(|| Status::unauthenticated("authenticated node identity missing"))?;
        let value = request.into_inner();
        if value.encoded_len() > MAX_BYTES
            || !(1..=MAX_MICROS).contains(&value.budget_micros)
            || value.region_id < 100
            || value.creation.len() != 32
            || value.creation == [0; 32]
            || value.challenge.len() != 16
            || value.challenge == [0; 16]
        {
            return Err(Status::invalid_argument("invalid DATA read-fence bounds"));
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
                .map_err(|_| Status::internal("invalid local identity"))?,
        );
        let creation = RootDigest::from_bytes(value.creation.as_slice().try_into().unwrap());
        let region = RegionId(value.region_id);
        let backend = self
            .registration
            .clone()
            .ok_or_else(|| Status::unimplemented("DATA floor backend unavailable"))?;
        // Shared across history, selection and fence reads; cancellation retains
        // the permit inside the actual blocking worker until filesystem I/O exits.
        let permit = self
            .checkpoint_history_capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("checkpoint observation reader busy"))?;
        let deadline = started + Duration::from_micros(value.budget_micros);
        let job = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            if Instant::now() >= deadline {
                return Err(invalid());
            }
            let floor = backend.data_read_fence(root, region, creation, deadline)?;
            if Instant::now() >= deadline {
                return Err(invalid());
            }
            Ok(floor)
        });
        let floor = tokio::time::timeout(deadline.saturating_duration_since(Instant::now()), job)
            .await
            .map_err(|_| Status::deadline_exceeded("DATA read-fence deadline elapsed"))?
            .map_err(|_| Status::unavailable("DATA read-fence worker failed"))?
            .map_err(|_| Status::unavailable("DATA read fence remains unknown"))?;
        confirm()?;
        let observation = if let Some(floor) = floor {
            if floor.validate().is_err()
                || floor.identity.root_digest != root
                || floor.identity.cluster_id.as_bytes().as_slice() != identity.cluster_id
                || floor.identity.node_id.0 != identity.node_id
                || floor.identity.store_incarnation.as_bytes().as_slice()
                    != identity.store_incarnation
                || floor.image.region != region
                || floor.creation != creation
            {
                return Err(Status::unavailable("DATA read-fence identity differs"));
            }
            pb::data_read_fence_response::Observation::Floor(
                floor
                    .encode()
                    .map_err(|_| Status::internal("invalid DATA floor"))?,
            )
        } else {
            pb::data_read_fence_response::Observation::Unknown(true)
        };
        Ok(Response::new(pb::DataReadFenceResponse {
            query: Some(value),
            observation: Some(observation),
        }))
    }
}
pub async fn inspect_on_channel(
    channel: tonic::transport::Channel,
    ticket: &FenceTicket,
    caller: NodeId,
    token: Option<String>,
    budget: Duration,
) -> kv9_common::Result<ConfirmedDataReadFence> {
    ticket.validate()?;
    if caller.0 == 0 || budget.as_micros() == 0 || budget.as_micros() > u128::from(MAX_MICROS) {
        return Err(invalid());
    }
    let query = ticket.request(budget);
    let mut request = Request::new(query.clone());
    request.set_timeout(budget);
    attach_auth(&mut request, &token, caller);
    let mut client = Kv9RaftClient::new(channel)
        .max_encoding_message_size(MAX_BYTES)
        .max_decoding_message_size(MAX_BYTES);
    let reply = tokio::time::timeout(budget, client.inspect_data_read_fence(request))
        .await
        .map_err(|_| invalid())?
        .map_err(|_| invalid())?
        .into_inner();
    ConfirmedDataReadFence::decode(ticket, &query, reply)
}

#[cfg(test)]
mod tests;
