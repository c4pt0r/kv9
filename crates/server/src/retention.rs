//! Bounded, single-attempt retention administration over the public transport.
//! Observations and exact receipts do not certify complete reference coverage,
//! live-use drainage, destination admission, or physical deletion eligibility.
use std::time::Duration;

use kv9_common::retention::OwnerId;
use kv9_common::{AppliedPosition, NodeId, RootDigest};
use kv9_meta::retention::{
    decode_owner_observation, decode_request, encode_request, LedgerOwner, LedgerRequest,
};
use tonic::{Request, Status};

use crate::api::RetentionUpdateResult;
use crate::grpc::proto;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetentionRpcError {
    Local(String),
    NotLeader {
        leader: Option<NodeId>,
    },
    AdmissionRefused {
        reason: &'static str,
    },
    /// An error is not proof that a mutation failed to commit. No automatic retry.
    Unconfirmed(String),
}
impl std::fmt::Display for RetentionRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local(e) => write!(f, "retention client input: {e}"),
            Self::NotLeader { leader } => write!(f, "not leader; hint={leader:?}"),
            Self::AdmissionRefused { reason } => write!(f, "retention admission refused: {reason}"),
            Self::Unconfirmed(e) => write!(f, "retention outcome unconfirmed: {e}"),
        }
    }
}
impl std::error::Error for RetentionRpcError {}
fn rpc_error(status: Status) -> RetentionRpcError {
    if let crate::client::Reason::NotLeader { leader } =
        crate::client::classify_status(&status, false)
    {
        RetentionRpcError::NotLeader {
            leader: leader.map(NodeId),
        }
    } else if let Some(reason) = crate::grpc::admission_refusal(&status) {
        RetentionRpcError::AdmissionRefused { reason }
    } else {
        RetentionRpcError::Unconfirmed(status.to_string())
    }
}

pub struct RetentionClient {
    client: proto::kv9_client::Kv9Client<tonic::transport::Channel>,
    authorization: tonic::metadata::MetadataValue<tonic::metadata::Ascii>,
    runtime: tokio::runtime::Runtime,
}
impl RetentionClient {
    pub fn connect(address: &str, token: &str) -> Result<Self, RetentionRpcError> {
        let address = crate::endpoints::socket(address)
            .map_err(|e| RetentionRpcError::Local(e.to_string()))?;
        let authorization = format!("Bearer {token}")
            .parse()
            .map_err(|_| RetentionRpcError::Local("invalid bearer metadata".into()))?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| RetentionRpcError::Local(e.to_string()))?;
        let endpoint = tonic::transport::Endpoint::from_shared(format!("http://{address}"))
            .map_err(|e| RetentionRpcError::Local(e.to_string()))?
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(30));
        let channel = runtime
            .block_on(endpoint.connect())
            .map_err(|e| RetentionRpcError::Unconfirmed(e.to_string()))?;
        Ok(Self {
            client: proto::kv9_client::Kv9Client::new(channel),
            authorization,
            runtime,
        })
    }
    pub fn apply(
        &mut self,
        root: RootDigest,
        request: &LedgerRequest,
    ) -> Result<RetentionUpdateResult, RetentionRpcError> {
        let bytes =
            encode_request(root, request).map_err(|e| RetentionRpcError::Local(e.to_string()))?;
        self.apply_encoded(root, bytes)
    }
    pub fn apply_encoded(
        &mut self,
        root: RootDigest,
        command: Vec<u8>,
    ) -> Result<RetentionUpdateResult, RetentionRpcError> {
        decode_request(&command, root).map_err(|e| RetentionRpcError::Local(e.to_string()))?;
        let mut request = Request::new(proto::ApplyRetentionRequest { command });
        request
            .metadata_mut()
            .insert("authorization", self.authorization.clone());
        request.set_timeout(Duration::from_secs(30));
        let r = self
            .runtime
            .block_on(self.client.apply_retention(request))
            .map_err(rpc_error)?
            .into_inner();
        if r.applied_term == 0 || r.applied_index == 0 {
            return Err(RetentionRpcError::Unconfirmed(
                "response has no exact applied position".into(),
            ));
        }
        Ok(RetentionUpdateResult {
            revision: r.revision,
            changed: r.changed,
            applied: AppliedPosition {
                term: r.applied_term,
                index: r.applied_index,
            },
        })
    }
    pub fn get(
        &mut self,
        root: RootDigest,
        owner: OwnerId,
    ) -> Result<Option<LedgerOwner>, RetentionRpcError> {
        if root.as_bytes() == &[0; 32] {
            return Err(RetentionRpcError::Local("root must be nonzero".into()));
        }
        let mut request = Request::new(proto::GetRetentionOwnerRequest {
            root_digest: root.as_bytes().to_vec(),
            owner_id: owner.as_bytes().to_vec(),
        });
        request
            .metadata_mut()
            .insert("authorization", self.authorization.clone());
        request.set_timeout(Duration::from_secs(30));
        let r = self
            .runtime
            .block_on(self.client.get_retention_owner(request))
            .map_err(rpc_error)?
            .into_inner();
        if r.found {
            decode_owner_observation(&r.owner, root, owner)
                .map(Some)
                .map_err(|e| RetentionRpcError::Unconfirmed(e.to_string()))
        } else if r.owner.is_empty() {
            Ok(None)
        } else {
            Err(RetentionRpcError::Unconfirmed(
                "absent owner response contains state".into(),
            ))
        }
    }
}
