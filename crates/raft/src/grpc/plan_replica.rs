//! Replica-plan delivery on the node-authenticated service (docs/REPLICA-REPAIR.md §2):
//! a data-group leader that decided a repair asks the metadata leader to commit the
//! move plan through the planner `AddReplica` uses. No authority is added: the
//! planner's rules (committed member as source, active outsider as destination, one
//! live plan per group) decide.

use std::time::{Duration, Instant};

use kv9_common::{Error, NodeId, RootDigest};
use tonic::{Request, Response, Status};

use super::{attach_auth, pb, RaftGrpcService};
use pb::kv9_raft_client::Kv9RaftClient;

const MAX_BUDGET: Duration = Duration::from_secs(30);

impl RaftGrpcService {
    pub(super) async fn plan_replica_request(
        &self,
        request: Request<pb::PlanReplicaRequest>,
    ) -> std::result::Result<Response<pb::PlanReplicaReceipt>, Status> {
        let _caller = *request
            .extensions()
            .get::<NodeId>()
            .filter(|n| n.0 != 0)
            .ok_or_else(|| Status::unauthenticated("authenticated node identity missing"))?;
        let value = request.into_inner();
        if value.root_digest.len() != 32
            || value.operation_id.len() != 16
            || value.operation_id == [0; 16]
            || value.creation_task == 0
            || value.destination_node == 0
            || !matches!(value.mode.as_str(), "add" | "move")
        {
            return Err(Status::invalid_argument("invalid replica plan bounds"));
        }
        let root = RootDigest::from_bytes(value.root_digest.as_slice().try_into().expect("length checked"));
        let operation: [u8; 16] = value.operation_id.as_slice().try_into().expect("length checked");
        let backend = self
            .registration
            .clone()
            .ok_or_else(|| Status::unimplemented("replica plan backend unavailable"))?;
        let permit = self
            .registration_capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("a replica plan commit is already in progress"))?;
        let deadline = Instant::now() + MAX_BUDGET;
        let outcome = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            backend.plan_replica(
                root,
                operation,
                value.creation_task,
                NodeId(value.destination_node),
                &value.mode,
                (value.source_node != 0).then_some(NodeId(value.source_node)),
                deadline,
            )
        })
        .await
        .map_err(|_| Status::internal("replica plan task failed"))?;
        match outcome {
            Ok((plan_task, migration_task, changed)) => Ok(Response::new(pb::PlanReplicaReceipt {
                plan_task,
                migration_task,
                changed,
            })),
            Err(Error::NotLeader { .. }) => Err(Status::failed_precondition("not the metadata leader")),
            Err(Error::NotImplemented(what)) => Err(Status::unimplemented(what)),
            Err(error) => Err(Status::invalid_argument(error.to_string())),
        }
    }
}

/// Deliver a plan to the node behind `channel`; `NotLeader` means try the next
/// metadata candidate, any other error is the planner's typed refusal.
#[allow(clippy::too_many_arguments)]
pub async fn deliver_on_channel(
    channel: tonic::transport::Channel,
    root: RootDigest,
    operation: [u8; 16],
    creation_task: u64,
    destination: NodeId,
    mode: &str,
    source: Option<NodeId>,
    caller: NodeId,
    token: Option<String>,
    budget: Duration,
) -> kv9_common::Result<(u64, u64, bool)> {
    if caller.0 == 0 || budget.is_zero() || budget > MAX_BUDGET || operation == [0; 16] {
        return Err(Error::Config("invalid replica plan delivery".into()));
    }
    let mut client = Kv9RaftClient::new(channel);
    let mut request = Request::new(pb::PlanReplicaRequest {
        root_digest: root.as_bytes().to_vec(),
        operation_id: operation.to_vec(),
        creation_task,
        destination_node: destination.0,
        mode: mode.to_owned(),
        source_node: source.map_or(0, |n| n.0),
    });
    attach_auth(&mut request, &token, caller);
    let reply = tokio::time::timeout(budget, client.plan_replica(request))
        .await
        .map_err(|_| Error::MetaNotReady("replica plan delivery timed out".into()))?;
    match reply {
        Ok(reply) => {
            let reply = reply.into_inner();
            Ok((reply.plan_task, reply.migration_task, reply.changed))
        }
        Err(status) if status.code() == tonic::Code::FailedPrecondition => Err(Error::NotLeader { leader: None }),
        Err(status) if status.code() == tonic::Code::Unavailable => {
            Err(Error::MetaNotReady(format!("replica plan delivery: {status}")))
        }
        Err(status) => Err(Error::Config(format!("replica plan refused: {}", status.message()))),
    }
}
