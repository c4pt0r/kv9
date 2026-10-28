//! TTL calls bind an exact range and confirm the distinct protocol first. Only
//! capability probes and definitively unsubmitted activations may redirect.
//! A data invocation is sent at most once; callers must not replay unknown writes.
use super::*;
use crate::raw_ttl::{
    Operation, Refusal, Reply, MAX_BUDGET_MICROS, MAX_SCAN_EXAMINED, PROTOCOL_VERSION, REFUSED,
};
use kv9_common::data_range::DataRange;

const CAPABILITIES: usize = 0;
const WRITE: usize = MAX_BATCH_ITEMS + 1;
const SCAN: usize = MAX_BATCH_ITEMS + 2;
const DELETE_RANGE: usize = MAX_BATCH_ITEMS + 3;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Failure {
    Refused { refusal: Refusal },
    Rpc { reason: Reason },
}
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TtlOutcome {
    Success { reply: Reply },
    Refused { failure: Failure },
    UnknownWrite { failure: Failure },
    ReadFailure { failure: Failure },
    CapabilityFailure { failure: Failure },
    ClientRejected { reason: Reason },
}
#[derive(Debug, Serialize)]
pub struct TtlAttempt {
    pub node_id: u64,
    pub capability: bool,
    pub elapsed_ns: u64,
    pub failure: Option<Failure>,
}
#[derive(Debug, Serialize)]
pub struct TtlReport {
    pub elapsed_ns: u64,
    pub attempts: Vec<TtlAttempt>,
    pub outcome: TtlOutcome,
}

