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
    pub fn create_keyspace(
        &mut self,
        root: RootDigest,
        creation_task: u64,
        name: &str,
        tenant: kv9_common::TenantId,
    ) -> Result<crate::api::CreateDataKeyspaceResult, DataGroupRpcError> {
        if root.as_bytes() == &[0; 32]
            || creation_task < 100
            || name.is_empty()
            || name.len() > 1024
        {
            return Err(DataGroupRpcError::Local(
                "invalid data keyspace request".into(),
            ));
        }
        let mut request = Request::new(proto::CreateDataKeyspaceRequest {
            root_digest: root.as_bytes().to_vec(),
            creation_task,
            name: name.to_string(),
            tenant_id: tenant.0,
        });
        request
            .metadata_mut()
            .insert("authorization", self.authorization.clone());
        request.set_timeout(Duration::from_secs(30));
        let response = self
            .runtime
            .block_on(self.client.create_data_keyspace(request))
            .map_err(rpc_error)?
            .into_inner();
        let range = kv9_common::data_range::DataRange::decode(&response.binding)
            .map_err(|e| DataGroupRpcError::Unconfirmed(e.to_string()))?;
        if range.root != root
            || range.tenant != tenant
            || range.sealed
            || range.version != 1
            || range.conf_ver != 1
            || !range.start.is_empty()
            || !range.end.is_empty()
            || response.applied_term == 0
            || response.applied_index == 0
        {
            return Err(DataGroupRpcError::Unconfirmed(
                "data keyspace response lacks exact initial binding/receipt".into(),
            ));
        }
        Ok(crate::api::CreateDataKeyspaceResult {
            range,
            changed: response.changed,
            applied: AppliedPosition {
                term: response.applied_term,
                index: response.applied_index,
            },
        })
    }
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

    /// Commit one immutable migration intent. A confirmed receipt authorizes
    /// image-owner binding only, never transfer, voting or pin release.
    pub fn migrate(
        &mut self,
        root: RootDigest,
        operation: [u8; 16],
        creation_task: u64,
        destination: NodeId,
    ) -> Result<crate::api::MigrateDataGroupResult, DataGroupRpcError> {
        if root.as_bytes() == &[0; 32] || operation == [0; 16] || destination.0 == 0 {
            return Err(DataGroupRpcError::Local(
                "nonzero root, operation and destination required".into(),
            ));
        }
        let mut request = Request::new(proto::MigrateDataGroupRequest {
            root_digest: root.as_bytes().to_vec(),
            operation_id: operation.to_vec(),
            creation_task,
            destination_node: destination.0,
        });
        request
            .metadata_mut()
            .insert("authorization", self.authorization.clone());
        request.set_timeout(Duration::from_secs(30));
        let response = self
            .runtime
            .block_on(self.client.migrate_data_group(request))
            .map_err(rpc_error)?
            .into_inner();
        let intent =
            kv9_meta::data_groups::migration::MigrationIntent::decode(&response.migration_intent)
                .map_err(|e| DataGroupRpcError::Unconfirmed(e.to_string()))?;
        if intent.root() != root
            || intent.operation() != operation
            || intent.creation_task() != creation_task
            || intent.destination().node != destination
            || response.applied_term == 0
            || response.applied_index == 0
        {
            return Err(DataGroupRpcError::Unconfirmed(
                "response differs from request or lacks an exact applied receipt".into(),
            ));
        }
        Ok(crate::api::MigrateDataGroupResult {
            intent,
            changed: response.changed,
            applied: AppliedPosition {
                term: response.applied_term,
                index: response.applied_index,
            },
        })
    }

    /// Bind both tracking-only image owners for a committed migration. The
    /// returned IDs are observations; they carry no retention capability.
    pub fn bind_image(
        &mut self,
        root: RootDigest,
        operation: [u8; 16],
        manifest: &[u8],
    ) -> Result<crate::api::BindMigrationImageResult, DataGroupRpcError> {
        if root.as_bytes() == &[0; 32] || operation == [0; 16] || manifest.is_empty() {
            return Err(DataGroupRpcError::Local(
                "nonzero identities and a manifest required".into(),
            ));
        }
        let mut request = Request::new(proto::BindMigrationImageRequest {
            root_digest: root.as_bytes().to_vec(),
            operation_id: operation.to_vec(),
            manifest: manifest.to_vec(),
        });
        request
            .metadata_mut()
            .insert("authorization", self.authorization.clone());
        request.set_timeout(Duration::from_secs(30));
        let response = self
            .runtime
            .block_on(self.client.bind_migration_image(request))
            .map_err(rpc_error)?
            .into_inner();
        let owner = |bytes: Vec<u8>| {
            kv9_common::retention::OwnerId::new(
                bytes
                    .try_into()
                    .map_err(|_| DataGroupRpcError::Unconfirmed("invalid owner id".into()))?,
            )
            .map_err(|e| DataGroupRpcError::Unconfirmed(e.to_string()))
        };
        Ok(crate::api::BindMigrationImageResult {
            source_owner: owner(response.source_owner)?,
            destination_owner: owner(response.destination_owner)?,
        })
    }
}
