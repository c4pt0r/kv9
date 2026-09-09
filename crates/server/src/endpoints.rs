//! Typed public endpoint operations. No ambiguous update is retried implicitly.
use std::net::SocketAddr;
use std::time::Duration;

use kv9_common::{AppliedPosition, ClusterId, Error, NodeId, StoreIncarnation};
use tonic::{Request, Status};

use crate::api::{
    EndpointChange, EndpointReadResult, EndpointRefusal, EndpointUpdateResult, NodeEndpoint,
};
use crate::grpc::proto;

pub(crate) fn socket(value: &str) -> Result<SocketAddr, Error> {
    let addr: SocketAddr = value
        .parse()
        .map_err(|_| Error::Config("endpoint must be a numeric socket address".into()))?;
    if addr.port() == 0 || addr.ip().is_unspecified() {
        return Err(Error::Config(
            "advertised endpoint must have a routable address and non-zero port".into(),
        ));
    }
    Ok(addr)
}

fn identity(bytes: &[u8]) -> Result<[u8; 16], Error> {
    let value: [u8; 16] = bytes
        .try_into()
        .map_err(|_| Error::Config("endpoint identity must have exactly 16 bytes".into()))?;
    if value == [0; 16] {
        return Err(Error::Config("endpoint identity must be non-zero".into()));
    }
    Ok(value)
}

pub(crate) fn decode_change(
    value: proto::ChangeNodeEndpointRequest,
) -> Result<EndpointChange, Error> {
    if value.node_id == 0 {
        return Err(Error::Config("endpoint node id must be non-zero".into()));
    }
    Ok(EndpointChange {
        cluster: ClusterId::from_bytes(identity(&value.cluster_id)?),
        node: NodeId(value.node_id),
        incarnation: StoreIncarnation::from_bytes(identity(&value.store_incarnation)?),
        expected_address: socket(&value.expected_address)?,
        expected_generation: value.expected_generation,
        new_address: socket(&value.new_address)?,
    })
}

pub(crate) fn encode_endpoint(value: NodeEndpoint) -> proto::NodeEndpoint {
    proto::NodeEndpoint {
        node_id: value.node.0,
        store_incarnation: value.incarnation.as_bytes().to_vec(),
        address: value.address.to_string(),
        generation: value.generation,
        previous_address: value.previous_address.map(|a| a.to_string()),
        active: value.active,
    }
}

fn decode_endpoint(value: proto::NodeEndpoint) -> Result<NodeEndpoint, Error> {
    let previous_address = value.previous_address.as_deref().map(socket).transpose()?;
    if value.node_id == 0 || (value.generation == 0) != previous_address.is_none() {
        return Err(Error::Config(
            "invalid endpoint record identity or generation".into(),
        ));
    }
    Ok(NodeEndpoint {
        node: NodeId(value.node_id),
        incarnation: StoreIncarnation::from_bytes(identity(&value.store_incarnation)?),
        address: socket(&value.address)?,
        generation: value.generation,
        previous_address,
        active: value.active,
    })
}

pub(crate) fn encode_update(value: EndpointUpdateResult) -> proto::ChangeNodeEndpointResponse {
    use proto::change_node_endpoint_response::Outcome;
    let receipt = |endpoint, at: AppliedPosition| proto::EndpointReceipt {
        endpoint: Some(encode_endpoint(endpoint)),
        applied_term: at.term,
        applied_index: at.index,
    };
    let outcome = match value {
        EndpointUpdateResult::Changed { endpoint, applied } => {
            Outcome::Changed(receipt(endpoint, applied))
        }
        EndpointUpdateResult::Confirmed {
            endpoint,
            confirmation,
        } => Outcome::Confirmed(receipt(endpoint, confirmation)),
        EndpointUpdateResult::Refused(reason) => Outcome::Refused(match reason {
            EndpointRefusal::WrongCluster => proto::EndpointRefusal::WrongCluster,
            EndpointRefusal::MissingNode => proto::EndpointRefusal::MissingNode,
            EndpointRefusal::InactiveNode => proto::EndpointRefusal::InactiveNode,
            EndpointRefusal::InvalidIncarnation => proto::EndpointRefusal::InvalidIncarnation,
            EndpointRefusal::Conflict => proto::EndpointRefusal::Conflict,
            EndpointRefusal::GenerationExhausted => proto::EndpointRefusal::GenerationExhausted,
        } as i32),
    };
    proto::ChangeNodeEndpointResponse {
        outcome: Some(outcome),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndpointRpcError {
    Local(String),
    NotLeader {
        leader: Option<NodeId>,
    },
    AdmissionRefused {
        reason: &'static str,
    },
    /// The endpoint may have changed. Only an explicit retry of the identical
    /// CAS or a fresh read can resolve this; no original receipt is inferred.
    Unconfirmed(String),
}

impl std::fmt::Display for EndpointRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local(detail) => write!(f, "endpoint client input: {detail}"),
            Self::NotLeader { leader } => write!(f, "not leader; hint={leader:?}"),
            Self::AdmissionRefused { reason } => write!(f, "endpoint admission refused: {reason}"),
            Self::Unconfirmed(detail) => write!(f, "endpoint outcome unconfirmed: {detail}"),
        }
    }
}
impl std::error::Error for EndpointRpcError {}

