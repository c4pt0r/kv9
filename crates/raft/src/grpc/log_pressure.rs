//! Read-only exact-store pressure reports with request correlation. A report
//! reserves no remote capacity and is never a receipt for replicated data.
use super::*;
use crate::replica_pressure::PressureTicket;
use kv9_common::StoreIdentity;
use prost::Message as _;
use std::time::Instant;

const MAX_BYTES: usize = 512;
const MAX_BUDGET_MICROS: u64 = 2_000_000;
#[derive(Debug)]
pub struct ConfirmedPressure {
    identity: StoreIdentity,
    challenge: [u8; 16],
    capacity: Option<u64>,
}
impl ConfirmedPressure {
    pub(crate) fn matches(&self, ticket: &PressureTicket) -> bool {
        self.identity == ticket.route.identity && self.challenge == ticket.challenge
    }
    pub(crate) fn capacity(&self) -> Option<u64> {
        self.capacity
    }
    pub(crate) fn from_response(
        ticket: &PressureTicket,
        response: pb::LogPressureResponse,
    ) -> kv9_common::Result<Self> {
        let expected = ticket.route.identity;
        if response.encoded_len() > MAX_BYTES
            || response.root_digest.as_slice() != expected.root_digest.as_bytes()
            || response.cluster_id.as_slice() != expected.cluster_id.as_bytes()
            || response.node_id != expected.node_id.0
            || response.store_incarnation.as_slice() != expected.store_incarnation.as_bytes()
            || response.challenge.as_slice() != ticket.challenge
        {
            return Err(invalid());
        }
        let capacity = match response.observation {
            Some(pb::log_pressure_response::Observation::AvailableBytes(bytes)) => Some(bytes),
            Some(pb::log_pressure_response::Observation::Uncertain(true)) => None,
            _ => return Err(invalid()),
        };
        Ok(Self {
            identity: expected,
            challenge: ticket.challenge,
            capacity,
        })
    }
}
fn invalid() -> Error {
    Error::MetaNotReady("uncorrelated or invalid replica pressure observation".into())
}

impl RaftGrpcService {
    pub fn with_log_budget(mut self, budget: Arc<kv9_common::log_budget::LogBudget>) -> Self {
        self.log_budget = Some(budget);
        self
    }
    pub(super) async fn inspect_log_pressure_request(
        &self,
        request: Request<pb::LogPressureRequest>,
    ) -> std::result::Result<Response<pb::LogPressureResponse>, Status> {
        let caller = *request
            .extensions()
            .get::<NodeId>()
            .filter(|node| node.0 != 0)
            .ok_or_else(|| Status::unauthenticated("authenticated node identity missing"))?;
        let value = request.into_inner();
        if value.encoded_len() > MAX_BYTES
            || value.challenge.len() != 16
            || value.challenge == [0; 16]
            || value.budget_micros == 0
            || value.budget_micros > MAX_BUDGET_MICROS
        {
            return Err(Status::invalid_argument(
                "invalid pressure request budget or challenge",
            ));
        }
        // Reuse the exact active StoreGuard/root identity validation. A shared
        // cluster token or a request body alone cannot mint another store.
        let mut identity_request = Request::new(pb::RawTtlFormatRequest {
            root_digest: value.root_digest,
            cluster_id: value.cluster_id,
            node_id: value.node_id,
            store_incarnation: value.store_incarnation,
        });
        identity_request.extensions_mut().insert(caller);
        let identity = self
            .confirm_raw_ttl_format_request(identity_request)?
            .into_inner();
        let budget = self
            .log_budget
            .clone()
            .ok_or_else(|| Status::unimplemented("local log pressure provider unavailable"))?;
        let permit = self
            .log_pressure_capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("local pressure probe is busy"))?;
        let job = tokio::task::spawn_blocking(move || {
            // Retain actual capacity even if timeout/cancellation drops the RPC.
            let _permit = permit;
            budget.observe_data_capacity()
        });
        let capacity = tokio::time::timeout(Duration::from_micros(value.budget_micros), job)
            .await
            .map_err(|_| Status::deadline_exceeded("pressure probe deadline elapsed"))?
            .map_err(|_| Status::unavailable("pressure probe worker failed"))?;
        // Authority may be withdrawn while a filesystem syscall is blocked.
        // Revalidate the same durable store before emitting its observation.
        let mut current = Request::new(pb::RawTtlFormatRequest {
            root_digest: identity.root_digest.clone(),
            cluster_id: identity.cluster_id.clone(),
            node_id: identity.node_id,
            store_incarnation: identity.store_incarnation.clone(),
        });
        current.extensions_mut().insert(caller);
        self.confirm_raw_ttl_format_request(current)?;
        Ok(Response::new(pb::LogPressureResponse {
            root_digest: identity.root_digest,
            cluster_id: identity.cluster_id,
            node_id: identity.node_id,
            store_incarnation: identity.store_incarnation,
            challenge: value.challenge,
            observation: Some(match capacity {
                Some(bytes) => pb::log_pressure_response::Observation::AvailableBytes(bytes),
                None => pb::log_pressure_response::Observation::Uncertain(true),
            }),
        }))
    }
}

/// Timeout/malformed/unsupported replies return no observation and cannot
/// clear a previously reported capacity shortage.
pub async fn inspect_on_channel(
    channel: tonic::transport::Channel,
    ticket: &PressureTicket,
    caller: NodeId,
    token: Option<String>,
    budget: Duration,
) -> kv9_common::Result<ConfirmedPressure> {
    let micros = budget.as_micros();
    if caller.0 == 0 || micros == 0 || micros > u128::from(MAX_BUDGET_MICROS) {
        return Err(invalid());
    }
    let identity = ticket.route.identity;
    let mut request = Request::new(pb::LogPressureRequest {
        root_digest: identity.root_digest.as_bytes().to_vec(),
        cluster_id: identity.cluster_id.as_bytes().to_vec(),
        node_id: identity.node_id.0,
        store_incarnation: identity.store_incarnation.as_bytes().to_vec(),
        challenge: ticket.challenge.to_vec(),
        budget_micros: micros as u64,
    });
    attach_auth(&mut request, &token, caller);
    request.set_timeout(budget);
    let mut client = Kv9RaftClient::new(channel)
        .max_decoding_message_size(MAX_BYTES)
        .max_encoding_message_size(MAX_BYTES);
    let response = tokio::time::timeout(budget, client.inspect_log_pressure(request))
        .await
        .map_err(|_| invalid())?
        .map_err(|_| invalid())?
        .into_inner();
    ConfirmedPressure::from_response(ticket, response)
}

pub async fn inspect(
    ticket: &PressureTicket,
    caller: NodeId,
    token: Option<String>,
    budget: Duration,
) -> kv9_common::Result<ConfirmedPressure> {
    if budget.is_zero() || budget.as_micros() > u128::from(MAX_BUDGET_MICROS) {
        return Err(invalid());
    }
    let began = Instant::now();
    tokio::time::timeout(budget, async {
        let channel =
            tonic::transport::Endpoint::from_shared(format!("http://{}", ticket.route.address))
                .map_err(|_| invalid())?
                .connect_timeout(budget)
                .connect()
                .await
                .map_err(|_| invalid())?;
        let remaining = budget
            .checked_sub(began.elapsed())
            .filter(|d| !d.is_zero())
            .ok_or_else(invalid)?;
        inspect_on_channel(channel, ticket, caller, token, remaining).await
    })
    .await
    .map_err(|_| invalid())?
}

#[cfg(test)]
mod tests;
