use super::*;
use kv9_common::AppliedPosition;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointRoute {
    pub node: NodeId,
    pub incarnation: StoreIncarnation,
    pub address: SocketAddr,
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointConfirmationReceipt {
    pub subject: EndpointRoute,
    pub responder: EndpointRoute,
    pub applied: AppliedPosition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndpointConfirmError {
    NotLeader(Option<LeaderHint>),
    Unconfirmed(String),
}

fn identity(bytes: &[u8]) -> std::result::Result<[u8; 16], Status> {
    let value: [u8; 16] = bytes
        .try_into()
        .map_err(|_| Status::invalid_argument("identity must have 16 bytes"))?;
    if value == [0; 16] {
        return Err(Status::invalid_argument("identity must be non-zero"));
    }
    Ok(value)
}

fn route(value: pb::EndpointRoute) -> std::result::Result<EndpointRoute, Status> {
    let address: SocketAddr = value
        .address
        .parse()
        .map_err(|_| Status::invalid_argument("invalid endpoint address"))?;
    if value.node_id == 0 || address.port() == 0 || address.ip().is_unspecified() {
        return Err(Status::invalid_argument("invalid endpoint route"));
    }
    Ok(EndpointRoute {
        node: NodeId(value.node_id),
        incarnation: StoreIncarnation::from_bytes(identity(&value.store_incarnation)?),
        address,
        generation: value.generation,
    })
}

fn encode(value: EndpointRoute) -> pb::EndpointRoute {
    pb::EndpointRoute {
        node_id: value.node.0,
        store_incarnation: value.incarnation.as_bytes().to_vec(),
        address: value.address.to_string(),
        generation: value.generation,
    }
}

impl RaftGrpcService {
    pub(super) async fn confirm_endpoint_request(
        &self,
        request: Request<pb::ConfirmEndpointRequest>,
    ) -> std::result::Result<Response<pb::EndpointConfirmationReceipt>, Status> {
        let node = *request
            .extensions()
            .get::<NodeId>()
            .ok_or_else(|| Status::unauthenticated("authenticated node identity missing"))?;
        let value = request.into_inner();
        if value.node_id != node.0 || node.0 == 0 {
            return Err(Status::permission_denied(
                "endpoint subject does not match authenticated node",
            ));
        }
        let root = self.discovery.root_identity().root_digest;
        if value.root_digest.as_slice() != root.as_bytes() {
            return Err(Status::failed_precondition(
                "endpoint confirmation root mismatch",
            ));
        }
        let cluster = ClusterId::from_bytes(identity(&value.cluster_id)?);
        let incarnation = StoreIncarnation::from_bytes(identity(&value.store_incarnation)?);
        let address: SocketAddr = value
            .address
            .parse()
            .map_err(|_| Status::invalid_argument("invalid endpoint address"))?;
        if address.port() == 0 || address.ip().is_unspecified() {
            return Err(Status::invalid_argument(
                "endpoint address must be specified",
            ));
        }
        let backend = self
            .registration
            .clone()
            .ok_or_else(|| Status::unimplemented("endpoint confirmation unavailable"))?;
        let permit = self
            .registration_capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                Status::resource_exhausted("registration or endpoint confirmation is in progress")
            })?;
        let receipt = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            backend.confirm_endpoint(node, cluster, incarnation, address)
        })
        .await
        .map_err(|_| Status::internal("endpoint confirmation task failed"))?
        .map_err(|error| match error {
            RegistrationError::NotLeader {
                leader,
                leader_addr,
            } => {
                let mut status = Status::failed_precondition("not the leader");
                status
                    .metadata_mut()
                    .insert(NOT_LEADER_KEY, "true".parse().unwrap());
                if let Some(id) = leader {
                    status
                        .metadata_mut()
                        .insert(LEADER_NODE_ID_KEY, id.0.to_string().parse().unwrap());
                    if let Some(address) = leader_addr.and_then(|a| a.parse().ok()) {
                        status.metadata_mut().insert(LEADER_ADDR_KEY, address);
                    }
                }
                status
            }
            other => {
                Status::failed_precondition(format!("endpoint confirmation refused: {other:?}"))
            }
        })?;
        Ok(Response::new(pb::EndpointConfirmationReceipt {
            subject: Some(encode(receipt.subject)),
            responder: Some(encode(receipt.responder)),
            applied_term: receipt.applied.term,
            applied_index: receipt.applied.index,
            root_digest: root.as_bytes().to_vec(),
        }))
    }
}

