//! Authenticated, bounded metadata-ledger access for checkpoint workers.
//! The raft crate transports opaque canonical records; only the metadata
//! planner grants ownership. Neither transport success nor a leader hint does.
use super::*;
use kv9_common::{retention::OwnerId, AppliedPosition};
use prost::Message as _;
use std::time::Instant;

pub const MAX_COMMAND_BYTES: usize = kv9_engine::checkpoint::MAX_MANIFEST_BYTES + 256;
pub const MAX_OWNER_BYTES: usize = 8 * 1024;
pub const MAX_INTENT_BYTES: usize = kv9_engine::checkpoint::MAX_MANIFEST_BYTES + 100;
pub const MAX_EXECUTOR_PAGE_BYTES: usize = 16 * (MAX_OWNER_BYTES + 4) + 256;
pub const MAX_CAPTURE_BYTES: usize = crate::snapshot_install::capture::MAX_CAPTURE_DESCRIPTOR_BYTES;
const MAX_REQUEST_BYTES: usize = MAX_CAPTURE_BYTES + 128;
const MAX_RESPONSE_BYTES: usize = MAX_INTENT_BYTES + 128;
const MAX_BUDGET_MICROS: u64 = 30_000_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LedgerQuery {
    Apply(Vec<u8>),
    CapturePlan(Vec<u8>),
    Owner(OwnerId),
    Intent(OwnerId),
    Executors {
        pending: OwnerId,
        after: Option<OwnerId>,
        limit: usize,
    },
}
#[derive(Debug, PartialEq, Eq)]
pub enum LedgerReply {
    Applied {
        revision: u64,
        changed: bool,
        applied: AppliedPosition,
    },
    Owner(Option<Vec<u8>>),
    Intent(Option<Vec<u8>>),
    Executors(Vec<u8>),
}
fn invalid() -> Error {
    Error::MetaNotReady("checkpoint retention outcome unconfirmed".into())
}
impl LedgerQuery {
    pub fn validate(&self, root: RootDigest) -> kv9_common::Result<()> {
        if root.as_bytes() == &[0; 32]
            || matches!(self, Self::Apply(bytes) if bytes.is_empty() || bytes.len() > MAX_COMMAND_BYTES)
            || matches!(self, Self::CapturePlan(bytes) if bytes.is_empty() || bytes.len() > MAX_CAPTURE_BYTES)
            || matches!(self, Self::Executors { limit, .. } if !(1..=16).contains(limit))
        {
            return Err(invalid());
        }
        Ok(())
    }
    fn digest(&self, root: RootDigest) -> RootDigest {
        let mut bytes = b"kv9-checkpoint-retention-query-v1".to_vec();
        bytes.extend(root.as_bytes());
        match self {
            Self::Apply(command) => {
                bytes.push(1);
                bytes.extend(command);
            }
            Self::CapturePlan(record) => {
                bytes.push(5);
                bytes.extend(record);
            }
            Self::Owner(owner) => {
                bytes.push(2);
                bytes.extend(owner.as_bytes());
            }
            Self::Intent(owner) => {
                bytes.push(3);
                bytes.extend(owner.as_bytes());
            }
            Self::Executors {
                pending,
                after,
                limit,
            } => {
                bytes.push(4);
                bytes.extend(pending.as_bytes());
                bytes.push(u8::from(after.is_some()));
                if let Some(id) = after {
                    bytes.extend(id.as_bytes());
                }
                bytes.extend((*limit as u64).to_be_bytes());
            }
        }
        RootDigest::sha256(&bytes)
    }
    fn encode(&self) -> pb::checkpoint_retention_request::Query {
        match self {
            Self::Apply(bytes) => pb::checkpoint_retention_request::Query::Apply(bytes.clone()),
            Self::CapturePlan(bytes) => {
                pb::checkpoint_retention_request::Query::CapturePlan(bytes.clone())
            }
            Self::Owner(id) => {
                pb::checkpoint_retention_request::Query::OwnerId(id.as_bytes().to_vec())
            }
            Self::Intent(id) => {
                pb::checkpoint_retention_request::Query::IntentOwnerId(id.as_bytes().to_vec())
            }
            Self::Executors {
                pending,
                after,
                limit,
            } => pb::checkpoint_retention_request::Query::Executors(pb::CheckpointExecutorsQuery {
                pending_owner_id: pending.as_bytes().to_vec(),
                after_owner_id: after.map(|id| id.as_bytes().to_vec()),
                limit: *limit as u32,
            }),
        }
    }
}
impl LedgerReply {
    pub fn validate(&self, query: &LedgerQuery) -> kv9_common::Result<()> {
        match (query, self) {
            (
                LedgerQuery::Apply(_) | LedgerQuery::CapturePlan(_),
                Self::Applied { applied, .. },
            ) if applied.term > 0 && applied.index > 0 => Ok(()),
            (LedgerQuery::Owner(_), Self::Owner(bytes))
                if bytes
                    .as_ref()
                    .is_none_or(|b| !b.is_empty() && b.len() <= MAX_OWNER_BYTES) =>
            {
                Ok(())
            }
            (LedgerQuery::Intent(_), Self::Intent(bytes))
                if bytes
                    .as_ref()
                    .is_none_or(|b| !b.is_empty() && b.len() <= MAX_INTENT_BYTES) =>
            {
                Ok(())
            }
            (LedgerQuery::Executors { .. }, Self::Executors(bytes))
                if !bytes.is_empty() && bytes.len() <= MAX_EXECUTOR_PAGE_BYTES =>
            {
                Ok(())
            }
            _ => Err(invalid()),
        }
    }
    fn encode(self, root: RootDigest, query: &LedgerQuery) -> pb::CheckpointRetentionResponse {
        use pb::checkpoint_retention_response::Reply;
        let reply = match self {
            Self::Applied {
                revision,
                changed,
                applied,
            } => Reply::Applied(pb::CheckpointRetentionApplied {
                revision,
                changed,
                term: applied.term,
                index: applied.index,
            }),
            Self::Owner(observation) => Reply::Owner(pb::CheckpointRetentionOwner { observation }),
            Self::Intent(observation) => {
                Reply::Intent(pb::CheckpointRetentionOwner { observation })
            }
            Self::Executors(bytes) => Reply::Executors(pb::CheckpointRetentionOwner {
                observation: Some(bytes),
            }),
        };
        pb::CheckpointRetentionResponse {
            root_digest: root.as_bytes().to_vec(),
            query_digest: query.digest(root).as_bytes().to_vec(),
            reply: Some(reply),
        }
    }
    fn decode(
        value: pb::CheckpointRetentionResponse,
        root: RootDigest,
        query: &LedgerQuery,
    ) -> kv9_common::Result<Self> {
        if value.root_digest.as_slice() != root.as_bytes()
            || value.query_digest.as_slice() != query.digest(root).as_bytes()
            || value.encoded_len() > MAX_RESPONSE_BYTES
        {
            return Err(invalid());
        }
        use pb::checkpoint_retention_response::Reply;
        let reply = match value.reply.ok_or_else(invalid)? {
            Reply::Applied(at) => Self::Applied {
                revision: at.revision,
                changed: at.changed,
                applied: AppliedPosition {
                    term: at.term,
                    index: at.index,
                },
            },
            Reply::Owner(owner) => Self::Owner(owner.observation),
            Reply::Intent(intent) => Self::Intent(intent.observation),
            Reply::Executors(page) => Self::Executors(page.observation.ok_or_else(invalid)?),
        };
        reply.validate(query)?;
        Ok(reply)
    }
}
fn status(error: Error) -> Status {
    // Only the exact typed pre-append / provably-replaced refusal permits the
    // caller to walk to another node. All ambiguous backend failures are final.
    if let Error::NotLeader { leader } = error {
        let mut status = Status::failed_precondition("not the metadata leader");
        status
            .metadata_mut()
            .insert(NOT_LEADER_KEY, "true".parse().unwrap());
        if let Some(node) = leader.filter(|n| n.0 != 0) {
            status
                .metadata_mut()
                .insert(LEADER_NODE_ID_KEY, node.0.to_string().parse().unwrap());
        }
        status
    } else {
        Status::unavailable("checkpoint retention outcome unconfirmed")
    }
}
fn decode_status(status: Status) -> Error {
    if status.code() == tonic::Code::FailedPrecondition
        && status
            .metadata()
            .get(NOT_LEADER_KEY)
            .and_then(|v| v.to_str().ok())
            == Some("true")
    {
        let leader = match status.metadata().get(LEADER_NODE_ID_KEY) {
            None => None,
            Some(value) => match value
                .to_str()
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .filter(|n| *n > 0)
            {
                Some(n) => Some(NodeId(n)),
                None => return invalid(),
            },
        };
        Error::NotLeader { leader }
    } else {
        invalid()
    }
}
impl RaftGrpcService {
    pub(super) async fn checkpoint_retention_request(
        &self,
        request: Request<pb::CheckpointRetentionRequest>,
    ) -> std::result::Result<Response<pb::CheckpointRetentionResponse>, Status> {
        let started = Instant::now();
        request
            .extensions()
            .get::<NodeId>()
            .filter(|n| n.0 != 0)
            .ok_or_else(|| Status::unauthenticated("authenticated node identity missing"))?;
        if !self.discovery.raft_receive_allowed() {
            return Err(Status::failed_precondition("replica authority unavailable"));
        }
        let request = request.into_inner();
        if request.encoded_len() > MAX_REQUEST_BYTES
            || !(1..=MAX_BUDGET_MICROS).contains(&request.budget_micros)
        {
            return Err(Status::invalid_argument(
                "invalid checkpoint retention bounds",
            ));
        }
        let root = RootDigest::from_bytes(
            request
                .root_digest
                .try_into()
                .map_err(|_| Status::invalid_argument("invalid checkpoint retention root"))?,
        );
        if root != self.discovery.root_identity().root_digest {
            return Err(Status::failed_precondition(
                "checkpoint retention root differs",
            ));
        }
        use pb::checkpoint_retention_request::Query;
        let query = match request.query {
            Some(Query::Apply(bytes)) => LedgerQuery::Apply(bytes),
            Some(Query::CapturePlan(bytes)) => LedgerQuery::CapturePlan(bytes),
            Some(Query::OwnerId(bytes)) => LedgerQuery::Owner(
                OwnerId::new(
                    bytes
                        .try_into()
                        .map_err(|_| Status::invalid_argument("invalid checkpoint owner"))?,
                )
                .map_err(|_| Status::invalid_argument("invalid checkpoint owner"))?,
            ),
            Some(Query::IntentOwnerId(bytes)) => LedgerQuery::Intent(
                OwnerId::new(
                    bytes
                        .try_into()
                        .map_err(|_| Status::invalid_argument("invalid intent owner"))?,
                )
                .map_err(|_| Status::invalid_argument("invalid intent owner"))?,
            ),
            Some(Query::Executors(page)) => {
                let id = |bytes: Vec<u8>| {
                    OwnerId::new(
                        bytes
                            .try_into()
                            .map_err(|_| Status::invalid_argument("invalid executor cursor"))?,
                    )
                    .map_err(|_| Status::invalid_argument("invalid executor cursor"))
                };
                LedgerQuery::Executors {
                    pending: id(page.pending_owner_id)?,
                    after: page.after_owner_id.map(id).transpose()?,
                    limit: page.limit as usize,
                }
            }
            None => {
                return Err(Status::invalid_argument(
                    "checkpoint retention query missing",
                ))
            }
        };
        query
            .validate(root)
            .map_err(|_| Status::invalid_argument("invalid checkpoint retention query"))?;
        let backend = self
            .registration
            .clone()
            .ok_or_else(|| Status::unimplemented("checkpoint retention backend unavailable"))?;
        let permit = self
            .checkpoint_retention_capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("checkpoint retention capacity exhausted"))?;
        let deadline = started + Duration::from_micros(request.budget_micros);
        let response = tokio::task::spawn_blocking(move || {
            let _permit = permit; // Retained even if the RPC future is cancelled.
            if Instant::now() >= deadline {
                return Err(invalid());
            }
            let reply = backend.checkpoint_retention(root, &query, deadline)?;
            reply.validate(&query)?;
            if Instant::now() >= deadline {
                return Err(invalid());
            }
            Ok(reply.encode(root, &query))
        })
        .await
        .map_err(|_| Status::internal("checkpoint retention worker failed"))?
        .map_err(status)?;
        Ok(Response::new(response))
    }
}

