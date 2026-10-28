//! Independent exact-store local configuration timeline observations. Unknown/gapped
//! histories are not converted to empty/global configuration timeline certificates.
use super::data_read_fence::FenceTicket;
use super::*;
use crate::storage::configuration_timeline::{TimelineCursor, TimelinePage};
use kv9_common::{RegionId, StoreIdentity};
use prost::Message as _;
use std::time::Instant;

const MAX_BYTES: usize = crate::storage::configuration_timeline::MAX_PAGE_BYTES + 2048;
const MAX_MICROS: u64 = 2_000_000;
fn invalid() -> Error {
    Error::MetaNotReady("unconfirmed DATA configuration timeline observation".into())
}

#[derive(Debug, Clone)]
pub struct TimelineTicket {
    scope: FenceTicket,
    cursor: Option<TimelineCursor>,
}
impl TimelineTicket {
    /// Reconstruct a request from durable progress. This is request data, not
    /// a confirmed page or authority; the response must still be correlated.
    pub fn resume(
        identity: StoreIdentity,
        region: RegionId,
        creation: RootDigest,
        cursor: Option<TimelineCursor>,
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
    pub fn cursor(&self) -> Option<TimelineCursor> {
        self.cursor
    }
    fn validate_page(&self, page: &TimelinePage) -> kv9_common::Result<()> {
        page.validate_progress(self.cursor)?;
        if page
            .records
            .iter()
            .filter_map(|e| e.origin.as_ref())
            .any(|e| {
                e.origin.root != self.identity().root_digest
                    || e.origin.region != self.region()
                    || e.origin.creation != self.creation()
            })
        {
            return Err(invalid());
        }
        Ok(())
    }
    fn request(&self, budget: Duration) -> pb::ConfigurationTimelineRequest {
        pb::ConfigurationTimelineRequest {
            scope: Some(self.scope.request(budget)),
            cursor: self.cursor.map(TimelineCursor::encode).unwrap_or_default(),
            version: 1,
        }
    }
    fn decode(query: &pb::ConfigurationTimelineRequest) -> kv9_common::Result<Self> {
        let scope = query.scope.as_ref().ok_or_else(invalid)?;
        if query.encoded_len() > 2048
            || query.version != 1
            || !(1..=MAX_MICROS).contains(&scope.budget_micros)
        {
            return Err(invalid());
        }
        Ok(Self {
            scope: FenceTicket::from_request(scope)?,
            cursor: (!query.cursor.is_empty())
                .then(|| TimelineCursor::decode(&query.cursor))
                .transpose()?,
        })
    }
}
#[derive(Debug)]
pub struct ConfirmedTimelinePage {
    ticket: TimelineTicket,
    page: Option<TimelinePage>,
}
impl ConfirmedTimelinePage {
    pub fn page(&self) -> Option<&TimelinePage> {
        self.page.as_ref()
    }
    /// Unknown is an error, never an ordinary end-of-pagination result.
    pub fn next_ticket(&self) -> kv9_common::Result<Option<TimelineTicket>> {
        let page = self.page.as_ref().ok_or_else(invalid)?;
        let Some(cursor) = page.next else {
            return Ok(None);
        };
        let mut next = TimelineTicket::new(
            self.ticket.identity(),
            self.ticket.region(),
            self.ticket.creation(),
        )?;
        next.cursor = Some(cursor);
        Ok(Some(next))
    }
    fn decode(
        ticket: &TimelineTicket,
        query: &pb::ConfigurationTimelineRequest,
        reply: pb::ConfigurationTimelineResponse,
    ) -> kv9_common::Result<Self> {
        if reply.encoded_len() > MAX_BYTES || reply.query.as_ref() != Some(query) {
            return Err(invalid());
        }
        let page = match reply.observation {
            Some(pb::configuration_timeline_response::Observation::Page(bytes)) => {
                let page = TimelinePage::decode(&bytes)?;
                ticket.validate_page(&page)?;
                Some(page)
            }
            Some(pb::configuration_timeline_response::Observation::Unknown(true)) => None,
            _ => return Err(invalid()),
        };
        Ok(Self {
            ticket: ticket.clone(),
            page,
        })
    }
}

impl RaftGrpcService {
    pub(super) async fn inspect_configuration_timeline_request(
        &self,
        request: Request<pb::ConfigurationTimelineRequest>,
    ) -> std::result::Result<Response<pb::ConfigurationTimelineResponse>, Status> {
        let started = Instant::now();
        let caller = *request
            .extensions()
            .get::<NodeId>()
            .filter(|n| n.0 != 0)
            .ok_or_else(|| Status::unauthenticated("authenticated node identity missing"))?;
        let value = request.into_inner();
        let ticket = TimelineTicket::decode(&value)
            .map_err(|_| Status::invalid_argument("invalid configuration timeline query"))?;
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
            .ok_or_else(|| Status::unimplemented("configuration timeline backend unavailable"))?;
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
            let page = backend.configuration_timeline(
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
                Status::deadline_exceeded("configuration timeline observation deadline elapsed")
            })?
            .map_err(|_| Status::unavailable("configuration timeline observation worker failed"))?
            .map_err(|_| {
                Status::unavailable("configuration timeline observation remains unknown")
            })?;
        confirm()?;
        if Instant::now() >= deadline {
            return Err(Status::deadline_exceeded(
                "configuration timeline response deadline elapsed",
            ));
        }
        let observation = match page {
            Some(page) => pb::configuration_timeline_response::Observation::Page(
                page.encode()
                    .map_err(|_| Status::unavailable("invalid configuration timeline page"))?,
            ),
            None => pb::configuration_timeline_response::Observation::Unknown(true),
        };
        let reply = pb::ConfigurationTimelineResponse {
            query: Some(value),
            observation: Some(observation),
        };
        if reply.encoded_len() > MAX_BYTES {
            return Err(Status::resource_exhausted(
                "configuration timeline response too large",
            ));
        }
        Ok(Response::new(reply))
    }
}
pub async fn inspect_on_channel(
    channel: tonic::transport::Channel,
    ticket: &TimelineTicket,
    caller: NodeId,
    token: Option<String>,
    budget: Duration,
) -> kv9_common::Result<ConfirmedTimelinePage> {
    if caller.0 == 0 || budget.as_micros() == 0 || budget.as_micros() > u128::from(MAX_MICROS) {
        return Err(invalid());
    }
    let query = ticket.request(budget);
    TimelineTicket::decode(&query)?;
    let mut request = Request::new(query.clone());
    request.set_timeout(budget);
    attach_auth(&mut request, &token, caller);
    let mut client = Kv9RaftClient::new(channel)
        .max_encoding_message_size(MAX_BYTES)
        .max_decoding_message_size(MAX_BYTES);
    let reply = tokio::time::timeout(budget, client.inspect_configuration_timeline(request))
        .await
        .map_err(|_| invalid())?
        .map_err(|_| invalid())?
        .into_inner();
    ConfirmedTimelinePage::decode(ticket, &query, reply)
}

#[cfg(test)]
mod tests;
