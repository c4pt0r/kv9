//! Bounded asynchronous point client for a configured, unsplit raw keyspace.
//!
//! Channels survive logical operations. Only an exclusive, validated NotLeader
//! refusal permits another attempt; an uncertain write is NEVER retried. Dropping
//! a call future cancels client observation, not a possibly running backend job.
//! Workload callers must drain calls and retain their terminal reports.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use prost::Message;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;
use tokio::time::{sleep_until, timeout_at, Instant};
use tonic::metadata::{Ascii, KeyAndValueRef, MetadataMap, MetadataValue};
use tonic::transport::{Channel, Endpoint};
use tonic::{Code, Request, Status};

use crate::grpc::{ADMISSION_REFUSED_KEY, LEADER_HINT_KEY, NOT_LEADER_KEY, READ_UNCONFIRMED_KEY};
use crate::proto::{self, kv9_client::Kv9Client};

pub const MAX_PEERS: usize = 32;
pub const MAX_IN_FLIGHT: usize = 256;
pub const MAX_ATTEMPTS: usize = 16;
pub const MAX_KEY_BYTES: usize = 4096;
pub const MAX_VALUE_BYTES: usize = 65_536;
pub const MAX_MESSAGE_BYTES: usize = 1_048_576;
pub const MAX_BATCH_ITEMS: usize = 256;

/// Streaming is the normal transport; unary remains an explicit reference.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportKind {
    TonicUnary,
    #[default]
    TonicStream,
    #[cfg(feature = "rpc-experiment")]
    TarpcTcp,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Peer {
    pub node_id: u64,
    pub address: SocketAddr,
}

/// Static identities and addresses; leader hints cannot introduce endpoints.
/// Bounds are deliberately smaller than general server/transport limits.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientConfig {
    pub version: u32,
    pub peers: Vec<Peer>,
    pub keyspace_id: u32,
    pub epoch_conf_ver: u64,
    pub epoch_version: u64,
    pub max_in_flight: usize,
    pub max_attempts: usize,
    pub deadline_ms: u64,
    pub retry_backoff_ms: u64,
}