#[derive(Clone)]
pub struct ScopedTtlClient {
    client: PersistentRawClient,
    binding: DataRange,
}
impl ScopedTtlClient {
    /// Explicit one-way activation. Uses its distinct unary admin RPC even
    /// when data calls use point stream. Unknown outcomes are never replayed;
    /// a later explicit invocation returns a NEW confirmation position.
    pub async fn activate(&self) -> TtlReport {
        let start = Instant::now();
        let deadline = start + Duration::from_millis(self.client.0.config.deadline_ms);
        let mut report = TtlReport {
            elapsed_ns: 0,
            attempts: Vec::new(),
            outcome: TtlOutcome::ClientRejected {
                reason: Reason::ClientCapacity,
            },
        };
        let Ok(_permit) = self.client.0.capacity.clone().try_acquire_owned() else {
            return report;
        };
        let mut peer = self.client.0.preferred.load(Ordering::Relaxed);
        for ordinal in 0..self.client.0.config.max_attempts {
            let budget = deadline
                .saturating_duration_since(Instant::now())
                .as_micros() as u64;
            if budget == 0 {
                if report.attempts.is_empty() {
                    report.outcome = TtlOutcome::ClientRejected {
                        reason: Reason::Deadline,
                    };
                }
                break;
            }
            let request = self.client.request(
                proto::ActivateRawTtlRequest {
                    binding: self.binding.encode(),
                    protocol_version: PROTOCOL_VERSION,
                    budget_micros: budget,
                },
                deadline,
            );
            let attempted = Instant::now();
            let result = timeout_at(
                deadline,
                self.client.0.clients[peer]
                    .clone()
                    .activate_raw_ttl(request),
            )
            .await
            .map_err(|_| rpc(Reason::Deadline))
            .and_then(|r| r.map_err(|s| classify(&s)))
            .and_then(|response| {
                if !valid_envelope(&response, WRITE)
                    || response.get_ref().binding_digest != self.binding.digest().as_bytes()
                {
                    return Err(rpc(Reason::Protocol));
                }
                let Some(proto::raw_ttl_response::Result::Applied(at)) =
                    response.into_inner().result
                else {
                    return Err(rpc(Reason::Protocol));
                };
                Ok(Reply::Applied {
                    term: at.applied_term,
                    index: at.applied_index,
                })
            });
            report.attempts.push(TtlAttempt {
                node_id: self.client.0.config.peers[peer].node_id,
                capability: false,
                elapsed_ns: nanos(attempted.elapsed()),
                failure: result.as_ref().err().cloned(),
            });
            let redirect = match &result {
                Err(Failure::Rpc {
                    reason: Reason::NotLeader { leader },
                }) => Some(*leader),
                _ => None,
            };
            report.outcome = data_outcome(result, false);
            match redirect {
                Some(hint) if ordinal + 1 < self.client.0.config.max_attempts => {
                    peer = self.client.next_peer(peer, hint);
                    sleep_until(deadline.min(
                        Instant::now()
                            + Duration::from_millis(self.client.0.config.retry_backoff_ms),
                    ))
                    .await;
                }
                _ => break,
            }
        }
        self.client.0.preferred.store(peer, Ordering::Relaxed);
        report.elapsed_ns = nanos(start.elapsed());
        report
    }
    #[cfg(test)]
    pub(crate) fn with_test_transport(
        binding: DataRange,
        transport: TransportKind,
        channel: Channel,
        deadline_ms: u64,
    ) -> Self {
        Self::with_test_channels(binding, transport, vec![(1, channel)], deadline_ms)
    }
    #[cfg(test)]
    pub(crate) fn with_test_channels(
        binding: DataRange,
        transport: TransportKind,
        channels: Vec<(u64, Channel)>,
        deadline_ms: u64,
    ) -> Self {
        let config = ClientConfig {
            version: 1,
            peers: channels
                .iter()
                .map(|(node_id, _)| Peer {
                    node_id: *node_id,
                    address: format!("127.0.0.1:{node_id}").parse().unwrap(),
                })
                .collect(),
            keyspace_id: binding.keyspace.0,
            epoch_conf_ver: binding.conf_ver,
            epoch_version: binding.version,
            max_in_flight: 4,
            max_attempts: 4,
            deadline_ms,
            retry_backoff_ms: 1,
        };
        let mut c = Self::new(config, "secret", binding, transport).unwrap();
        let inner = Arc::get_mut(&mut c.client.0).unwrap();
        inner.clients = channels
            .iter()
            .map(|(_, channel)| Kv9Client::new(channel.clone()))
            .collect();
        if transport == TransportKind::TonicStream {
            inner.streaming = Some(
                channels
                    .into_iter()
                    .map(|(_, channel)| {
                        crate::point_stream::StreamClient::with_test_transport(channel, 4)
                    })
                    .collect(),
            );
        }
        c
    }
    /// `binding` must come from the caller's root-pinned route observation.
    /// This client does not refresh or cross ranges; stale scope is terminal.
    pub fn new(
        config: ClientConfig,
        token: &str,
        binding: DataRange,
        transport: TransportKind,
    ) -> Result<Self, &'static str> {
        if !matches!(
            transport,
            TransportKind::TonicUnary | TransportKind::TonicStream
        ) || binding.sealed
            || DataRange::decode(&binding.encode()).is_err()
            || config.keyspace_id != binding.keyspace.0
            || config.epoch_conf_ver != binding.conf_ver
            || config.epoch_version != binding.version
            || config.max_attempts < 2
        {
            return Err("invalid TTL scope, transport or attempt budget");
        }
        Ok(Self {
            client: PersistentRawClient::new_with_transport(config, token, transport)?,
            binding,
        })
    }

    async fn dispatch(
        &self,
        peer: usize,
        op: &Operation,
        deadline: Instant,
    ) -> Result<Reply, Failure> {
        let budget_micros = deadline
            .saturating_duration_since(Instant::now())
            .as_micros()
            .min(MAX_BUDGET_MICROS as u128) as u64;
        if budget_micros == 0 {
            return Err(rpc(Reason::Deadline));
        }
        let request = proto::RawTtlRequest {
            binding: self.binding.encode(),
            protocol_version: PROTOCOL_VERSION,
            budget_micros,
            operation: Some(op.clone().into()),
        };
        if request.encoded_len() > MAX_MESSAGE_BYTES {
            return Err(rpc(Reason::ClientInput));
        }
        let request = self.client.request(request, deadline);
        let response = if let Some(streaming) = &self.client.0.streaming {
            streaming[peer]
                .raw_ttl(request, expected(op), deadline)
                .await
        } else {
            self.client.0.clients[peer].clone().raw_ttl(request).await
        }
        .map_err(|s| classify(&s))?;
        decode(response, op, &self.binding)
    }

    pub async fn call(&self, op: Operation) -> TtlReport {
        let start = Instant::now();
        let deadline = start + Duration::from_millis(self.client.0.config.deadline_ms);
        let mut report = TtlReport {
            elapsed_ns: 0,
            attempts: vec![],
            outcome: TtlOutcome::ClientRejected {
                reason: Reason::ClientInput,
            },
        };
        let finish = |mut report: TtlReport| {
            report.elapsed_ns = nanos(start.elapsed());
            report
        };
        if op.validate().is_err()
            || !contained(&op, &self.binding)
            || (proto::RawTtlRequest {
                binding: self.binding.encode(),
                protocol_version: PROTOCOL_VERSION,
                budget_micros: MAX_BUDGET_MICROS,
                operation: Some(op.clone().into()),
            })
            .encoded_len()
                > MAX_MESSAGE_BYTES
        {
            return finish(report);
        }
        let Ok(_permit) = self.client.0.capacity.clone().try_acquire_owned() else {
            report.outcome = TtlOutcome::ClientRejected {
                reason: Reason::ClientCapacity,
            };
            return finish(report);
        };
        let mut peer = self.client.0.preferred.load(Ordering::Relaxed);
        let probes =
            self.client.0.config.max_attempts - usize::from(!matches!(op, Operation::Capabilities));
        for ordinal in 0..probes {
            let attempted = Instant::now();
            let result = timeout_at(
                deadline,
                self.dispatch(peer, &Operation::Capabilities, deadline),
            )
            .await
            .unwrap_or_else(|_| Err(rpc(Reason::Deadline)));
            report.attempts.push(TtlAttempt {
                node_id: self.client.0.config.peers[peer].node_id,
                capability: true,
                elapsed_ns: nanos(attempted.elapsed()),
                failure: result.as_ref().err().cloned(),
            });
            match result {
                Ok(reply) if matches!(op, Operation::Capabilities) => {
                    report.outcome = TtlOutcome::Success { reply };
                    return finish(report);
                }
                Ok(_) => break,
                Err(failure) => {
                    let hint = match failure {
                        Failure::Rpc {
                            reason: Reason::NotLeader { leader },
                        } => leader,
                        _ => None,
                    };
                    let retry = matches!(
                        failure,
                        Failure::Rpc {
                            reason: Reason::NotLeader { .. }
                        }
                    );
                    report.outcome = TtlOutcome::CapabilityFailure { failure };
                    if !retry || ordinal + 1 == probes {
                        return finish(report);
                    }
                    peer = self.client.next_peer(peer, hint);
                    sleep_until(deadline.min(
                        Instant::now()
                            + Duration::from_millis(self.client.0.config.retry_backoff_ms),
                    ))
                    .await;
                }
            }
        }
        if Instant::now() >= deadline {
            report.outcome = TtlOutcome::ClientRejected {
                reason: Reason::Deadline,
            };
            return finish(report);
        }
        let attempted = Instant::now();
        let result = timeout_at(deadline, self.dispatch(peer, &op, deadline))
            .await
            .unwrap_or_else(|_| Err(rpc(Reason::Deadline)));
        report.attempts.push(TtlAttempt {
            node_id: self.client.0.config.peers[peer].node_id,
            capability: false,
            elapsed_ns: nanos(attempted.elapsed()),
            failure: result.as_ref().err().cloned(),
        });
        report.outcome = data_outcome(result, op.is_read());
        self.client.0.preferred.store(peer, Ordering::Relaxed);
        finish(report)
    }
}