fn refusal(status: Status) -> EndpointConfirmError {
    // Only an exclusive, canonical marker is a routing hint. Everything else
    // remains unconfirmed; no response text establishes authorization.
    let metadata = status.metadata();
    if status.code() == tonic::Code::FailedPrecondition
        && metadata.get_all(NOT_LEADER_KEY).iter().count() == 1
        && metadata.get(NOT_LEADER_KEY).and_then(|v| v.to_str().ok()) == Some("true")
        && !metadata.contains_key(REJECTION_REASON_KEY)
        && metadata.get_all(LEADER_NODE_ID_KEY).iter().count() <= 1
        && metadata.get_all(LEADER_ADDR_KEY).iter().count() <= 1
    {
        let id = metadata
            .get(LEADER_NODE_ID_KEY)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|n| *n > 0);
        if let Some(id) = id {
            let addr = match metadata.get(LEADER_ADDR_KEY) {
                Some(value) => match value
                    .to_str()
                    .ok()
                    .and_then(|v| v.parse::<SocketAddr>().ok())
                {
                    Some(addr) if addr.port() > 0 && !addr.ip().is_unspecified() => Some(addr),
                    _ => return EndpointConfirmError::Unconfirmed(status.to_string()),
                },
                None => None,
            };
            return EndpointConfirmError::NotLeader(Some(LeaderHint {
                id: NodeId(id),
                addr,
            }));
        }
        if !metadata.contains_key(LEADER_NODE_ID_KEY) && !metadata.contains_key(LEADER_ADDR_KEY) {
            return EndpointConfirmError::NotLeader(None);
        }
    }
    EndpointConfirmError::Unconfirmed(status.to_string())
}

#[allow(clippy::too_many_arguments)]
pub fn grpc_confirm_endpoint(
    handle: &tokio::runtime::Handle,
    me: NodeId,
    peer: (NodeId, SocketAddr),
    cluster: ClusterId,
    root: RootDigest,
    incarnation: StoreIncarnation,
    address: SocketAddr,
    token: String,
    timeout: Duration,
) -> std::result::Result<EndpointConfirmationReceipt, EndpointConfirmError> {
    handle.block_on(async {
        let operation = async {
            let mut client = Kv9RaftClient::connect(format!("http://{}", peer.1))
                .await
                .map_err(|e| EndpointConfirmError::Unconfirmed(e.to_string()))?;
            let mut request = Request::new(pb::ConfirmEndpointRequest {
                node_id: me.0,
                cluster_id: cluster.as_bytes().to_vec(),
                store_incarnation: incarnation.as_bytes().to_vec(),
                address: address.to_string(),
                root_digest: root.as_bytes().to_vec(),
            });
            attach_auth(&mut request, &Some(token), me);
            let response = client
                .confirm_endpoint(request)
                .await
                .map_err(refusal)?
                .into_inner();
            decode_confirmation(response, (me, incarnation, address), peer.0, root)
                .map_err(|e| EndpointConfirmError::Unconfirmed(e.to_string()))
        };
        tokio::time::timeout(timeout, operation)
            .await
            .map_err(|_| {
                EndpointConfirmError::Unconfirmed("endpoint confirmation deadline expired".into())
            })?
    })
}

