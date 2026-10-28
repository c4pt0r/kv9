use super::*;
use crate::raw_ttl::{Operation, Refusal, Reply, MAX_BUDGET_MICROS, PROTOCOL_VERSION, REFUSED};
use kv9_common::data_range::DataRange;
use kv9_common::raw_ttl::TtlRejection;
use prost::Message;
use std::time::{Duration, Instant};

fn ttl_error(error: Error) -> Status {
    let refusal = match error {
        Error::RawTtlRejected { reason, .. } => match reason {
            TtlRejection::NotEnabled => Refusal::NotEnabled,
            TtlRejection::LegacyCommand => Refusal::LegacyCommand,
            TtlRejection::InvalidScope => Refusal::InvalidScope,
        },
        Error::RawTtlClock { reason } => Refusal::Clock { reason },
        Error::StaleEpoch { .. } => Refusal::StaleScope,
        Error::RangeCrossesRegion => Refusal::CrossRange,
        Error::WriteBackpressure { .. } => Refusal::Backpressure,
        other => return error_status(other),
    };
    let mut status =
        Status::failed_precondition("Raw TTL request refused without user write effects");
    status.metadata_mut().insert(
        REFUSED,
        serde_json::to_string(&refusal)
            .expect("refusal JSON")
            .parse()
            .expect("ASCII refusal"),
    );
    status
}

impl Kv9Grpc {
    pub(super) async fn activate_ttl(
        &self,
        request: Request<proto::ActivateRawTtlRequest>,
    ) -> Result<Response<proto::RawTtlResponse>, Status> {
        let started = Instant::now();
        let auth = auth_context(&request)?;
        let reservation = self.reserve(&request, WorkClass::MetadataWrite)?;
        let request = request.into_inner();
        if request.protocol_version != PROTOCOL_VERSION
            || request.encoded_len() > crate::raw_ttl::MAX_ACTIVATION_REQUEST_BYTES
            || !(1..=MAX_BUDGET_MICROS).contains(&request.budget_micros)
        {
            return Err(Status::invalid_argument(
                "invalid TTL activation protocol or bounds",
            ));
        }
        let binding = DataRange::decode(&request.binding).map_err(ttl_error)?;
        if binding.sealed {
            return Err(ttl_error(Error::StaleEpoch {
                region: binding.region,
            }));
        }
        let digest = binding.digest().as_bytes().to_vec();
        let prepare = self.backend.inner.clone().prepare_raw_ttl_activation(
            auth.principal.to_string(),
            binding,
            started + Duration::from_micros(request.budget_micros),
        );
        // Completion owns the reservation through all RPCs and exact apply,
        // including when the caller cancels after activation was appended.
        let at = self
            .backend
            .prepared_operation_mapped(reservation, prepare, ttl_error)
            .await?;
        Ok(Response::new(proto::RawTtlResponse {
            binding_digest: digest,
            protocol_version: PROTOCOL_VERSION,
            result: Some(proto::raw_ttl_response::Result::Applied(applied_response(
                at,
            ))),
        }))
    }
    pub(super) async fn call_ttl(
        &self,
        request: Request<proto::RawTtlRequest>,
    ) -> Result<Response<proto::RawTtlResponse>, Status> {
        let start = Instant::now();
        let auth = auth_context(&request)?;
        let read = matches!(
            request.get_ref().operation,
            Some(
                proto::raw_ttl_request::Operation::Capabilities(_)
                    | proto::raw_ttl_request::Operation::Get(_)
                    | proto::raw_ttl_request::Operation::Scan(_)
            )
        );
        let reservation = self.reserve(
            &request,
            if read {
                WorkClass::RawRead
            } else {
                WorkClass::RawWrite
            },
        )?;
        let request = request.into_inner();
        if request.encoded_len() > crate::client::MAX_MESSAGE_BYTES
            || request.protocol_version != PROTOCOL_VERSION
            || !(1..=MAX_BUDGET_MICROS).contains(&request.budget_micros)
        {
            return Err(Status::invalid_argument(
                "invalid Raw TTL protocol or request bounds",
            ));
        }
        let deadline = start + Duration::from_micros(request.budget_micros);
        let scope = DataRange::decode(&request.binding).map_err(ttl_error)?;
        if scope.sealed {
            return Err(ttl_error(Error::StaleEpoch {
                region: scope.region,
            }));
        }
        let digest = scope.digest().as_bytes().to_vec();
        let ctx = RequestContext {
            keyspace: scope.keyspace,
            region_epoch: RegionEpoch {
                conf_ver: scope.conf_ver,
                version: scope.version,
            },
            origin: RequestOrigin::from_transport(auth.principal),
        };
        let op = match request
            .operation
            .ok_or_else(|| Status::invalid_argument("Raw TTL operation required"))?
        {
            proto::raw_ttl_request::Operation::Capabilities(_) => Operation::Capabilities,
            proto::raw_ttl_request::Operation::Get(get) => Operation::Get { keys: get.keys },
            proto::raw_ttl_request::Operation::Put(put) => Operation::Put {
                pairs: put.pairs.into_iter().map(|p| (p.key, p.value)).collect(),
                ttl_secs: put.ttl_secs,
            },
            proto::raw_ttl_request::Operation::DeleteKey(key) => Operation::Delete { key },
            proto::raw_ttl_request::Operation::Scan(scan) => Operation::Scan {
                start: scan.start,
                end: scan.end,
                limit: scan.limit as usize,
            },
            proto::raw_ttl_request::Operation::DeleteRange(scan) => Operation::DeleteRange {
                start: scan.start,
                end: scan.end,
                limit: scan.limit as usize,
            },
        };
        op.validate()
            .map_err(|_| Status::invalid_argument("invalid Raw TTL operation bounds"))?;
        let backend = self.backend.inner.clone();
        let prepare = Box::new(move || {
            backend
                .routed_target(&scope)?
                .prepare_raw_ttl(ctx, op, deadline)()
        });
        // Observe appends a log entry too: hold admission across cancellation,
        // apply wait, ReadIndex and the final blocking snapshot job.
        let reply = self
            .backend
            .prepared_operation_mapped(reservation, prepare, ttl_error)
            .await?;
        use proto::raw_ttl_response::Result as Wire;
        let result = match reply {
            Reply::Capabilities => Wire::Capabilities(proto::Empty {}),
            Reply::Values { values } => Wire::Values(proto::RawBatchGetResponse {
                values: values.into_iter().map(optional_value).collect(),
            }),
            Reply::Applied { term, index } => Wire::Applied(proto::RawWriteResponse {
                applied_term: term,
                applied_index: index,
            }),
            Reply::Page {
                rows,
                next,
                examined,
            } => Wire::Page(proto::RawTtlPage {
                pairs: rows
                    .into_iter()
                    .map(|(key, value)| proto::KeyValue { key, value })
                    .collect(),
                next,
                examined: examined as u32,
            }),
            Reply::Deleted {
                processed_keys,
                applied,
                next,
            } => Wire::Deleted(proto::RawTtlDeleted {
                processed_keys: processed_keys as u32,
                applied: applied.map(|at| proto::RawWriteResponse {
                    applied_term: at.term,
                    applied_index: at.index,
                }),
                next,
            }),
        };
        let response = proto::RawTtlResponse {
            binding_digest: digest,
            protocol_version: PROTOCOL_VERSION,
            result: Some(result),
        };
        if response.encoded_len() > crate::client::MAX_MESSAGE_BYTES {
            return Err(Status::resource_exhausted(
                "Raw TTL response exceeds message limit",
            ));
        }
        Ok(Response::new(response))
    }
}

#[cfg(test)]
mod tests;