fn rpc_error(status: Status) -> EndpointRpcError {
    if let crate::client::Reason::NotLeader { leader } =
        crate::client::classify_status(&status, false)
    {
        EndpointRpcError::NotLeader {
            leader: leader.map(NodeId),
        }
    } else if let Some(reason) = crate::grpc::admission_refusal(&status) {
        EndpointRpcError::AdmissionRefused { reason }
    } else {
        EndpointRpcError::Unconfirmed(status.to_string())
    }
}

/// A bounded blocking admin client. Each method issues exactly one RPC.
pub struct EndpointClient {
    client: proto::kv9_client::Kv9Client<tonic::transport::Channel>,
    authorization: tonic::metadata::MetadataValue<tonic::metadata::Ascii>,
    runtime: tokio::runtime::Runtime,
}

impl EndpointClient {
    pub fn connect(address: &str, token: &str) -> Result<Self, EndpointRpcError> {
        let address = socket(address).map_err(|e| EndpointRpcError::Local(e.to_string()))?;
        let authorization = format!("Bearer {token}")
            .parse()
            .map_err(|_| EndpointRpcError::Local("invalid bearer metadata".into()))?;
        let runtime =
            tokio::runtime::Runtime::new().map_err(|e| EndpointRpcError::Local(e.to_string()))?;
        let endpoint = tonic::transport::Endpoint::from_shared(format!("http://{address}"))
            .map_err(|e| EndpointRpcError::Local(e.to_string()))?
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(30));
        let channel = runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(3), endpoint.connect())
                .await
                .map_err(|e| EndpointRpcError::Unconfirmed(e.to_string()))?
                .map_err(|e| EndpointRpcError::Unconfirmed(e.to_string()))
        })?;
        Ok(Self {
            client: proto::kv9_client::Kv9Client::new(channel),
            authorization,
            runtime,
        })
    }

    pub fn get(&mut self, node: NodeId) -> Result<EndpointReadResult, EndpointRpcError> {
        let mut request = Request::new(proto::GetNodeEndpointRequest { node_id: node.0 });
        request
            .metadata_mut()
            .insert("authorization", self.authorization.clone());
        let response = self
            .runtime
            .block_on(self.client.get_node_endpoint(request))
            .map_err(rpc_error)?
            .into_inner();
        let cluster = ClusterId::from_bytes(
            identity(&response.cluster_id)
                .map_err(|e| EndpointRpcError::Unconfirmed(e.to_string()))?,
        );
        let endpoint = response
            .endpoint
            .map(decode_endpoint)
            .transpose()
            .map_err(|e| EndpointRpcError::Unconfirmed(e.to_string()))?;
        if endpoint.is_some_and(|e| e.node != node) {
            return Err(EndpointRpcError::Unconfirmed(
                "endpoint response names another node".into(),
            ));
        }
        Ok(EndpointReadResult { cluster, endpoint })
    }

    pub fn change(
        &mut self,
        change: EndpointChange,
    ) -> Result<EndpointUpdateResult, EndpointRpcError> {
        let body = proto::ChangeNodeEndpointRequest {
            cluster_id: change.cluster.as_bytes().to_vec(),
            node_id: change.node.0,
            store_incarnation: change.incarnation.as_bytes().to_vec(),
            expected_address: change.expected_address.to_string(),
            expected_generation: change.expected_generation,
            new_address: change.new_address.to_string(),
        };
        decode_change(body.clone()).map_err(|e| EndpointRpcError::Local(e.to_string()))?;
        let mut request = Request::new(body);
        request
            .metadata_mut()
            .insert("authorization", self.authorization.clone());
        let response = self
            .runtime
            .block_on(self.client.change_node_endpoint(request))
            .map_err(rpc_error)?
            .into_inner();
        decode_update(response, change).map_err(EndpointRpcError::Unconfirmed)
    }
}