impl ClientConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.version != 1 {
            return Err("unsupported client configuration version");
        }
        if self.peers.is_empty() || self.peers.len() > MAX_PEERS {
            return Err("peer count must be within 1..=32");
        }
        // Bound the input before allocating these small sets or any channels.
        let mut ids = BTreeSet::new();
        let mut addresses = BTreeSet::new();
        for peer in &self.peers {
            if peer.node_id == 0 || !ids.insert(peer.node_id) {
                return Err("peer identities must be positive and distinct");
            }
            if peer.address.port() == 0 || !addresses.insert(peer.address) {
                return Err("peer addresses must have a port and be distinct");
            }
        }
        if !(1..=MAX_IN_FLIGHT).contains(&self.max_in_flight)
            || !(1..=MAX_ATTEMPTS).contains(&self.max_attempts)
            || !(1..=30_000).contains(&self.deadline_ms)
            || self.retry_backoff_ms > self.deadline_ms
        {
            return Err("client concurrency, attempts or time bounds are invalid");
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub enum RawOperation {
    Get { key: Vec<u8> },
    Put { key: Vec<u8>, value: Vec<u8> },
    Delete { key: Vec<u8> },
    BatchGet { keys: Vec<Vec<u8>> },
    BatchPut { pairs: Vec<(Vec<u8>, Vec<u8>)> },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Get,
    Put,
    Delete,
    BatchGet,
    BatchPut,
}

impl OperationKind {
    pub fn is_read(self) -> bool {
        matches!(self, Self::Get | Self::BatchGet)
    }
}

impl RawOperation {
    pub fn kind(&self) -> OperationKind {
        match self {
            Self::Get { .. } => OperationKind::Get,
            Self::Put { .. } => OperationKind::Put,
            Self::Delete { .. } => OperationKind::Delete,
            Self::BatchGet { .. } => OperationKind::BatchGet,
            Self::BatchPut { .. } => OperationKind::BatchPut,
        }
    }

    fn valid(&self, context: Option<proto::RequestContext>) -> bool {
        match self {
            Self::Get { key } | Self::Delete { key } => key.len() <= MAX_KEY_BYTES,
            Self::Put { key, value } => {
                key.len() <= MAX_KEY_BYTES && value.len() <= MAX_VALUE_BYTES
            }
            Self::BatchGet { keys } => {
                valid_batch_keys(keys)
                    && proto::RawBatchGetRequest {
                        context,
                        keys: keys.clone(),
                    }
                    .encoded_len()
                        <= MAX_MESSAGE_BYTES
            }
            Self::BatchPut { pairs } => {
                valid_batch_pairs(pairs)
                    && proto::RawBatchPutRequest {
                        context,
                        pairs: wire_pairs(pairs),
                    }
                    .encoded_len()
                        <= MAX_MESSAGE_BYTES
            }
        }
    }
}

pub(crate) fn valid_batch_keys(keys: &[Vec<u8>]) -> bool {
    (1..=MAX_BATCH_ITEMS).contains(&keys.len()) && keys.iter().all(|key| key.len() <= MAX_KEY_BYTES)
}

pub(crate) fn valid_batch_pairs(pairs: &[(Vec<u8>, Vec<u8>)]) -> bool {
    (1..=MAX_BATCH_ITEMS).contains(&pairs.len())
        && pairs
            .iter()
            .all(|(key, value)| key.len() <= MAX_KEY_BYTES && value.len() <= MAX_VALUE_BYTES)
        && pairs
            .iter()
            .map(|(key, value)| key.len() + value.len())
            .sum::<usize>()
            <= MAX_MESSAGE_BYTES
}

fn wire_pairs(pairs: &[(Vec<u8>, Vec<u8>)]) -> Vec<proto::KeyValue> {
    pairs
        .iter()
        .map(|(key, value)| proto::KeyValue {
            key: key.clone(),
            value: value.clone(),
        })
        .collect()
}

pub(crate) fn valid_optional(value: &proto::OptionalValue) -> bool {
    if value.found {
        value.value.len() <= MAX_VALUE_BYTES
    } else {
        value.value.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Value {
    Get { value: Option<Vec<u8>> },
    BatchGet { values: Vec<Option<Vec<u8>>> },
    Applied { term: u64, index: u64 },
}

/// No server prose, authentication metadata or user payload appears in reasons.
/// An unmarked RPC status does not establish whether the server executed a write.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Reason {
    NotLeader { leader: Option<u64> },
    AdmissionCount,
    AdmissionBytes,
    AdmissionOversize,
    ReadQuorumUnconfirmed,
    ReadApplyUnconfirmed,
    RpcStatus { code: i32 },
    Protocol,
    Deadline,
    ClientCapacity,
    ClientInput,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    Success { value: Value },
    Refused { reason: Reason },
    UnknownWrite { reason: Reason },
    ReadFailure { reason: Reason },
    ClientRejected { reason: Reason },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stop {
    Terminal,
    AttemptLimit,
    Deadline,
    ClientRejected,
}

#[derive(Clone, Debug, Serialize)]
pub struct Attempt {
    pub ordinal: usize,
    pub node_id: u64,
    pub elapsed_ns: u64,
    // Successful data is retained once, in the terminal result, not per attempt.
    pub failure: Option<Reason>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CallReport {
    pub operation: OperationKind,
    pub elapsed_ns: u64,
    pub attempts: Vec<Attempt>,
    pub stop: Stop,
    pub outcome: Outcome,
}

struct Inner {
    config: ClientConfig,
    clients: Vec<Kv9Client<Channel>>,
    #[cfg(feature = "rpc-experiment")]
    experimental: Option<Vec<crate::rpc_experiment::ExperimentClient>>,
    streaming: Option<Vec<crate::point_stream::StreamClient>>,
    authorization: MetadataValue<Ascii>,
    capacity: Arc<Semaphore>,
    preferred: AtomicUsize,
}

/// One lazily connected channel per configured peer, shared by all clones.
/// There is no synchronous dependency on a seed being reachable at construction.
#[derive(Clone)]
pub struct PersistentRawClient(Arc<Inner>);

impl PersistentRawClient {
    /// Must run inside a Tokio runtime. Secrets are excluded from config/reports.
    pub fn new(config: ClientConfig, token: &str) -> Result<Self, &'static str> {
        Self::new_with_transport(config, token, TransportKind::default())
    }

    fn new_base(config: ClientConfig, token: &str) -> Result<Self, &'static str> {
        config.validate()?;
        if token.is_empty() || token.len() > 4096 {
            return Err("authentication token length is invalid");
        }
        let mut authorization: MetadataValue<Ascii> = format!("Bearer {token}")
            .parse()
            .map_err(|_| "authentication token is invalid")?;
        authorization.set_sensitive(true);
        if tokio::runtime::Handle::try_current().is_err() {
            return Err("persistent client requires a Tokio runtime");
        }
        let clients = config
            .peers
            .iter()
            .map(|peer| {
                Endpoint::from_shared(format!("http://{}", peer.address))
                    .map(|endpoint| {
                        let channel = endpoint
                            .connect_timeout(Duration::from_millis(config.deadline_ms.min(1000)))
                            .buffer_size(config.max_in_flight)
                            .concurrency_limit(config.max_in_flight)
                            .connect_lazy();
                        Kv9Client::new(channel)
                            .max_encoding_message_size(MAX_MESSAGE_BYTES)
                            .max_decoding_message_size(MAX_MESSAGE_BYTES)
                    })
                    .map_err(|_| "invalid peer endpoint")
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self(Arc::new(Inner {
            capacity: Arc::new(Semaphore::new(config.max_in_flight)),
            config,
            clients,
            #[cfg(feature = "rpc-experiment")]
            experimental: None,
            streaming: None,
            authorization,
            preferred: AtomicUsize::new(0),
        })))
    }

    /// Explicit transport selection; retry and outcome classification stay shared.
    pub fn new_with_transport(
        config: ClientConfig,
        token: &str,
        transport: TransportKind,
    ) -> Result<Self, &'static str> {
        let mut client = Self::new_base(config, token)?;
        #[cfg(feature = "rpc-experiment")]
        if transport == TransportKind::TarpcTcp {
            let inner = Arc::get_mut(&mut client.0).ok_or("new client unexpectedly shared")?;
            inner.experimental = Some(
                inner
                    .config
                    .peers
                    .iter()
                    .map(|peer| {
                        crate::rpc_experiment::ExperimentClient::new(
                            peer.address,
                            inner.config.max_in_flight,
                        )
                    })
                    .collect(),
            );
        }
        if transport == TransportKind::TonicStream {
            let inner = Arc::get_mut(&mut client.0).ok_or("new client unexpectedly shared")?;
            inner.streaming = Some(
                inner
                    .config
                    .peers
                    .iter()
                    .map(|peer| {
                        crate::point_stream::StreamClient::new(
                            peer.address,
                            inner.config.max_in_flight,
                        )
                    })
                    .collect(),
            );
        }
        Ok(client)
    }

    /// One invocation, one immutable payload, one absolute monotonic deadline.
    /// Only NotLeader refusals can pass the retry edge below. Admission pressure
    /// remains visible as a terminal refusal. No transport failure is retried.
    pub async fn call(&self, operation: RawOperation) -> CallReport {
        let start = Instant::now();
        let deadline = start + Duration::from_millis(self.0.config.deadline_ms);
        let mut report = CallReport {
            operation: operation.kind(),
            elapsed_ns: 0,
            attempts: Vec::new(),
            stop: Stop::ClientRejected,
            outcome: Outcome::ClientRejected {
                reason: Reason::ClientInput,
            },
        };
        if !operation.valid(self.context()) {
            return finish(report, start);
        }
        let Ok(_permit) = self.0.capacity.clone().try_acquire_owned() else {
            report.outcome = Outcome::ClientRejected {
                reason: Reason::ClientCapacity,
            };
            return finish(report, start);
        };
        let mut peer = self.0.preferred.load(Ordering::Relaxed);
        for ordinal in 1..=self.0.config.max_attempts {
            if Instant::now() >= deadline {
                if report.attempts.is_empty() {
                    report.outcome = Outcome::ClientRejected {
                        reason: Reason::Deadline,
                    };
                }
                report.stop = Stop::Deadline;
                break;
            }
            let attempt_start = Instant::now();
            let result = timeout_at(deadline, self.dispatch(peer, &operation, deadline))
                .await
                .unwrap_or(Err(Reason::Deadline));
            report.attempts.push(Attempt {
                ordinal,
                node_id: self.0.config.peers[peer].node_id,
                elapsed_ns: nanos(attempt_start.elapsed()),
                failure: result.as_ref().err().cloned(),
            });
            report.stop = Stop::Terminal;
            match result {
                Ok(value) => {
                    self.0.preferred.store(peer, Ordering::Relaxed);
                    report.outcome = Outcome::Success { value };
                    break;
                }
                Err(reason @ Reason::NotLeader { .. }) => {
                    let Reason::NotLeader { leader } = reason else {
                        unreachable!()
                    };
                    peer = self.next_peer(peer, leader);
                    self.0.preferred.store(peer, Ordering::Relaxed);
                    report.outcome = Outcome::Refused { reason };
                    if ordinal == self.0.config.max_attempts {
                        report.stop = Stop::AttemptLimit;
                        break;
                    }
                    let wake = (Instant::now()
                        + Duration::from_millis(self.0.config.retry_backoff_ms))
                    .min(deadline);
                    sleep_until(wake).await;
                    // RETRY_EDGE: every preceding attempt proved no write effect.
                    continue;
                }
                Err(
                    reason @ (Reason::AdmissionCount
                    | Reason::AdmissionBytes
                    | Reason::AdmissionOversize),
                ) => {
                    report.outcome = Outcome::Refused { reason };
                    break;
                }
                Err(reason) => {
                    // Rotate for a NEW logical operation, never replay this one.
                    self.0
                        .preferred
                        .store(self.next_peer(peer, None), Ordering::Relaxed);
                    report.outcome = if operation.kind().is_read() {
                        Outcome::ReadFailure { reason }
                    } else {
                        Outcome::UnknownWrite { reason }
                    };
                    break;
                }
            }
        }
        finish(report, start)
    }

    /// One quorum-confirmed read view, with results in input order including duplicates.
    pub async fn batch_get(&self, keys: Vec<Vec<u8>>) -> CallReport {
        self.call(RawOperation::BatchGet { keys }).await
    }

    /// One atomic Raft command and one applied receipt for the whole batch.
    /// Empty/oversized batches are rejected locally; batches are never split.
    pub async fn batch_put(&self, pairs: Vec<(Vec<u8>, Vec<u8>)>) -> CallReport {
        self.call(RawOperation::BatchPut { pairs }).await
    }

    fn context(&self) -> Option<proto::RequestContext> {
        Some(proto::RequestContext {
            keyspace_id: self.0.config.keyspace_id,
            region_epoch: Some(proto::RegionEpoch {
                conf_ver: self.0.config.epoch_conf_ver,
                version: self.0.config.epoch_version,
            }),
        })
    }

    fn next_peer(&self, peer: usize, hint: Option<u64>) -> usize {
        hint.and_then(|id| self.0.config.peers.iter().position(|p| p.node_id == id))
            .filter(|next| *next != peer)
            .unwrap_or((peer + 1) % self.0.clients.len())
    }

    fn request<T>(&self, message: T, deadline: Instant) -> Request<T> {
        let mut request = Request::new(message);
        request
            .metadata_mut()
            .insert("authorization", self.0.authorization.clone());
        request.set_timeout(deadline.saturating_duration_since(Instant::now()));
        request
    }

    async fn raw_get(
        &self,
        peer: usize,
        request: Request<proto::RawGetRequest>,
        _deadline: Instant,
    ) -> Result<tonic::Response<proto::RawGetResponse>, Status> {
        #[cfg(feature = "rpc-experiment")]
        if let Some(clients) = &self.0.experimental {
            return clients[peer].raw_get(request, _deadline).await;
        }
        if let Some(clients) = &self.0.streaming {
            return clients[peer].raw_get(request, _deadline).await;
        }
        self.0.clients[peer].clone().raw_get(request).await
    }

    async fn raw_put(
        &self,
        peer: usize,
        request: Request<proto::RawPutRequest>,
        _deadline: Instant,
    ) -> Result<tonic::Response<proto::RawWriteResponse>, Status> {
        #[cfg(feature = "rpc-experiment")]
        if let Some(clients) = &self.0.experimental {
            return clients[peer].raw_put(request, _deadline).await;
        }
        if let Some(clients) = &self.0.streaming {
            return clients[peer].raw_put(request, _deadline).await;
        }
        self.0.clients[peer].clone().raw_put(request).await
    }

    async fn raw_delete(
        &self,
        peer: usize,
        request: Request<proto::RawDeleteRequest>,
        _deadline: Instant,
    ) -> Result<tonic::Response<proto::RawWriteResponse>, Status> {
        #[cfg(feature = "rpc-experiment")]
        if let Some(clients) = &self.0.experimental {
            return clients[peer].raw_delete(request, _deadline).await;
        }
        if let Some(clients) = &self.0.streaming {
            return clients[peer].raw_delete(request, _deadline).await;
        }
        self.0.clients[peer].clone().raw_delete(request).await
    }

    async fn raw_batch_get(
        &self,
        peer: usize,
        request: Request<proto::RawBatchGetRequest>,
        deadline: Instant,
    ) -> Result<tonic::Response<proto::RawBatchGetResponse>, Status> {
        #[cfg(feature = "rpc-experiment")]
        if let Some(clients) = &self.0.experimental {
            return clients[peer].raw_batch_get(request, deadline).await;
        }
        if let Some(clients) = &self.0.streaming {
            return clients[peer].raw_batch_get(request, deadline).await;
        }
        self.0.clients[peer].clone().raw_batch_get(request).await
    }

    async fn raw_batch_put(
        &self,
        peer: usize,
        request: Request<proto::RawBatchPutRequest>,
        deadline: Instant,
    ) -> Result<tonic::Response<proto::RawWriteResponse>, Status> {
        #[cfg(feature = "rpc-experiment")]
        if let Some(clients) = &self.0.experimental {
            return clients[peer].raw_batch_put(request, deadline).await;
        }
        if let Some(clients) = &self.0.streaming {
            return clients[peer].raw_batch_put(request, deadline).await;
        }
        self.0.clients[peer].clone().raw_batch_put(request).await
    }

    async fn dispatch(
        &self,
        peer: usize,
        operation: &RawOperation,
        deadline: Instant,
    ) -> Result<Value, Reason> {
        let context = self.context();
        match operation {
            RawOperation::Get { key } => {
                let response = self
                    .raw_get(
                        peer,
                        self.request(
                            proto::RawGetRequest {
                                context,
                                key: key.clone(),
                            },
                            deadline,
                        ),
                        deadline,
                    )
                    .await
                    .map_err(|status| classify_status(&status, true))?;
                if has_control(response.metadata()) {
                    return Err(Reason::Protocol);
                }
                match response.into_inner().value {
                    Some(value) if value.found && value.value.len() <= MAX_VALUE_BYTES => {
                        Ok(Value::Get {
                            value: Some(value.value),
                        })
                    }
                    Some(value) if !value.found && value.value.is_empty() => {
                        Ok(Value::Get { value: None })
                    }
                    _ => Err(Reason::Protocol),
                }
            }
            RawOperation::Put { key, value } => {
                let response = self
                    .raw_put(
                        peer,
                        self.request(
                            proto::RawPutRequest {
                                context,
                                key: key.clone(),
                                value: value.clone(),
                            },
                            deadline,
                        ),
                        deadline,
                    )
                    .await
                    .map_err(|status| classify_status(&status, false))?;
                applied(response)
            }
            RawOperation::BatchGet { keys } => {
                let request = self.request(
                    proto::RawBatchGetRequest {
                        context,
                        keys: keys.clone(),
                    },
                    deadline,
                );
                let response = self
                    .raw_batch_get(peer, request, deadline)
                    .await
                    .map_err(|status| classify_status(&status, true))?;
                if has_control(response.metadata())
                    || response.get_ref().encoded_len() > MAX_MESSAGE_BYTES
                {
                    return Err(Reason::Protocol);
                }
                let values = response.into_inner().values;
                if values.len() != keys.len() || !values.iter().all(valid_optional) {
                    return Err(Reason::Protocol);
                }
                Ok(Value::BatchGet {
                    values: values
                        .into_iter()
                        .map(|value| value.found.then_some(value.value))
                        .collect(),
                })
            }
            RawOperation::BatchPut { pairs } => {
                let request = self.request(
                    proto::RawBatchPutRequest {
                        context,
                        pairs: wire_pairs(pairs),
                    },
                    deadline,
                );
                applied(
                    self.raw_batch_put(peer, request, deadline)
                        .await
                        .map_err(|status| classify_status(&status, false))?,
                )
            }
            RawOperation::Delete { key } => {
                let response = self
                    .raw_delete(
                        peer,
                        self.request(
                            proto::RawDeleteRequest {
                                context,
                                key: key.clone(),
                            },
                            deadline,
                        ),
                        deadline,
                    )
                    .await
                    .map_err(|status| classify_status(&status, false))?;
                applied(response)
            }
        }
    }
}

fn applied(response: tonic::Response<proto::RawWriteResponse>) -> Result<Value, Reason> {
    if has_control(response.metadata()) {
        return Err(Reason::Protocol);
    }
    let at = response.into_inner();
    if at.applied_term == 0 || at.applied_index == 0 {
        return Err(Reason::Protocol);
    }
    Ok(Value::Applied {
        term: at.applied_term,
        index: at.applied_index,
    })
}

fn nanos(duration: Duration) -> u64 {
    // Logical budgets <=30s; saturate only if the executor was suspended for >584y.
    duration.as_nanos().min(u128::from(u64::MAX)) as u64
}

fn finish(mut report: CallReport, start: Instant) -> CallReport {
    report.elapsed_ns = nanos(start.elapsed());
    report
}

fn control_key(entry: KeyAndValueRef<'_>) -> &str {
    match entry {
        KeyAndValueRef::Ascii(key, _) => key.as_str(),
        KeyAndValueRef::Binary(key, _) => key.as_str(),
    }
}

pub(crate) fn has_control(metadata: &MetadataMap) -> bool {
    metadata
        .iter()
        .any(|entry| control_key(entry).starts_with("kv9-"))
}

fn exclusive(metadata: &MetadataMap, allowed: &[&str]) -> bool {
    metadata.iter().all(|entry| {
        let key = control_key(entry);
        !key.starts_with("kv9-") || allowed.contains(&key)
    })
}

fn single<'a>(metadata: &'a MetadataMap, key: &str) -> Option<&'a str> {
    let mut values = metadata.get_all(key).iter();
    let value = values.next()?.to_str().ok()?;
    values.next().is_none().then_some(value)
}

