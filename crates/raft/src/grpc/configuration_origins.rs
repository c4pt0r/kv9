//! Independent exact-store local configuration origin observations. Unknown/gapped
//! histories are not converted to empty/global configuration origin certificates.
use super::data_read_fence::FenceTicket;
use super::*;
use crate::storage::configuration_origins::{OriginCursor, OriginPage};
use kv9_common::{RegionId, StoreIdentity};
use prost::Message as _;
use std::time::Instant;

const MAX_BYTES: usize = 2048;
const MAX_MICROS: u64 = 2_000_000;
fn invalid() -> Error {
    Error::MetaNotReady("unconfirmed DATA configuration origin observation".into())
}

#[derive(Debug, Clone)]
pub struct OriginTicket {
    scope: FenceTicket,
    cursor: Option<OriginCursor>,
}
impl OriginTicket {
    /// Reconstruct a request from durable progress. This is request data, not
    /// a confirmed page or authority; the response must still be correlated.
    pub fn resume(
        identity: StoreIdentity,
        region: RegionId,
        creation: RootDigest,
        cursor: Option<OriginCursor>,
    ) -> kv9_common::Result<Self> {
        Ok(Self {
            scope: FenceTicket::new(identity, region, creation)?,
            cursor,
        })
    }
    pub fn new(
        identity: StoreIdentity,
        region: RegionId,
        creation: RootDigest,
    ) -> kv9_common::Result<Self> {
        Ok(Self {
            scope: FenceTicket::new(identity, region, creation)?,
            cursor: None,
        })
    }
    pub fn identity(&self) -> StoreIdentity {
        self.scope.identity
    }
    pub fn region(&self) -> RegionId {
        self.scope.region
    }
    pub fn creation(&self) -> RootDigest {
        self.scope.creation
    }
    pub fn cursor(&self) -> Option<OriginCursor> {
        self.cursor
    }
    fn validate_page(&self, page: &OriginPage) -> kv9_common::Result<()> {
        page.validate_progress(self.cursor)?;
        if page.initial.is_none() {
            return Err(invalid());
        }
        if page.entries.iter().any(|e| {
            e.origin.root != self.identity().root_digest
                || e.origin.region != self.region()
                || e.origin.creation != self.creation()
        }) {
            return Err(invalid());
        }
        Ok(())
    }
    fn request(&self, budget: Duration) -> pb::ConfigurationOriginsRequest {
        pb::ConfigurationOriginsRequest {
            scope: Some(self.scope.request(budget)),
            cursor: self.cursor.map(OriginCursor::encode).unwrap_or_default(),
            version: 2,
        }
    }
    fn decode(query: &pb::ConfigurationOriginsRequest) -> kv9_common::Result<Self> {
        let scope = query.scope.as_ref().ok_or_else(invalid)?;
        if query.encoded_len() > MAX_BYTES
            || query.version != 2
            || !(1..=MAX_MICROS).contains(&scope.budget_micros)
        {
            return Err(invalid());
        }
        Ok(Self {
            scope: FenceTicket::from_request(scope)?,
            cursor: (!query.cursor.is_empty())
                .then(|| OriginCursor::decode(&query.cursor))
                .transpose()?,
        })
    }
}
#[derive(Debug)]
pub struct ConfirmedOriginPage {
    ticket: OriginTicket,
    page: Option<OriginPage>,
}
impl ConfirmedOriginPage {
    pub fn page(&self) -> Option<&OriginPage> {
        self.page.as_ref()
    }
    /// Unknown is an error, never an ordinary end-of-pagination result.
    pub fn next_ticket(&self) -> kv9_common::Result<Option<OriginTicket>> {
        let page = self.page.as_ref().ok_or_else(invalid)?;
        let Some(cursor) = page.next else {
            return Ok(None);
        };
        let mut next = OriginTicket::new(
            self.ticket.identity(),
            self.ticket.region(),
            self.ticket.creation(),
        )?;
        next.cursor = Some(cursor);
        Ok(Some(next))
    }
    fn decode(
        ticket: &OriginTicket,
        query: &pb::ConfigurationOriginsRequest,
        reply: pb::ConfigurationOriginsResponse,
    ) -> kv9_common::Result<Self> {
        if reply.encoded_len() > MAX_BYTES || reply.query.as_ref() != Some(query) {
            return Err(invalid());
        }
        let page = match reply.observation {
            Some(pb::configuration_origins_response::Observation::Page(bytes)) => {
                let page = OriginPage::decode(&bytes)?;
                ticket.validate_page(&page)?;
                Some(page)
            }
            Some(pb::configuration_origins_response::Observation::Unknown(true)) => None,
            _ => return Err(invalid()),
        };
        Ok(Self {
            ticket: ticket.clone(),
            page,
        })
    }
}