fn decode_update(
    response: proto::ChangeNodeEndpointResponse,
    change: EndpointChange,
) -> Result<EndpointUpdateResult, String> {
    use proto::change_node_endpoint_response::Outcome;
    let outcome = response.outcome.ok_or("missing endpoint outcome")?;
    if let Outcome::Refused(reason) = outcome {
        return Ok(EndpointUpdateResult::Refused(
            match proto::EndpointRefusal::try_from(reason) {
                Ok(proto::EndpointRefusal::WrongCluster) => EndpointRefusal::WrongCluster,
                Ok(proto::EndpointRefusal::MissingNode) => EndpointRefusal::MissingNode,
                Ok(proto::EndpointRefusal::InactiveNode) => EndpointRefusal::InactiveNode,
                Ok(proto::EndpointRefusal::InvalidIncarnation) => {
                    EndpointRefusal::InvalidIncarnation
                }
                Ok(proto::EndpointRefusal::Conflict) => EndpointRefusal::Conflict,
                Ok(proto::EndpointRefusal::GenerationExhausted) => {
                    EndpointRefusal::GenerationExhausted
                }
                _ => return Err("invalid endpoint refusal".into()),
            },
        ));
    }
    let (receipt, changed) = match outcome {
        Outcome::Changed(r) => (r, true),
        Outcome::Confirmed(r) => (r, false),
        Outcome::Refused(_) => unreachable!("handled above"),
    };
    let endpoint = decode_endpoint(receipt.endpoint.ok_or("missing endpoint receipt record")?)
        .map_err(|e| e.to_string())?;
    if receipt.applied_term == 0
        || receipt.applied_index == 0
        || !endpoint.active
        || endpoint.node != change.node
        || endpoint.incarnation != change.incarnation
        || endpoint.address != change.new_address
        || endpoint.previous_address != Some(change.expected_address)
        || Some(endpoint.generation) != change.expected_generation.checked_add(1)
    {
        return Err("endpoint receipt does not establish the requested transition".into());
    }
    let at = AppliedPosition {
        term: receipt.applied_term,
        index: receipt.applied_index,
    };
    Ok(if changed {
        EndpointUpdateResult::Changed {
            endpoint,
            applied: at,
        }
    } else {
        EndpointUpdateResult::Confirmed {
            endpoint,
            confirmation: at,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::change_node_endpoint_response::Outcome;

    #[test]
    fn endpoint_decoder_refuses_foreign_or_incomplete_mutation_and_confirmation_receipts() {
        let request = EndpointChange {
            cluster: ClusterId::from_bytes([1; 16]),
            node: NodeId(4),
            incarnation: StoreIncarnation::from_bytes([4; 16]),
            expected_address: "127.0.0.1:40004".parse().unwrap(),
            expected_generation: 7,
            new_address: "127.0.0.1:41004".parse().unwrap(),
        };
        let endpoint = NodeEndpoint {
            node: request.node,
            incarnation: request.incarnation,
            address: request.new_address,
            generation: 8,
            previous_address: Some(request.expected_address),
            active: true,
        };
        let at = AppliedPosition { term: 3, index: 45 };
        let good = proto::EndpointReceipt {
            endpoint: Some(encode_endpoint(endpoint)),
            applied_term: at.term,
            applied_index: at.index,
        };
        let mut invalid = vec![
            proto::EndpointReceipt {
                endpoint: None,
                ..good.clone()
            },
            proto::EndpointReceipt {
                applied_term: 0,
                ..good.clone()
            },
            proto::EndpointReceipt {
                applied_index: 0,
                ..good.clone()
            },
        ];
        for changed in [
            NodeEndpoint {
                node: NodeId(5),
                ..endpoint
            },
            NodeEndpoint {
                incarnation: StoreIncarnation::from_bytes([5; 16]),
                ..endpoint
            },
            NodeEndpoint {
                address: request.expected_address,
                ..endpoint
            },
            NodeEndpoint {
                previous_address: Some(request.new_address),
                ..endpoint
            },
            NodeEndpoint {
                generation: 7,
                ..endpoint
            },
            NodeEndpoint {
                active: false,
                ..endpoint
            },
        ] {
            invalid.push(proto::EndpointReceipt {
                endpoint: Some(encode_endpoint(changed)),
                ..good.clone()
            });
        }
        for changed in [true, false] {
            let response = |r| proto::ChangeNodeEndpointResponse {
                outcome: Some(if changed {
                    Outcome::Changed(r)
                } else {
                    Outcome::Confirmed(r)
                }),
            };
            let decoded = decode_update(response(good.clone()), request).unwrap();
            assert_eq!(
                decoded,
                if changed {
                    EndpointUpdateResult::Changed {
                        endpoint,
                        applied: at,
                    }
                } else {
                    EndpointUpdateResult::Confirmed {
                        endpoint,
                        confirmation: at,
                    }
                }
            );
            for receipt in &invalid {
                assert!(
                    decode_update(response(receipt.clone()), request).is_err(),
                    "foreign or incomplete endpoint receipt became confirmed authority"
                );
            }
            assert!(decode_update(
                response(good.clone()),
                EndpointChange {
                    expected_generation: u64::MAX,
                    ..request
                }
            )
            .is_err());
        }
        for outcome in [None, Some(Outcome::Refused(0)), Some(Outcome::Refused(999))] {
            assert!(
                decode_update(proto::ChangeNodeEndpointResponse { outcome }, request).is_err(),
                "unknown endpoint outcome became a confirmed refusal"
            );
        }
    }
}
