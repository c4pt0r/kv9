//! Bounded administrative client for durable empty-group creation requests.
//! No implicit retries: an unconfirmed mutation keeps its operation identity.

use crate::{api::CreateDataGroupResult, grpc::proto};
use kv9_common::{AppliedPosition, NodeId, RootDigest};
use kv9_meta::data_groups::CreationIntent;
use std::time::Duration;
use tonic::{Request, Status};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataGroupRpcError {
    Local(String),
    NotLeader {
        leader: Option<NodeId>,
    },
    AdmissionRefused {
        reason: &'static str,
    },
    /// The mutation might have committed. Reuse the exact operation on retry.
    Unconfirmed(String),
}
impl std::fmt::Display for DataGroupRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "data group request: {self:?}")
    }
}
impl std::error::Error for DataGroupRpcError {}

fn rpc_error(status: Status) -> DataGroupRpcError {
    if let crate::client::Reason::NotLeader { leader } =
        crate::client::classify_status(&status, false)
    {
        DataGroupRpcError::NotLeader {
            leader: leader.map(NodeId),
        }
    } else if let Some(reason) = crate::grpc::admission_refusal(&status) {
        DataGroupRpcError::AdmissionRefused { reason }
    } else {
        DataGroupRpcError::Unconfirmed(status.to_string())
    }
}

pub struct DataGroupClient {
    client: proto::kv9_client::Kv9Client<tonic::transport::Channel>,
    authorization: tonic::metadata::MetadataValue<tonic::metadata::Ascii>,
    runtime: tokio::runtime::Runtime,
}
impl DataGroupClient {
    pub fn connect(address: &str, token: &str) -> Result<Self, DataGroupRpcError> {
        let address = crate::endpoints::socket(address)
            .map_err(|e| DataGroupRpcError::Local(e.to_string()))?;
        let authorization = format!("Bearer {token}")
            .parse()
            .map_err(|_| DataGroupRpcError::Local("invalid bearer metadata".into()))?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| DataGroupRpcError::Local(e.to_string()))?;
        let endpoint = tonic::transport::Endpoint::from_shared(format!("http://{address}"))
            .map_err(|e| DataGroupRpcError::Local(e.to_string()))?
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(30));
        let channel = runtime
            .block_on(endpoint.connect())
            .map_err(|e| DataGroupRpcError::Unconfirmed(e.to_string()))?;
        Ok(Self {
            client: proto::kv9_client::Kv9Client::new(channel),
            authorization,
            runtime,
        })
    }

    pub fn create(
        &mut self,
        root: RootDigest,
        operation: [u8; 16],
        voters: &[NodeId],
    ) -> Result<CreateDataGroupResult, DataGroupRpcError> {
        let mut voters = voters.to_vec();
        voters.sort();
        if root.as_bytes() == &[0; 32]
            || operation == [0; 16]
            || ![3, 5, 7].contains(&voters.len())
            || voters.iter().any(|n| n.0 == 0)
            || voters.windows(2).any(|v| v[0] == v[1])
        {
            return Err(DataGroupRpcError::Local(
                "nonzero identities and 3, 5 or 7 distinct voters required".into(),
            ));
        }
        let mut request = Request::new(proto::CreateDataGroupRequest {
            root_digest: root.as_bytes().to_vec(),
            operation_id: operation.to_vec(),
            voters: voters.iter().map(|n| n.0).collect(),
        });
        request
            .metadata_mut()
            .insert("authorization", self.authorization.clone());
        request.set_timeout(Duration::from_secs(30));
        let response = self
            .runtime
            .block_on(self.client.create_data_group(request))
            .map_err(rpc_error)?
            .into_inner();
        let intent = CreationIntent::decode(&response.creation_intent)
            .map_err(|e| DataGroupRpcError::Unconfirmed(e.to_string()))?;
        if intent.root() != root
            || intent.operation() != operation
            || intent.replicas().iter().map(|r| r.node).collect::<Vec<_>>() != voters
            || response.applied_term == 0
            || response.applied_index == 0
        {
            return Err(DataGroupRpcError::Unconfirmed(
                "response differs from request or lacks an exact applied receipt".into(),
            ));
        }
        Ok(CreateDataGroupResult {
            intent,
            changed: response.changed,
            applied: AppliedPosition {
                term: response.applied_term,
                index: response.applied_index,
            },
        })
    }
}