/// Fail closed on duplicated, mixed, unknown or malformed kv9 control metadata.
/// Unmarked statuses retain only their code and NEVER prove a write was refused.
pub(crate) fn classify_status(status: &Status, read: bool) -> Reason {
    let metadata = status.metadata();
    if metadata.contains_key(NOT_LEADER_KEY)
        && status.code() == Code::FailedPrecondition
        && exclusive(metadata, &[NOT_LEADER_KEY, LEADER_HINT_KEY])
        && single(metadata, NOT_LEADER_KEY) == Some("true")
    {
        let leader = if metadata.contains_key(LEADER_HINT_KEY) {
            let Some(text) = single(metadata, LEADER_HINT_KEY) else {
                return Reason::Protocol;
            };
            let Ok(id) = text.parse::<u64>() else {
                return Reason::Protocol;
            };
            if id == 0 || id.to_string() != text {
                return Reason::Protocol;
            }
            Some(id)
        } else {
            None
        };
        return Reason::NotLeader { leader };
    }
    if exclusive(metadata, &[ADMISSION_REFUSED_KEY]) {
        if let Some(reason) = crate::grpc::admission_refusal(status) {
            return match reason {
                "request_count" => Reason::AdmissionCount,
                "encoded_bytes" => Reason::AdmissionBytes,
                "request_too_large" => Reason::AdmissionOversize,
                _ => Reason::Protocol,
            };
        }
    }
    if read && status.code() == Code::Unavailable && exclusive(metadata, &[READ_UNCONFIRMED_KEY]) {
        match single(metadata, READ_UNCONFIRMED_KEY) {
            Some("quorum") => return Reason::ReadQuorumUnconfirmed,
            Some("apply") => return Reason::ReadApplyUnconfirmed,
            _ => {}
        }
    }
    if has_control(metadata) {
        Reason::Protocol
    } else {
        Reason::RpcStatus {
            code: status.code() as i32,
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod batch_tests;
