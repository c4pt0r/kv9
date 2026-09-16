//! Scoped RPCs never fall back to unscoped methods or metadata data storage.
use super::*;
use kv9_common::data_range::DataRange;
use prost::Message;

pub(crate) const REFUSED: &str = "kv9-route-refused";
fn routed_error(error: Error) -> Status {
    let refusal = match &error {
        Error::StaleEpoch { .. } => Some("scope"),
        Error::RangeCrossesRegion => Some("cross_range"),
        _ => None,
    };
    if let Some(reason) = refusal {
        let mut status =
            Status::failed_precondition("scoped request refused without write effects");
        status
            .metadata_mut()
            .insert(REFUSED, reason.parse().expect("static ascii"));
        status
    } else {
        error_status(error)
    }
}

impl Kv9Grpc {
    pub(super) async fn lookup_routed(
        &self,
        request: Request<proto::LookupRawRouteRequest>,
    ) -> Result<Response<proto::LookupRawRouteResponse>, Status> {
        let _auth = auth_context(&request)?;
        let reservation = self.reserve(&request, WorkClass::MetadataRead)?;
        let request = request.into_inner();
        if request.keyspace_id == 0
            || request.keyspace_id > KeyspaceId::MAX
            || request.key.len() > crate::client::MAX_KEY_BYTES
        {
            return Err(Status::invalid_argument("invalid route lookup bounds"));
        }
        let root = kv9_common::RootDigest::from_bytes(
            request
                .root_digest
                .try_into()
                .map_err(|_| Status::invalid_argument("route root must contain 32 bytes"))?,
        );
        let result = self
            .backend
            .call(reservation, move |backend| {
                backend.lookup_raw_route(
                    root,
                    TenantId(request.tenant_id),
                    KeyspaceId(request.keyspace_id),
                    &request.key,
                )
            })
            .await?;
        Ok(Response::new(proto::LookupRawRouteResponse {
            root_digest: result.root.as_bytes().to_vec(),
            metadata_peers: result
                .metadata_peers
                .into_iter()
                .map(crate::endpoints::encode_endpoint)
                .collect(),
            metadata_leader: result.metadata_leader.map(|n| n.0),
            binding: result.range.map(|r| r.encode()).unwrap_or_default(),
            replicas: result
                .replicas
                .into_iter()
                .map(crate::endpoints::encode_endpoint)
                .collect(),
            data_leader: result.data_leader.map(|n| n.0),
        }))
    }

    pub(super) async fn call_routed(
        &self,
        request: Request<proto::RoutedRawRequest>,
    ) -> Result<Response<proto::RoutedRawResponse>, Status> {
        use proto::routed_raw_request::Operation;
        use proto::routed_raw_response::Result as Reply;
        let auth = auth_context(&request)?;
        let read = matches!(request.get_ref().operation, Some(Operation::Get(_)));
        let reservation = self.reserve(
            &request,
            if read {
                WorkClass::RawRead
            } else {
                WorkClass::RawWrite
            },
        )?;
        if request.get_ref().encoded_len() > crate::client::MAX_MESSAGE_BYTES {
            return Err(Status::invalid_argument(
                "scoped request exceeds byte bound",
            ));
        }
        let request = request.into_inner();
        let scope = DataRange::decode(&request.binding).map_err(error_status)?;
        if scope.sealed {
            return Err(routed_error(Error::StaleEpoch {
                region: scope.region,
            }));
        }
        let digest = scope.digest().as_bytes().to_vec();
        let context = RequestContext {
            keyspace: scope.keyspace,
            region_epoch: RegionEpoch {
                conf_ver: scope.conf_ver,
                version: scope.version,
            },
            origin: RequestOrigin::from_transport(auth.principal),
        };
        let operation = request
            .operation
            .ok_or_else(|| Status::invalid_argument("scoped operation is required"))?;
        let result = match operation {
            Operation::Get(get) => {
                if !crate::client::valid_batch_keys(&get.keys) {
                    return Err(Status::invalid_argument("invalid scoped read bounds"));
                }
                let target = self
                    .backend
                    .inner
                    .routed_target(&scope)
                    .map_err(routed_error)?;
                let values = self
                    .backend
                    .prepared_read_mapped(
                        reservation,
                        target.prepare_raw_batch_get(context, get.keys),
                        PreparedReadKind::Batch,
                        routed_error,
                    )
                    .await?;
                Reply::Values(proto::RawBatchGetResponse {
                    values: values.into_iter().map(optional_value).collect(),
                })
            }
            operation => {
                let operation = match operation {
                    Operation::Put(put) => {
                        let pairs: Vec<_> =
                            put.pairs.into_iter().map(|p| (p.key, p.value)).collect();
                        if !crate::client::valid_batch_pairs(&pairs) {
                            return Err(Status::invalid_argument("invalid scoped write bounds"));
                        }
                        crate::api::RawWrite::BatchPut(pairs)
                    }
                    Operation::DeleteKey(key) if key.len() <= crate::client::MAX_KEY_BYTES => {
                        crate::api::RawWrite::Delete { key }
                    }
                    _ => return Err(Status::invalid_argument("invalid scoped delete bounds")),
                };
                let backend = self.backend.inner.clone();
                let preparation = Box::new(move || {
                    let target = backend.routed_target(&scope)?;
                    target.prepare_raw_write(context, operation)()
                });
                let applied = self
                    .backend
                    .prepared_write_mapped(reservation, preparation, routed_error)
                    .await?;
                Reply::Applied(proto::RawWriteResponse {
                    applied_term: applied.term,
                    applied_index: applied.index,
                })
            }
        };
        Ok(Response::new(proto::RoutedRawResponse {
            binding_digest: digest,
            result: Some(result),
        }))
    }
}