fn rpc(reason: Reason) -> Failure {
    Failure::Rpc { reason }
}
pub(crate) fn classify(status: &Status) -> Failure {
    if status.code() == Code::FailedPrecondition && exclusive(status.metadata(), &[REFUSED]) {
        if let Some(value) = single(status.metadata(), REFUSED).filter(|s| s.len() <= 256) {
            if let Ok(refusal) = serde_json::from_str::<Refusal>(value) {
                return Failure::Refused { refusal };
            }
        }
    }
    rpc(classify_status(status, true))
}
pub(super) fn data_outcome(result: Result<Reply, Failure>, read: bool) -> TtlOutcome {
    match result {
        Ok(reply) => TtlOutcome::Success { reply },
        Err(
            failure @ (Failure::Refused { .. }
            | Failure::Rpc {
                reason:
                    Reason::NotLeader { .. }
                    | Reason::AdmissionCount
                    | Reason::AdmissionBytes
                    | Reason::AdmissionOversize,
            }),
        ) => TtlOutcome::Refused { failure },
        Err(failure) if read => TtlOutcome::ReadFailure { failure },
        Err(failure) => TtlOutcome::UnknownWrite { failure },
    }
}
pub(super) fn expected(op: &Operation) -> usize {
    match op {
        Operation::Capabilities => CAPABILITIES,
        Operation::Get { keys } => keys.len(),
        Operation::Scan { .. } => SCAN,
        Operation::DeleteRange { .. } => DELETE_RANGE,
        _ => WRITE,
    }
}
pub(super) fn contained(op: &Operation, range: &DataRange) -> bool {
    match op {
        Operation::Capabilities => true,
        Operation::Get { keys } => keys.iter().all(|k| range.contains(k)),
        Operation::Put { pairs, .. } => pairs.iter().all(|(k, _)| range.contains(k)),
        Operation::Delete { key } => range.contains(key),
        Operation::Scan { start, end, .. } | Operation::DeleteRange { start, end, .. } => {
            range.contains(start)
                && (range.end.is_empty() || (!end.is_empty() && *end <= range.end))
        }
    }
}
pub(crate) fn valid_envelope(
    response: &tonic::Response<proto::RawTtlResponse>,
    expected: usize,
) -> bool {
    use proto::raw_ttl_response::Result as Wire;
    let r = response.get_ref();
    if has_control(response.metadata())
        || r.protocol_version != PROTOCOL_VERSION
        || r.binding_digest.len() != 32
        || r.encoded_len() > MAX_MESSAGE_BYTES
    {
        return false;
    }
    match &r.result {
        Some(Wire::Capabilities(_)) => expected == CAPABILITIES,
        Some(Wire::Values(v)) => {
            (1..=MAX_BATCH_ITEMS).contains(&expected)
                && v.values.len() == expected
                && v.values.iter().all(valid_optional)
        }
        Some(Wire::Applied(at)) => expected == WRITE && at.applied_term > 0 && at.applied_index > 0,
        Some(Wire::Page(p)) => {
            expected == SCAN
                && (p.next.is_none() || p.examined > 0)
                && p.pairs.len() <= MAX_BATCH_ITEMS
                && p.examined as usize <= MAX_SCAN_EXAMINED
                && p.examined as usize >= p.pairs.len()
                && p.pairs
                    .iter()
                    .all(|p| p.key.len() <= MAX_KEY_BYTES && p.value.len() <= MAX_VALUE_BYTES)
                && p.next
                    .as_ref()
                    .is_none_or(|k| crate::raw_ttl::valid_range_position(k))
        }
        Some(Wire::Deleted(p)) => {
            expected == DELETE_RANGE
                && p.processed_keys as usize <= MAX_BATCH_ITEMS
                && if p.processed_keys == 0 {
                    p.applied.is_none() && p.next.is_none()
                } else {
                    p.applied
                        .as_ref()
                        .is_some_and(|at| at.applied_term > 0 && at.applied_index > 0)
                        && p.next
                            .as_ref()
                            .is_none_or(|k| crate::raw_ttl::valid_range_position(k))
                }
        }
        _ => false,
    }
}
pub(super) fn decode(
    response: tonic::Response<proto::RawTtlResponse>,
    op: &Operation,
    range: &DataRange,
) -> Result<Reply, Failure> {
    use proto::raw_ttl_response::Result as Wire;
    if !valid_envelope(&response, expected(op))
        || response.get_ref().binding_digest != range.digest().as_bytes()
    {
        return Err(rpc(Reason::Protocol));
    }
    Ok(
        match response.into_inner().result.expect("validated result") {
            Wire::Capabilities(_) => Reply::Capabilities,
            Wire::Applied(at) => Reply::Applied {
                term: at.applied_term,
                index: at.applied_index,
            },
            Wire::Values(v) => Reply::Values {
                values: v
                    .values
                    .into_iter()
                    .map(|v| v.found.then_some(v.value))
                    .collect(),
            },
            Wire::Deleted(p) => {
                let Operation::DeleteRange { start, end, limit } = op else {
                    return Err(rpc(Reason::Protocol));
                };
                if p.processed_keys as usize > *limit
                    || p.next
                        .as_ref()
                        .is_some_and(|next| next <= start || (!end.is_empty() && next >= end))
                {
                    return Err(rpc(Reason::Protocol));
                }
                Reply::Deleted {
                    processed_keys: p.processed_keys as usize,
                    applied: p.applied.map(|at| crate::raw_ttl::Receipt {
                        term: at.applied_term,
                        index: at.applied_index,
                    }),
                    next: p.next,
                }
            }
            Wire::Page(p) => {
                let Operation::Scan { start, end, limit } = op else {
                    return Err(rpc(Reason::Protocol));
                };
                if p.pairs.len() > *limit
                    || p.pairs.iter().any(|p| {
                        p.key < *start
                            || (!end.is_empty() && p.key >= *end)
                            || !range.contains(&p.key)
                    })
                    || p.pairs.windows(2).any(|w| w[0].key >= w[1].key)
                    || p.next.as_ref().is_some_and(|next| {
                        next <= start
                            || (!end.is_empty() && next >= end)
                            || p.pairs.last().is_some_and(|last| last.key >= *next)
                    })
                {
                    return Err(rpc(Reason::Protocol));
                }
                Reply::Page {
                    rows: p.pairs.into_iter().map(|p| (p.key, p.value)).collect(),
                    next: p.next,
                    examined: p.examined as usize,
                }
            }
        },
    )
}

#[cfg(test)]
mod tests;
