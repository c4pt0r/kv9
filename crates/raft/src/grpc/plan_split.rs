//! Split-intent delivery on the node-authenticated service (docs/RANGE-SPLIT.md §11):
//! a data-group leader whose automatic trigger fired asks the metadata leader to
//! commit the split intent through the planner `SplitRange` uses. No authority is
//! added: the planner's rules (bound unsealed parent, key strictly inside, one live
//! intent per parent) decide.

use std::time::{Duration, Instant};

use kv9_common::{Error, NodeId, RootDigest};
use tonic::{Request, Response, Status};

use super::{attach_auth, pb, RaftGrpcService};
use pb::kv9_raft_client::Kv9RaftClient;

const MAX_BUDGET: Duration = Duration::from_secs(30);

impl RaftGrpcService {
    pub(super) async fn plan_split_request(
        &self,
        request: Request<pb::PlanSplitRequest>,
    ) -> std::result::Result<Response<pb::PlanSplitReceipt>, Status> {
        let _caller = *request
            .extensions()
            .get::<NodeId>()
            .filter(|n| n.0 != 0)
            .ok_or_else(|| Status::unauthenticated("authenticated node identity missing"))?;
        let value = request.into_inner();
        if value.root_digest.len() != 32
            || value.operation_id.len() != 16
            || value.operation_id == [0; 16]
            || value.parent_region == 0
            || value.split_key.is_empty()
            || value.split_key.len() > 1024
        {
            return Err(Status::invalid_argument("invalid split intent bounds"));
        }
        let root = RootDigest::from_bytes(value.root_digest.as_slice().try_into().expect("length checked"));
        let operation: [u8; 16] = value.operation_id.as_slice().try_into().expect("length checked");
        let backend = self
            .registration
            .clone()
            .ok_or_else(|| Status::unimplemented("split intent backend unavailable"))?;
        let permit = self
            .registration_capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("a split intent commit is already in progress"))?;
        let deadline = Instant::now() + MAX_BUDGET;
        let outcome = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            backend.plan_split(
                root,
                operation,
                kv9_common::RegionId(value.parent_region),
                &value.split_key,
                deadline,
            )
        })
        .await
        .map_err(|_| Status::internal("split intent task failed"))?;
        match outcome {
            Ok((intent_task, child_low_task, child_high_task, changed)) => Ok(Response::new(pb::PlanSplitReceipt {
                intent_task,
                child_low_task,
                child_high_task,
                changed,
            })),
            Err(Error::NotLeader { .. }) => Err(Status::failed_precondition("not the metadata leader")),
            Err(Error::NotImplemented(what)) => Err(Status::unimplemented(what)),
            Err(error) => Err(Status::invalid_argument(error.to_string())),
        }
    }
}

/// Deliver a split intent to the node behind `channel`; `NotLeader` means try the
/// next metadata candidate, any other error is the planner's typed refusal.
#[allow(clippy::too_many_arguments)]
pub async fn deliver_on_channel(
    channel: tonic::transport::Channel,
    root: RootDigest,
    operation: [u8; 16],
    parent_region: kv9_common::RegionId,
    split_key: &[u8],
    caller: NodeId,
    token: Option<String>,
    budget: Duration,
) -> kv9_common::Result<(u64, u64, u64, bool)> {
    if caller.0 == 0 || budget.is_zero() || budget > MAX_BUDGET || operation == [0; 16] || split_key.is_empty() {
        return Err(Error::Config("invalid split intent delivery".into()));
    }
    let mut client = Kv9RaftClient::new(channel);
    let mut request = Request::new(pb::PlanSplitRequest {
        root_digest: root.as_bytes().to_vec(),
        operation_id: operation.to_vec(),
        parent_region: parent_region.0,
        split_key: split_key.to_vec(),
    });
    attach_auth(&mut request, &token, caller);
    let reply = tokio::time::timeout(budget, client.plan_split(request))
        .await
        .map_err(|_| Error::MetaNotReady("split intent delivery timed out".into()))?;
    match reply {
        Ok(reply) => {
            let reply = reply.into_inner();
            Ok((reply.intent_task, reply.child_low_task, reply.child_high_task, reply.changed))
        }
        Err(status) if status.code() == tonic::Code::FailedPrecondition => Err(Error::NotLeader { leader: None }),
        Err(status) if status.code() == tonic::Code::Unavailable => {
            Err(Error::MetaNotReady(format!("split intent delivery: {status}")))
        }
        Err(status) => Err(Error::Config(format!("split intent refused: {}", status.message()))),
    }
}
