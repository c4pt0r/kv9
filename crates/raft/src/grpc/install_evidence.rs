//! Install-evidence delivery on the node-authenticated service: a migration
//! destination hands its durable KV9EVD01 receipt to the metadata leader, which
//! commits the kind-104 row through the same planner the public verb uses
//! (docs/REPLICA-ADDITION.md §5). Nothing here grants a capability: a receipt
//! whose subject differs from the published pin is refused by that planner.

use std::time::{Duration, Instant};

use kv9_common::{AppliedPosition, Error, NodeId, RootDigest};
use tonic::{Request, Response, Status};

use super::{attach_auth, pb, RaftGrpcService};
use pb::kv9_raft_client::Kv9RaftClient;

/// A canonical receipt is a few hundred bytes; anything larger is not one.
pub const MAX_RECEIPT_BYTES: usize = 4096;
const MAX_BUDGET: Duration = Duration::from_secs(30);

impl RaftGrpcService {
    pub(super) async fn record_install_evidence_request(
        &self,
        request: Request<pb::InstallEvidenceRequest>,
    ) -> std::result::Result<Response<pb::InstallEvidenceReceipt>, Status> {
        let caller = *request
            .extensions()
            .get::<NodeId>()
            .filter(|n| n.0 != 0)
            .ok_or_else(|| Status::unauthenticated("authenticated node identity missing"))?;
        let value = request.into_inner();
        if value.root_digest.len() != 32 || value.receipt.is_empty() || value.receipt.len() > MAX_RECEIPT_BYTES {
            return Err(Status::invalid_argument("invalid install evidence bounds"));
        }
        let root = RootDigest::from_bytes(value.root_digest.as_slice().try_into().expect("length checked"));
        let backend = self
            .registration
            .clone()
            .ok_or_else(|| Status::unimplemented("install evidence backend unavailable"))?;
        let permit = self
            .registration_capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("install evidence commit is already in progress"))?;
        let deadline = Instant::now() + MAX_BUDGET;
        let _ = caller;
        let outcome = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            backend.record_install_evidence(root, &value.receipt, deadline)
        })
        .await
        .map_err(|_| Status::internal("install evidence task failed"))?;
        match outcome {
            Ok((task, changed, applied)) => Ok(Response::new(pb::InstallEvidenceReceipt {
                evidence_task: task,
                changed,
                applied_term: applied.term,
                applied_index: applied.index,
            })),
            // The caller walks the metadata candidates on this code and no other.
            Err(Error::NotLeader { .. }) => Err(Status::failed_precondition("not the metadata leader")),
            Err(Error::NotImplemented(what)) => Err(Status::unimplemented(what)),
            Err(error) => Err(Status::invalid_argument(error.to_string())),
        }
    }
}

/// Deliver `receipt` to the node behind `channel`. `NotLeader` means try the next
/// metadata candidate; any other error is the leader's typed refusal.
pub async fn deliver_on_channel(
    channel: tonic::transport::Channel,
    root: RootDigest,
    receipt: &[u8],
    caller: NodeId,
    token: Option<String>,
    budget: Duration,
) -> kv9_common::Result<(u64, bool, AppliedPosition)> {
    if caller.0 == 0 || budget.is_zero() || budget > MAX_BUDGET || receipt.is_empty() || receipt.len() > MAX_RECEIPT_BYTES {
        return Err(Error::Config("invalid install evidence delivery".into()));
    }
    let mut client = Kv9RaftClient::new(channel);
    let mut request = Request::new(pb::InstallEvidenceRequest {
        root_digest: root.as_bytes().to_vec(),
        receipt: receipt.to_vec(),
    });
    attach_auth(&mut request, &token, caller);
    let reply = tokio::time::timeout(budget, client.record_install_evidence(request))
        .await
        .map_err(|_| Error::MetaNotReady("install evidence delivery timed out".into()))?;
    match reply {
        Ok(reply) => {
            let reply = reply.into_inner();
            Ok((
                reply.evidence_task,
                reply.changed,
                AppliedPosition { term: reply.applied_term, index: reply.applied_index },
            ))
        }
        Err(status) if status.code() == tonic::Code::FailedPrecondition => {
            Err(Error::NotLeader { leader: None })
        }
        Err(status) if status.code() == tonic::Code::Unavailable => {
            Err(Error::MetaNotReady(format!("install evidence delivery: {status}")))
        }
        Err(status) => Err(Error::Config(format!("install evidence refused: {}", status.message()))),
    }
}
