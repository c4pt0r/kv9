//! Root-pinned TTL discovery. Lookup and capability probes may retry; a data
//! invocation is dispatched at most once, under the original call deadline.
use super::*;
use crate::client::ttl::{self as scoped, Failure as TtlFailure, TtlOutcome};
use crate::raw_ttl::{Operation, Refusal, Reply, MAX_BUDGET_MICROS, PROTOCOL_VERSION};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Lookup,
    Capability,
    Data,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Failure {
    Lookup { failure: super::Failure },
    Ttl { failure: TtlFailure },
}
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    Success {
        reply: Reply,
    },
    Refused {
        failure: TtlFailure,
    },
    UnknownWrite {
        failure: TtlFailure,
    },
    ReadFailure {
        failure: TtlFailure,
    },
    CapabilityFailure {
        failure: TtlFailure,
    },
    LookupFailure {
        failure: super::Failure,
    },
    ClientRejected {
        reason: Reason,
    },
    /// Discovery/probing exhausted its budget without issuing a data request.
    NotDispatched,
}
impl From<TtlOutcome> for Outcome {
    fn from(outcome: TtlOutcome) -> Self {
        match outcome {
            TtlOutcome::Success { reply } => Self::Success { reply },
            TtlOutcome::Refused { failure } => Self::Refused { failure },
            TtlOutcome::UnknownWrite { failure } => Self::UnknownWrite { failure },
            TtlOutcome::ReadFailure { failure } => Self::ReadFailure { failure },
            TtlOutcome::CapabilityFailure { failure } => Self::CapabilityFailure { failure },
            TtlOutcome::ClientRejected { reason } => Self::ClientRejected { reason },
        }
    }
}
#[derive(Debug, Serialize)]
pub struct Attempt {
    pub phase: Phase,
    pub node_id: u64,
    pub route: Option<ScopeObservation>,
    pub elapsed_ns: u64,
    pub failure: Option<Failure>,
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub elapsed_ns: u64,
    pub attempts: Vec<Attempt>,
    pub stop: Stop,
    pub outcome: Outcome,
    /// Present only after a successful scan page with more work. An empty page
    /// can advance to another range. This is not a cluster-wide snapshot token.
    pub continuation: Option<ScanCursor>,
    /// Separate from scan continuation: resuming deletion is an explicit action.
    pub delete_continuation: Option<DeleteCursor>,
}
impl Report {
    fn new() -> Self {
        Self {
            elapsed_ns: 0,
            attempts: Vec::new(),
            stop: Stop::ClientRejected,
            outcome: Outcome::ClientRejected {
                reason: Reason::ClientInput,
            },
            continuation: None,
            delete_continuation: None,
        }
    }
    fn finish(mut self, start: Instant) -> Self {
        self.elapsed_ns = nanos(start.elapsed());
        self
    }
}

/// Versioned query position, not ownership authority. Re-resolve `next` under
/// current topology on continuation; never pin a retired group into the cursor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanCursor {
    version: u32,
    root_digest: [u8; 32],
    tenant_id: u64,
    keyspace_id: u32,
    start: Vec<u8>,
    end: Vec<u8>,
    next: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeleteCursor(ScanCursor);
impl DeleteCursor {
    pub fn next(&self) -> &[u8] {
        self.0.next()
    }
}
impl ScanCursor {
    pub fn next(&self) -> &[u8] {
        &self.next
    }
    fn valid(&self, config: &RoutedConfig, limit: usize) -> bool {
        self.version == 1
            && self.root_digest == config.root_digest
            && self.tenant_id == config.tenant_id
            && self.keyspace_id == config.keyspace_id
            && crate::raw_ttl::valid_range_position(&self.start)
            && crate::raw_ttl::valid_range_position(&self.next)
            && self.end.len() <= crate::raw_ttl::MAX_RANGE_BOUND_BYTES
            && self.next > self.start
            && Operation::Scan {
                start: self.start.clone(),
                end: self.end.clone(),
                limit,
            }
            .validate()
            .is_ok()
            && Operation::Scan {
                start: self.next.clone(),
                end: self.end.clone(),
                limit,
            }
            .validate()
            .is_ok()
    }
}

