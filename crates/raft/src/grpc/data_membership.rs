//! Independent exact-store local membership observations. Unknown/gapped
//! histories are not converted to empty/global membership certificates.
use super::data_read_fence::FenceTicket;
use super::*;
use crate::storage::membership_history::{MembershipCursor, MembershipPage, MAX_KNOWN_MEMBERS};
use kv9_common::{RegionId, StoreIdentity};
use prost::Message as _;
use std::time::Instant;

const MAX_BYTES: usize = 1024;
const MAX_MICROS: u64 = 2_000_000;
fn invalid() -> Error {
    Error::MetaNotReady("unconfirmed DATA membership observation".into())
}

#[derive(Debug, Clone)]
pub struct MembershipTicket {
    scope: FenceTicket,
    cursor: Option<MembershipCursor>,
    seen: usize,
}
impl MembershipTicket {
    /// Reconstruct a request from durable progress. This is request data, not
    /// a confirmed page or authority; the response must still be correlated.
    pub fn resume(
        identity: StoreIdentity,
        region: RegionId,
        creation: RootDigest,
        cursor: Option<MembershipCursor>,
        seen: usize,
    ) -> kv9_common::Result<Self> {
        if (seen == 0) != cursor.is_none() || seen > MAX_KNOWN_MEMBERS || !seen.is_multiple_of(4) {
            return Err(invalid());
        }
        Ok(Self {
            scope: FenceTicket::new(identity, region, creation)?,
            cursor,
            seen,
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
            seen: 0,
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
    pub fn cursor(&self) -> Option<MembershipCursor> {
        self.cursor
    }
    pub fn seen(&self) -> usize {
        self.seen
    }
    fn request(&self, budget: Duration) -> pb::DataMembershipRequest {
        pb::DataMembershipRequest {
            scope: Some(self.scope.request(budget)),
            cursor: self
                .cursor
                .map(MembershipCursor::encode)
                .unwrap_or_default(),
            seen: self.seen as u32,
            version: 1,
        }
    }
    fn decode(query: &pb::DataMembershipRequest) -> kv9_common::Result<Self> {
        let scope = query.scope.as_ref().ok_or_else(invalid)?;
        if query.encoded_len() > MAX_BYTES
            || query.version != 1
            || !(1..=MAX_MICROS).contains(&scope.budget_micros)
            || query.seen as usize > MAX_KNOWN_MEMBERS
            || !query.seen.is_multiple_of(4)
            || (query.seen == 0) != query.cursor.is_empty()
        {
            return Err(invalid());
        }
        Ok(Self {
            scope: FenceTicket::from_request(scope)?,
            cursor: (!query.cursor.is_empty())
                .then(|| MembershipCursor::decode(&query.cursor))
                .transpose()?,
            seen: query.seen as usize,
        })
    }
}
#[derive(Debug)]
pub struct ConfirmedMembershipPage {
    ticket: MembershipTicket,
    page: Option<MembershipPage>,
}
impl ConfirmedMembershipPage {
    pub fn page(&self) -> Option<&MembershipPage> {
        self.page.as_ref()
    }
    /// Unknown is an error, never an ordinary end-of-pagination result.
    pub fn next_ticket(&self) -> kv9_common::Result<Option<MembershipTicket>> {
        let page = self.page.as_ref().ok_or_else(invalid)?;
        let Some(cursor) = page.next else {
            return Ok(None);
        };
        let mut next = MembershipTicket::new(
            self.ticket.identity(),
            self.ticket.region(),
            self.ticket.creation(),
        )?;
        next.cursor = Some(cursor);
        next.seen = self.ticket.seen + page.members.len();
        Ok(Some(next))
    }
    fn decode(
        ticket: &MembershipTicket,
        query: &pb::DataMembershipRequest,
        reply: pb::DataMembershipResponse,
    ) -> kv9_common::Result<Self> {
        if reply.encoded_len() > MAX_BYTES || reply.query.as_ref() != Some(query) {
            return Err(invalid());
        }
        let page = match reply.observation {
            Some(pb::data_membership_response::Observation::Page(bytes)) => {
                let page = MembershipPage::decode(&bytes)?;
                page.validate_progress(ticket.cursor, ticket.seen)?;
                Some(page)
            }
            Some(pb::data_membership_response::Observation::Unknown(true)) => None,
            _ => return Err(invalid()),
        };
        Ok(Self {
            ticket: ticket.clone(),
            page,
        })
    }
}

impl RaftGrpcService {
    pub(super) async fn inspect_data_membership_request(
        &self,
        request: Request<pb::DataMembershipRequest>,
    ) -> std::result::Result<Response<pb::DataMembershipResponse>, Status> {
        let started = Instant::now();
        let caller = *request
            .extensions()
            .get::<NodeId>()
            .filter(|n| n.0 != 0)
            .ok_or_else(|| Status::unauthenticated("authenticated node identity missing"))?;
        let value = request.into_inner();
        let ticket = MembershipTicket::decode(&value)
            .map_err(|_| Status::invalid_argument("invalid membership query"))?;
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
            .ok_or_else(|| Status::unimplemented("membership backend unavailable"))?;
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
            let page = backend.data_membership(
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
                page.validate_progress(requested.cursor, requested.seen)?;
            }
            Ok(page)
        });
        let page = tokio::time::timeout(deadline.saturating_duration_since(Instant::now()), job)
            .await
            .map_err(|_| Status::deadline_exceeded("membership observation deadline elapsed"))?
            .map_err(|_| Status::unavailable("membership observation worker failed"))?
            .map_err(|_| Status::unavailable("membership observation remains unknown"))?;
        confirm()?;
        if Instant::now() >= deadline {
            return Err(Status::deadline_exceeded(
                "membership response deadline elapsed",
            ));
        }
        let observation = match page {
            Some(page) => pb::data_membership_response::Observation::Page(
                page.encode()
                    .map_err(|_| Status::unavailable("invalid membership page"))?,
            ),
            None => pb::data_membership_response::Observation::Unknown(true),
        };
        let reply = pb::DataMembershipResponse {
            query: Some(value),
            observation: Some(observation),
        };
        if reply.encoded_len() > MAX_BYTES {
            return Err(Status::resource_exhausted("membership response too large"));
        }
        Ok(Response::new(reply))
    }
}
pub async fn inspect_on_channel(
    channel: tonic::transport::Channel,
    ticket: &MembershipTicket,
    caller: NodeId,
    token: Option<String>,
    budget: Duration,
) -> kv9_common::Result<ConfirmedMembershipPage> {
    if caller.0 == 0 || budget.as_micros() == 0 || budget.as_micros() > u128::from(MAX_MICROS) {
        return Err(invalid());
    }
    let query = ticket.request(budget);
    MembershipTicket::decode(&query)?;
    let mut request = Request::new(query.clone());
    request.set_timeout(budget);
    attach_auth(&mut request, &token, caller);
    let mut client = Kv9RaftClient::new(channel)
        .max_encoding_message_size(MAX_BYTES)
        .max_decoding_message_size(MAX_BYTES);
    let reply = tokio::time::timeout(budget, client.inspect_data_membership(request))
        .await
        .map_err(|_| invalid())?
        .map_err(|_| invalid())?
        .into_inner();
    ConfirmedMembershipPage::decode(ticket, &query, reply)
}

#[cfg(test)]
pub(crate) mod tests;