impl RaftGrpcService {
    pub(super) async fn inspect_configuration_origins_request(
        &self,
        request: Request<pb::ConfigurationOriginsRequest>,
    ) -> std::result::Result<Response<pb::ConfigurationOriginsResponse>, Status> {
        let started = Instant::now();
        let caller = *request
            .extensions()
            .get::<NodeId>()
            .filter(|n| n.0 != 0)
            .ok_or_else(|| Status::unauthenticated("authenticated node identity missing"))?;
        let value = request.into_inner();
        let ticket = OriginTicket::decode(&value)
            .map_err(|_| Status::invalid_argument("invalid configuration origin query"))?;
        let scope = value.scope.as_ref().expect("validated scope");
        let confirm = || {
            let mut q = Request::new(pb::RawTtlFormatRequest {
                root_digest: scope.root_digest.clone(),
                cluster_id: scope.cluster_id.clone(),
                node_id: scope.node_id,
                store_incarnation: scope.store_incarnation.clone(),
            });
            q.extensions_mut().insert(caller);
            self.confirm_raw_ttl_format_request(q)
        };
        confirm()?;
        let backend = self
            .registration
            .clone()
            .ok_or_else(|| Status::unimplemented("configuration origin backend unavailable"))?;
        let permit = self
            .checkpoint_history_capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("checkpoint observation reader busy"))?;
        let deadline = started + Duration::from_micros(scope.budget_micros);
        let requested = ticket.clone();
        let job = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            if Instant::now() >= deadline {
                return Err(invalid());
            }
            let page = backend.configuration_origins(
                requested.identity().root_digest,
                requested.region(),
                requested.creation(),
                requested.cursor,
                deadline,
            )?;
            if Instant::now() >= deadline {
                return Err(invalid());
            }
            if let Some(page) = &page {
                requested.validate_page(page)?;
            }
            Ok(page)
        });
        let page = tokio::time::timeout(deadline.saturating_duration_since(Instant::now()), job)
            .await
            .map_err(|_| {
                Status::deadline_exceeded("configuration origin observation deadline elapsed")
            })?
            .map_err(|_| Status::unavailable("configuration origin observation worker failed"))?
            .map_err(|_| Status::unavailable("configuration origin observation remains unknown"))?;
        confirm()?;
        if Instant::now() >= deadline {
            return Err(Status::deadline_exceeded(
                "configuration origin response deadline elapsed",
            ));
        }
        let observation = match page {
            Some(page) => pb::configuration_origins_response::Observation::Page(
                page.encode()
                    .map_err(|_| Status::unavailable("invalid configuration origin page"))?,
            ),
            None => pb::configuration_origins_response::Observation::Unknown(true),
        };
        let reply = pb::ConfigurationOriginsResponse {
            query: Some(value),
            observation: Some(observation),
        };
        if reply.encoded_len() > MAX_BYTES {
            return Err(Status::resource_exhausted(
                "configuration origin response too large",
            ));
        }
        Ok(Response::new(reply))
    }
}
pub async fn inspect_on_channel(
    channel: tonic::transport::Channel,
    ticket: &OriginTicket,
    caller: NodeId,
    token: Option<String>,
    budget: Duration,
) -> kv9_common::Result<ConfirmedOriginPage> {
    if caller.0 == 0 || budget.as_micros() == 0 || budget.as_micros() > u128::from(MAX_MICROS) {
        return Err(invalid());
    }
    let query = ticket.request(budget);
    OriginTicket::decode(&query)?;
    let mut request = Request::new(query.clone());
    request.set_timeout(budget);
    attach_auth(&mut request, &token, caller);
    let mut client = Kv9RaftClient::new(channel)
        .max_encoding_message_size(MAX_BYTES)
        .max_decoding_message_size(MAX_BYTES);
    let reply = tokio::time::timeout(budget, client.inspect_configuration_origins(request))
        .await
        .map_err(|_| invalid())?
        .map_err(|_| invalid())?
        .into_inner();
    ConfirmedOriginPage::decode(ticket, &query, reply)
}

#[cfg(test)]
mod tests;