#[derive(Clone)]
pub struct RoutedTtlClient(RoutedRawClient);
impl RoutedTtlClient {
    pub fn new(config: RoutedConfig, token: &str) -> Result<Self, &'static str> {
        Self::new_with_transport(config, token, TransportKind::TonicStream)
    }
    pub fn new_with_transport(
        config: RoutedConfig,
        token: &str,
        transport: TransportKind,
    ) -> Result<Self, &'static str> {
        if config.max_attempts < 3 {
            return Err("TTL routing requires room for discovery, capability and data");
        }
        RoutedRawClient::new_with_transport(config, token, transport).map(Self)
    }
    #[cfg(test)]
    pub(crate) fn inject_test_channel(&self, address: SocketAddr, channel: Channel) {
        self.0.inject_test_channel(address, channel);
    }

    pub async fn continue_scan(&self, cursor: &ScanCursor, limit: usize) -> Report {
        if !cursor.valid(&self.0 .0.config, limit) {
            return Report::new();
        }
        self.call_inner(
            Operation::Scan {
                start: cursor.next.clone(),
                end: cursor.end.clone(),
                limit,
            },
            Some(cursor.start.clone()),
        )
        .await
    }

    pub async fn continue_delete_range(&self, cursor: &DeleteCursor, limit: usize) -> Report {
        if !cursor.0.valid(&self.0 .0.config, limit) {
            return Report::new();
        }
        self.call_inner(
            Operation::DeleteRange {
                start: cursor.0.next.clone(),
                end: cursor.0.end.clone(),
                limit,
            },
            Some(cursor.0.start.clone()),
        )
        .await
    }

    /// Get/Put batches must fit ONE range. Scan reads at most one range per
    /// page and returns a continuation across boundaries, including empty ones.
    /// Explicit activation still requires a caller-reviewed exact binding.
    pub async fn call(&self, operation: Operation) -> Report {
        self.call_inner(operation, None).await
    }

    async fn dispatch(
        &self,
        peer: &Peer,
        route: &Route,
        op: &Operation,
        deadline: Instant,
    ) -> Result<Reply, TtlFailure> {
        let budget_micros = deadline
            .saturating_duration_since(Instant::now())
            .as_micros()
            .min(MAX_BUDGET_MICROS as u128) as u64;
        if budget_micros == 0 {
            return Err(ttl_rpc(Reason::Deadline));
        }
        let body = proto::RawTtlRequest {
            binding: route.binding.encode(),
            protocol_version: PROTOCOL_VERSION,
            budget_micros,
            operation: Some(op.clone().into()),
        };
        if body.encoded_len() > MAX_MESSAGE_BYTES {
            return Err(ttl_rpc(Reason::ClientInput));
        }
        let connection = self.0.connection(peer.address);
        let request = self.0.request(body, deadline);
        let result = timeout_at(deadline, async {
            if self.0 .0.transport == TransportKind::TonicStream {
                connection
                    .stream
                    .raw_ttl(request, scoped::expected(op), deadline)
                    .await
            } else {
                connection.unary.clone().raw_ttl(request).await
            }
        })
        .await
        .map_err(|_| ttl_rpc(Reason::Deadline))?
        .map_err(|s| scoped::classify(&s))?;
        scoped::decode(result, op, &route.binding)
    }

    async fn call_inner(&self, operation: Operation, scan_origin: Option<Vec<u8>>) -> Report {
        let started = Instant::now();
        let config = &self.0 .0.config;
        let deadline = started + Duration::from_millis(config.deadline_ms);
        let mut report = Report::new();
        if operation.validate().is_err() {
            return report.finish(started);
        }
        let key = match &operation {
            Operation::Get { keys } => &keys[0],
            Operation::Put { pairs, .. } => &pairs[0].0,
            Operation::Delete { key } => key,
            Operation::Scan { start, .. } | Operation::DeleteRange { start, .. } => start,
            Operation::Capabilities => return report.finish(started),
        };
        let Ok(_permit) = self.0 .0.capacity.clone().try_acquire_owned() else {
            report.outcome = Outcome::ClientRejected {
                reason: Reason::ClientCapacity,
            };
            return report.finish(started);
        };
        let mut route = self.0.cached(key);
        let mut fresh = false;
        let mut confirmed: Option<Peer> = None;
        let mut tried = BTreeSet::new();
        let mut lookup_tried = BTreeSet::new();
        let mut lookup_cursor = self.0 .0.metadata_preferred.load(Ordering::Relaxed);
        report.outcome = Outcome::NotDispatched;
        report.stop = Stop::AttemptLimit;
        while report.attempts.len() < config.max_attempts {
            if Instant::now() >= deadline {
                report.stop = Stop::Deadline;
                break;
            }
            let attempt_start = Instant::now();
            if let Some(current) = &mut route {
                let local_op = clip(&operation, &current.binding);
                if !scoped::contained(&local_op, &current.binding) {
                    // A stale cached split cannot permanently reject a batch
                    // that may fit after a later metadata publication.
                    if !fresh {
                        self.0.invalidate(current.binding.digest());
                        route = None;
                        continue;
                    }
                    report.outcome = Outcome::Refused {
                        failure: TtlFailure::Refused {
                            refusal: Refusal::CrossRange,
                        },
                    };
                    report.stop = Stop::Terminal;
                    break;
                }
                if local_op.validate().is_err()
                    || (proto::RawTtlRequest {
                        binding: current.binding.encode(),
                        protocol_version: PROTOCOL_VERSION,
                        budget_micros: MAX_BUDGET_MICROS,
                        operation: Some(local_op.clone().into()),
                    })
                    .encoded_len()
                        > MAX_MESSAGE_BYTES
                {
                    report.outcome = Outcome::ClientRejected {
                        reason: Reason::ClientInput,
                    };
                    report.stop = Stop::ClientRejected;
                    break;
                }
                if let Some(peer) = confirmed.take() {
                    let result = self.dispatch(&peer, current, &local_op, deadline).await;
                    report.attempts.push(Attempt {
                        phase: Phase::Data,
                        node_id: peer.node_id,
                        route: Some((&current.binding).into()),
                        elapsed_ns: nanos(attempt_start.elapsed()),
                        failure: result
                            .as_ref()
                            .err()
                            .cloned()
                            .map(|failure| Failure::Ttl { failure }),
                    });
                    match &result {
                        Ok(_) => {
                            current.leader = Some(peer.node_id);
                            self.0.cache(current.clone());
                            self.0
                                 .0
                                .suspect
                                .lock()
                                .expect("suspects poisoned")
                                .remove(&peer.address);
                        }
                        Err(_) => {
                            self.0.invalidate(current.binding.digest());
                            self.0.suspect(&peer);
                        }
                    }
                    let result = result.map(|reply| {
                        self.page(
                            reply,
                            &operation,
                            &current.binding,
                            scan_origin.as_deref(),
                            &mut report,
                        )
                    });
                    report.outcome = scoped::data_outcome(result, operation.is_read()).into();
                    report.stop = Stop::Terminal;
                    break; // Never replay a data invocation, including known refusals.
                }
                let peer = self.0.choose(current, &tried);
                let probe_deadline =
                    deadline.min(Instant::now() + Duration::from_millis(config.probe_timeout_ms));
                let result = self
                    .dispatch(&peer, current, &Operation::Capabilities, probe_deadline)
                    .await;
                report.attempts.push(Attempt {
                    phase: Phase::Capability,
                    node_id: peer.node_id,
                    route: Some((&current.binding).into()),
                    elapsed_ns: nanos(attempt_start.elapsed()),
                    failure: result
                        .as_ref()
                        .err()
                        .cloned()
                        .map(|failure| Failure::Ttl { failure }),
                });
                match result {
                    Ok(_) => {
                        confirmed = Some(peer);
                        report.outcome = Outcome::NotDispatched;
                        continue;
                    }
                    Err(failure) => {
                        let stale = matches!(
                            failure,
                            TtlFailure::Refused {
                                refusal: Refusal::StaleScope
                            }
                        );
                        let retry = stale || probe_retry(&failure);
                        if let TtlFailure::Rpc {
                            reason: Reason::NotLeader { leader },
                        } = &failure
                        {
                            current.leader = *leader;
                        }
                        report.outcome = Outcome::CapabilityFailure { failure };
                        if !retry {
                            report.stop = Stop::Terminal;
                            break;
                        }
                        tried.insert(peer.node_id);
                        if stale || tried.len() == current.replicas.len() {
                            self.0.invalidate(current.binding.digest());
                            route = None;
                            tried.clear();
                        }
                    }
                }
            } else {
                let peer = {
                    let peers = self.0 .0.metadata.lock().expect("metadata routes poisoned");
                    if peers.iter().all(|p| lookup_tried.contains(&p.node_id)) {
                        lookup_tried.clear();
                    }
                    let peer = peers
                        .iter()
                        .cycle()
                        .skip(lookup_cursor % peers.len())
                        .take(peers.len())
                        .find(|p| !lookup_tried.contains(&p.node_id))
                        .expect("untried metadata peer")
                        .clone();
                    lookup_tried.insert(peer.node_id);
                    peer
                };
                let probe_deadline =
                    deadline.min(Instant::now() + Duration::from_millis(config.probe_timeout_ms));
                let result = self.0.lookup_once(&peer, key, probe_deadline).await;
                report.attempts.push(Attempt {
                    phase: Phase::Lookup,
                    node_id: peer.node_id,
                    route: None,
                    elapsed_ns: nanos(attempt_start.elapsed()),
                    failure: result
                        .as_ref()
                        .err()
                        .cloned()
                        .map(|failure| Failure::Lookup { failure }),
                });
                match result {
                    Ok((metadata, leader, found)) => {
                        self.0.learn_metadata(metadata);
                        let peers = self.0 .0.metadata.lock().expect("metadata routes poisoned");
                        lookup_cursor = leader
                            .and_then(|id| {
                                peers
                                    .iter()
                                    .position(|p| p.node_id == id && !lookup_tried.contains(&id))
                            })
                            .unwrap_or((lookup_cursor + 1) % peers.len());
                        if let Some(found) = found {
                            self.0 .0.metadata_preferred.store(
                                peers
                                    .iter()
                                    .position(|p| p.node_id == peer.node_id)
                                    .unwrap_or(0),
                                Ordering::Relaxed,
                            );
                            self.0.cache(found.clone());
                            route = Some(found);
                            fresh = true;
                            report.outcome = Outcome::NotDispatched;
                            continue;
                        }
                        report.outcome = Outcome::LookupFailure {
                            failure: super::rpc(Reason::NotLeader { leader }),
                        };
                    }
                    Err(failure) => {
                        lookup_cursor += 1;
                        report.outcome = Outcome::LookupFailure { failure };
                    }
                }
            }
            if report.attempts.len() < config.max_attempts {
                sleep_until(
                    deadline.min(Instant::now() + Duration::from_millis(config.retry_backoff_ms)),
                )
                .await;
            }
        }
        report.finish(started)
    }

    fn page(
        &self,
        mut reply: Reply,
        operation: &Operation,
        range: &DataRange,
        origin: Option<&[u8]>,
        report: &mut Report,
    ) -> Reply {
        let bounds = match operation {
            Operation::Scan { start, end, .. } | Operation::DeleteRange { start, end, .. } => {
                Some((start, end))
            }
            _ => None,
        };
        if let (Some((start, end)), Reply::Page { next, .. } | Reply::Deleted { next, .. }) =
            (bounds, &mut reply)
        {
            if next.is_none() && !range.end.is_empty() && (end.is_empty() || range.end < *end) {
                *next = Some(range.end.clone());
            }
            let cursor = next.as_ref().map(|next| ScanCursor {
                version: 1,
                root_digest: self.0 .0.config.root_digest,
                tenant_id: self.0 .0.config.tenant_id,
                keyspace_id: self.0 .0.config.keyspace_id,
                start: origin.unwrap_or(start).to_vec(),
                end: end.clone(),
                next: next.clone(),
            });
            if matches!(operation, Operation::DeleteRange { .. }) {
                report.delete_continuation = cursor.map(DeleteCursor);
            } else {
                report.continuation = cursor;
            }
        }
        reply
    }
}

fn ttl_rpc(reason: Reason) -> TtlFailure {
    TtlFailure::Rpc { reason }
}
fn probe_retry(failure: &TtlFailure) -> bool {
    matches!(
        failure,
        TtlFailure::Rpc {
            reason: Reason::NotLeader { .. }
                | Reason::Deadline
                | Reason::ReadQuorumUnconfirmed
                | Reason::ReadApplyUnconfirmed
                | Reason::RpcStatus { code: 4 | 14 }
        }
    )
}
fn clip(operation: &Operation, range: &DataRange) -> Operation {
    match operation {
        Operation::Scan { start, end, limit } => Operation::Scan {
            start: start.clone(),
            limit: *limit,
            end: if !range.end.is_empty() && (end.is_empty() || range.end < *end) {
                range.end.clone()
            } else {
                end.clone()
            },
        },
        Operation::DeleteRange { start, end, limit } => Operation::DeleteRange {
            start: start.clone(),
            limit: *limit,
            end: if !range.end.is_empty() && (end.is_empty() || range.end < *end) {
                range.end.clone()
            } else {
                end.clone()
            },
        },
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests;