fn decode_confirmation(
    response: pb::EndpointConfirmationReceipt,
    expected: (NodeId, StoreIncarnation, SocketAddr),
    peer: NodeId,
    root: RootDigest,
) -> std::result::Result<EndpointConfirmationReceipt, Status> {
    let subject = route(
        response
            .subject
            .ok_or_else(|| Status::data_loss("missing endpoint subject"))?,
    )?;
    let responder = route(
        response
            .responder
            .ok_or_else(|| Status::data_loss("missing endpoint responder"))?,
    )?;
    if subject.node != expected.0
        || subject.incarnation != expected.1
        || subject.address != expected.2
        || responder.node != peer
        || response.applied_term == 0
        || response.applied_index == 0
        || response.root_digest.as_slice() != root.as_bytes()
    {
        return Err(Status::data_loss(
            "endpoint confirmation does not match its request",
        ));
    }
    Ok(EndpointConfirmationReceipt {
        subject,
        responder,
        applied: AppliedPosition {
            term: response.applied_term,
            index: response.applied_index,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirmation_receipt_binds_root_subject_responder_and_exact_position() {
        let subject = EndpointRoute {
            node: NodeId(4),
            incarnation: StoreIncarnation::from_bytes([4; 16]),
            address: "127.0.0.1:44004".parse().unwrap(),
            generation: 1,
        };
        let responder = EndpointRoute {
            node: NodeId(1),
            incarnation: StoreIncarnation::from_bytes([1; 16]),
            address: "127.0.0.1:44001".parse().unwrap(),
            generation: 2,
        };
        let root = RootDigest::from_bytes([7; 32]);
        let good = pb::EndpointConfirmationReceipt {
            subject: Some(encode(subject)),
            responder: Some(encode(responder)),
            applied_term: 3,
            applied_index: 31,
            root_digest: root.as_bytes().to_vec(),
        };
        let decode = |r| {
            decode_confirmation(
                r,
                (subject.node, subject.incarnation, subject.address),
                responder.node,
                root,
            )
        };
        assert_eq!(
            decode(good.clone()).unwrap().applied,
            AppliedPosition { term: 3, index: 31 }
        );
        let mut invalid = vec![
            pb::EndpointConfirmationReceipt {
                root_digest: vec![8; 32],
                ..good.clone()
            },
            pb::EndpointConfirmationReceipt {
                subject: None,
                ..good.clone()
            },
            pb::EndpointConfirmationReceipt {
                responder: None,
                ..good.clone()
            },
            pb::EndpointConfirmationReceipt {
                applied_term: 0,
                ..good.clone()
            },
            pb::EndpointConfirmationReceipt {
                applied_index: 0,
                ..good.clone()
            },
            pb::EndpointConfirmationReceipt {
                responder: Some(encode(EndpointRoute {
                    node: NodeId(2),
                    ..responder
                })),
                ..good.clone()
            },
        ];
        for changed in [
            EndpointRoute {
                node: NodeId(5),
                ..subject
            },
            EndpointRoute {
                incarnation: responder.incarnation,
                ..subject
            },
            EndpointRoute {
                address: responder.address,
                ..subject
            },
        ] {
            invalid.push(pb::EndpointConfirmationReceipt {
                subject: Some(encode(changed)),
                ..good.clone()
            });
        }
        for receipt in invalid {
            assert!(
                decode(receipt).is_err(),
                "foreign or incomplete confirmation granted endpoint authority"
            );
        }
    }

    #[test]
    fn contradictory_or_malformed_endpoint_hints_remain_unconfirmed() {
        let mut good = Status::failed_precondition("not leader");
        good.metadata_mut()
            .insert(NOT_LEADER_KEY, "true".parse().unwrap());
        good.metadata_mut()
            .insert(LEADER_NODE_ID_KEY, "1".parse().unwrap());
        good.metadata_mut()
            .insert(LEADER_ADDR_KEY, "127.0.0.1:44001".parse().unwrap());
        assert!(matches!(
            refusal(good.clone()),
            EndpointConfirmError::NotLeader(Some(_))
        ));
        let mut duplicate = good.clone();
        duplicate
            .metadata_mut()
            .append(NOT_LEADER_KEY, "true".parse().unwrap());
        let mut conflict = good.clone();
        conflict.metadata_mut().insert(
            REJECTION_REASON_KEY,
            "root-identity-mismatch".parse().unwrap(),
        );
        let mut invalid_address = good;
        invalid_address
            .metadata_mut()
            .insert(LEADER_ADDR_KEY, "0.0.0.0:0".parse().unwrap());
        for status in [
            duplicate,
            conflict,
            invalid_address,
            Status::deadline_exceeded("ambiguous"),
            Status::unavailable("ambiguous"),
        ] {
            assert!(
                matches!(refusal(status), EndpointConfirmError::Unconfirmed(_)),
                "malformed or contradictory status became a retryable endpoint hint"
            );
        }
    }

    #[test]
    fn endpoint_confirmation_rejects_foreign_root_or_subject_before_backend_admission() {
        struct Discovery;
        impl GrpcDiscoveryState for Discovery {
            fn answer(&self) -> (NodeId, bool, u64) {
                (NodeId(1), true, 0)
            }
            fn raft_receive_allowed(&self) -> bool {
                false
            }
            fn root_identity(&self) -> RootWireIdentity {
                RootWireIdentity {
                    bootstrap_generation: BootstrapGeneration::from_bytes([1; 16]),
                    root_digest: RootDigest::from_bytes([7; 32]),
                }
            }
        }
        let (tx, _) = mpsc::unbounded_channel();
        let service = RaftGrpcService::new(NodeId(1), tx, Arc::new(Discovery));
        let runtime = tokio::runtime::Runtime::new().unwrap();
        for (node, root, expected) in [
            (4, vec![7; 32], tonic::Code::Unimplemented),
            (4, vec![8; 32], tonic::Code::FailedPrecondition),
            (5, vec![7; 32], tonic::Code::PermissionDenied),
        ] {
            let mut request = Request::new(pb::ConfirmEndpointRequest {
                node_id: node,
                cluster_id: vec![1; 16],
                store_incarnation: vec![4; 16],
                address: "127.0.0.1:44004".into(),
                root_digest: root,
            });
            request.extensions_mut().insert(NodeId(4));
            let result = runtime
                .block_on(service.confirm_endpoint_request(request))
                .unwrap_err();
            assert_eq!(
                result.code(),
                expected,
                "invalid endpoint request reached backend admission"
            );
            assert_eq!(service.registration_capacity.available_permits(), 1);
        }
    }
}