/// One call only. The caller may reroute a typed NotLeader refusal, but never
/// retries a deadline, cancellation, malformed receipt or lost response here.
pub async fn on_channel(
    channel: tonic::transport::Channel,
    root: RootDigest,
    query: &LedgerQuery,
    caller: NodeId,
    token: Option<String>,
    budget: Duration,
) -> kv9_common::Result<LedgerReply> {
    query.validate(root)?;
    if caller.0 == 0 || budget.as_micros() == 0 || budget > Duration::from_secs(30) {
        return Err(invalid());
    }
    let mut client = Kv9RaftClient::new(channel)
        .max_encoding_message_size(MAX_REQUEST_BYTES)
        .max_decoding_message_size(MAX_RESPONSE_BYTES);
    let mut request = Request::new(pb::CheckpointRetentionRequest {
        root_digest: root.as_bytes().to_vec(),
        budget_micros: budget.as_micros() as u64,
        query: Some(query.encode()),
    });
    request.set_timeout(budget);
    attach_auth(&mut request, &token, caller);
    let reply = tokio::time::timeout(budget, client.checkpoint_retention(request))
        .await
        .map_err(|_| invalid())?
        .map_err(decode_status)?
        .into_inner();
    LedgerReply::decode(reply, root, query)
}

#[cfg(test)]
mod tests;
