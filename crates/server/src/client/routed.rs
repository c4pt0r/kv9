//! Bounded discovery and scoped calls over the selected persistent transport.
//! Unknown writes terminate observation; only proven refusals cross a retry edge.
use super::*;
use kv9_common::{data_range::DataRange, RootDigest};
use std::collections::BTreeMap;
use std::sync::Mutex;

const ROUTE_REFUSED: &str = "kv9-route-refused";
pub const MAX_CACHED_ROUTES: usize = 128;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutedConfig {
    pub version: u32,
    pub root_digest: [u8; 32],
    pub tenant_id: u64,
    pub keyspace_id: u32,
    pub seeds: Vec<Peer>,
    pub max_in_flight: usize,
    /// Includes discovery, redirects and data dispatches together.
    pub max_attempts: usize,
    pub deadline_ms: u64,
    pub probe_timeout_ms: u64,
    pub retry_backoff_ms: u64,
    pub cache_capacity: usize,
}
impl RoutedConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.version != 1
            || self.root_digest == [0; 32]
            || self.seeds.len() < 2
            || self.keyspace_id == 0
            || self.keyspace_id > kv9_common::KeyspaceId::MAX
            || !(1..=MAX_CACHED_ROUTES).contains(&self.cache_capacity)
            || self.probe_timeout_ms == 0
            || self.probe_timeout_ms > self.deadline_ms
        {
            return Err("invalid scoped routing configuration");
        }
        ClientConfig {
            version: 1,
            peers: self.seeds.clone(),
            keyspace_id: self.keyspace_id,
            epoch_conf_ver: 1,
            epoch_version: 1,
            max_in_flight: self.max_in_flight,
            max_attempts: self.max_attempts,
            deadline_ms: self.deadline_ms,
            retry_backoff_ms: self.retry_backoff_ms,
        }
        .validate()?;
        if self.seeds.iter().any(|p| p.address.ip().is_unspecified()) {
            return Err("route seed address must be usable");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Failure {
    ScopeRefused,
    CrossRange,
    InvalidRoute,
    RequestBounds,
    Rpc { reason: Reason },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RoutedOutcome {
    Success { value: Value },
    Refused { failure: Failure },
    UnknownWrite { failure: Failure },
    ReadFailure { failure: Failure },
    LookupFailure { failure: Failure },
    ClientRejected { reason: Reason },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Lookup,
    Data,
}
#[derive(Clone, Debug, Serialize)]
pub struct RoutedAttempt {
    pub route: Option<ScopeObservation>,
    pub ordinal: usize,
    pub phase: Phase,
    pub node_id: u64,
    pub elapsed_ns: u64,
    pub failure: Option<Failure>,
}
#[derive(Clone, Debug, Serialize)]
pub struct ScopeObservation {
    pub region_id: u64,
    pub epoch_conf_ver: u64,
    pub epoch_version: u64,
    pub binding_digest: [u8; 32],
}
impl From<&DataRange> for ScopeObservation {
    fn from(range: &DataRange) -> Self {
        Self {
            region_id: range.region.0,
            epoch_conf_ver: range.conf_ver,
            epoch_version: range.version,
            binding_digest: *range.digest().as_bytes(),
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct RoutedReport {
    pub operation: OperationKind,
    pub elapsed_ns: u64,
    pub attempts: Vec<RoutedAttempt>,
    pub stop: Stop,
    pub outcome: RoutedOutcome,
}

#[derive(Clone)]
struct Route {
    binding: DataRange,
    replicas: Vec<Peer>,
    leader: Option<u64>,
}
struct Connection {
    unary: Kv9Client<Channel>,
    stream: crate::point_stream::StreamClient,
}
struct RoutedInner {
    config: RoutedConfig,
    transport: TransportKind,
    authorization: MetadataValue<Ascii>,
    capacity: Arc<Semaphore>,
    metadata: Mutex<Vec<Peer>>,
    metadata_preferred: AtomicUsize,
    routes: Mutex<Vec<Route>>,
    connections: Mutex<BTreeMap<SocketAddr, Arc<Connection>>>,
    /// Avoid repeatedly selecting an endpoint with an uncertain transport result.
    /// This affects new logical calls, never authorization or write replay.
    suspect: Mutex<BTreeSet<SocketAddr>>,
}
#[derive(Clone)]
pub struct RoutedRawClient(Arc<RoutedInner>);

impl RoutedRawClient {
    pub fn new(config: RoutedConfig, token: &str) -> Result<Self, &'static str> {
        Self::new_with_transport(config, token, TransportKind::TonicStream)
    }
    pub fn new_with_transport(
        config: RoutedConfig,
        token: &str,
        transport: TransportKind,
    ) -> Result<Self, &'static str> {
        config.validate()?;
        if !matches!(
            transport,
            TransportKind::TonicStream | TransportKind::TonicUnary
        ) {
            return Err("scoped routing requires a supported scoped transport");
        }
        if token.is_empty() || token.len() > 4096 || tokio::runtime::Handle::try_current().is_err()
        {
            return Err("routing requires a valid token and Tokio runtime");
        }
        let mut authorization: MetadataValue<Ascii> = format!("Bearer {token}")
            .parse()
            .map_err(|_| "invalid routing authentication token")?;
        authorization.set_sensitive(true);
        Ok(Self(Arc::new(RoutedInner {
            metadata: Mutex::new(config.seeds.clone()),
            capacity: Arc::new(Semaphore::new(config.max_in_flight)),
            config,
            transport,
            authorization,
            metadata_preferred: AtomicUsize::new(0),
            routes: Mutex::new(Vec::new()),
            connections: Mutex::new(BTreeMap::new()),
            suspect: Mutex::new(BTreeSet::new()),
        })))
    }
    fn connection(&self, address: SocketAddr) -> Arc<Connection> {
        let mut all = self
            .0
            .connections
            .lock()
            .expect("route connections poisoned");
        if let Some(existing) = all.get(&address) {
            return existing.clone();
        }
        if all.len() == MAX_PEERS {
            let first = *all.keys().next().expect("bounded nonempty connections");
            all.remove(&first);
        }
        let channel = Endpoint::from_shared(format!("http://{address}"))
            .expect("numeric endpoint")
            .connect_timeout(Duration::from_millis(self.0.config.probe_timeout_ms))
            .buffer_size(self.0.config.max_in_flight)
            .concurrency_limit(self.0.config.max_in_flight)
            .connect_lazy();
        let connection = Arc::new(Connection {
            unary: Kv9Client::new(channel)
                .max_encoding_message_size(MAX_MESSAGE_BYTES)
                .max_decoding_message_size(MAX_MESSAGE_BYTES),
            stream: crate::point_stream::StreamClient::new(address, self.0.config.max_in_flight),
        });
        all.insert(address, connection.clone());
        connection
    }
    fn request<T>(&self, body: T, deadline: Instant) -> Request<T> {
        let mut request = Request::new(body);
        request
            .metadata_mut()
            .insert("authorization", self.0.authorization.clone());
        request.set_timeout(deadline.saturating_duration_since(Instant::now()));
        request
    }
    fn cached(&self, key: &[u8]) -> Option<Route> {
        self.0
            .routes
            .lock()
            .expect("routes poisoned")
            .iter()
            .find(|r| r.binding.contains(key))
            .cloned()
    }
    fn invalidate(&self, digest: RootDigest) {
        self.0
            .routes
            .lock()
            .expect("routes poisoned")
            .retain(|r| r.binding.digest() != digest);
    }
    fn cache(&self, route: Route) {
        let mut routes = self.0.routes.lock().expect("routes poisoned");
        // Cached observations are hints: remove any overlapping prior owner.
        routes.retain(|old| !overlaps(&old.binding, &route.binding));
        if routes.len() == self.0.config.cache_capacity {
            routes.remove(0);
        }
        routes.push(route);
    }
    fn learn_metadata(&self, peers: Vec<Peer>) {
        let mut known = self.0.metadata.lock().expect("metadata routes poisoned");
        for peer in peers {
            if let Some(old) = known.iter_mut().find(|p| p.node_id == peer.node_id) {
                *old = peer;
            } else if known.len() < MAX_PEERS {
                known.push(peer);
            }
        }
    }
    fn suspect(&self, peer: &Peer) {
        let mut suspects = self.0.suspect.lock().expect("suspects poisoned");
        if suspects.len() == MAX_PEERS {
            suspects.clear();
        }
        suspects.insert(peer.address);
    }
    fn choose(&self, route: &Route, tried: &BTreeSet<u64>) -> Peer {
        let suspects = self.0.suspect.lock().expect("suspects poisoned");
        route
            .replicas
            .iter()
            .find(|p| {
                Some(p.node_id) == route.leader
                    && !tried.contains(&p.node_id)
                    && !suspects.contains(&p.address)
            })
            .or_else(|| {
                route
                    .replicas
                    .iter()
                    .find(|p| !tried.contains(&p.node_id) && !suspects.contains(&p.address))
            })
            .or_else(|| route.replicas.iter().find(|p| !tried.contains(&p.node_id)))
            .unwrap_or(&route.replicas[0])
            .clone()
    }
    async fn lookup_once(
        &self,
        peer: &Peer,
        key: &[u8],
        deadline: Instant,
    ) -> Result<(Vec<Peer>, Option<u64>, Option<Route>), Failure> {
        let connection = self.connection(peer.address);
        let request = self.request(
            proto::LookupRawRouteRequest {
                root_digest: self.0.config.root_digest.to_vec(),
                tenant_id: self.0.config.tenant_id,
                keyspace_id: self.0.config.keyspace_id,
                key: key.to_vec(),
            },
            deadline,
        );
        let response = timeout_at(deadline, connection.unary.clone().lookup_raw_route(request))
            .await
            .map_err(|_| rpc(Reason::Deadline))?
            .map_err(|s| classify(&s, true))?;
        if has_control(response.metadata()) {
            return Err(Failure::InvalidRoute);
        }
        let response = response.into_inner();
        if response.root_digest.as_slice() != self.0.config.root_digest {
            return Err(Failure::InvalidRoute);
        }
        let metadata = peers(response.metadata_peers, false)?;
        if metadata.is_empty()
            || response
                .metadata_leader
                .is_some_and(|id| !metadata.iter().any(|p| p.node_id == id))
        {
            return Err(Failure::InvalidRoute);
        }
        if response.binding.is_empty() {
            if !response.replicas.is_empty() || response.data_leader.is_some() {
                return Err(Failure::InvalidRoute);
            }
            return Ok((metadata, response.metadata_leader, None));
        }
        let binding = DataRange::decode(&response.binding).map_err(|_| Failure::InvalidRoute)?;
        if binding.root.as_bytes() != &self.0.config.root_digest
            || binding.tenant.0 != self.0.config.tenant_id
            || binding.keyspace.0 != self.0.config.keyspace_id
            || binding.sealed
            || !binding.contains(key)
        {
            return Err(Failure::InvalidRoute);
        }
        let replicas = peers(response.replicas, true)?;
        if response
            .data_leader
            .is_some_and(|id| !replicas.iter().any(|p| p.node_id == id))
        {
            return Err(Failure::InvalidRoute);
        }
        Ok((
            metadata,
            response.metadata_leader,
            Some(Route {
                binding,
                replicas,
                leader: response.data_leader,
            }),
        ))
    }
    async fn dispatch(
        &self,
        peer: &Peer,
        route: &Route,
        operation: &RawOperation,
        deadline: Instant,
    ) -> Result<Value, Failure> {
        use proto::routed_raw_request::Operation;
        let (body, count) = match operation {
            RawOperation::Get { key } => (
                Operation::Get(proto::RoutedKeys {
                    keys: vec![key.clone()],
                }),
                1,
            ),
            RawOperation::BatchGet { keys } => (
                Operation::Get(proto::RoutedKeys { keys: keys.clone() }),
                keys.len(),
            ),
            RawOperation::Put { key, value } => (
                Operation::Put(proto::RoutedPairs {
                    pairs: vec![proto::KeyValue {
                        key: key.clone(),
                        value: value.clone(),
                    }],
                }),
                0,
            ),
            RawOperation::BatchPut { pairs } => (
                Operation::Put(proto::RoutedPairs {
                    pairs: wire_pairs(pairs),
                }),
                0,
            ),
            RawOperation::Delete { key } => (Operation::DeleteKey(key.clone()), 0),
        };
        let request = proto::RoutedRawRequest {
            binding: route.binding.encode(),
            operation: Some(body),
        };
        if request.encoded_len() > MAX_MESSAGE_BYTES {
            return Err(Failure::RequestBounds);
        }
        let connection = self.connection(peer.address);
        let request = self.request(request, deadline);
        let response = if self.0.transport == TransportKind::TonicStream {
            connection.stream.routed_raw(request, count, deadline).await
        } else {
            connection.unary.clone().routed_raw(request).await
        }
        .map_err(|s| classify(&s, operation.kind().is_read()))?;
        if !valid_reply(&response, count)
            || response.get_ref().binding_digest != route.binding.digest().as_bytes()
        {
            return Err(rpc(Reason::Protocol));
        }
        match response.into_inner().result.expect("validated result") {
            proto::routed_raw_response::Result::Applied(at) => {
                applied(tonic::Response::new(at)).map_err(rpc)
            }
            proto::routed_raw_response::Result::Values(values) => {
                let mut values: Vec<_> = values
                    .values
                    .into_iter()
                    .map(|v| v.found.then_some(v.value))
                    .collect();
                if matches!(operation, RawOperation::Get { .. }) {
                    Ok(Value::Get {
                        value: values.remove(0),
                    })
                } else {
                    Ok(Value::BatchGet { values })
                }
            }
        }
    }

    /// One logical call owns one absolute deadline and shared attempt budget.
    /// Directory reads may fail over; data writes never retry an uncertain result.
    pub async fn call(&self, operation: RawOperation) -> RoutedReport {
        let start = Instant::now();
        let deadline = start + Duration::from_millis(self.0.config.deadline_ms);
        let mut report = RoutedReport {
            operation: operation.kind(),
            elapsed_ns: 0,
            attempts: Vec::new(),
            stop: Stop::ClientRejected,
            outcome: RoutedOutcome::ClientRejected {
                reason: Reason::ClientInput,
            },
        };
        if !operation.valid(None) {
            return finish_routed(report, start);
        }
        let keys = keys(&operation);
        let Some(key) = keys.first() else {
            return finish_routed(report, start);
        };
        let Ok(_permit) = self.0.capacity.clone().try_acquire_owned() else {
            report.outcome = RoutedOutcome::ClientRejected {
                reason: Reason::ClientCapacity,
            };
            return finish_routed(report, start);
        };
        let mut route = self.cached(key);
        let mut tried = BTreeSet::new();
        let mut lookup_tried = BTreeSet::new();
        let mut lookup_cursor = self.0.metadata_preferred.load(Ordering::Relaxed);
        for ordinal in 1..=self.0.config.max_attempts {
            if Instant::now() >= deadline {
                if report.attempts.is_empty() {
                    report.outcome = RoutedOutcome::ClientRejected {
                        reason: Reason::Deadline,
                    };
                }
                report.stop = Stop::Deadline;
                break;
            }
            report.stop = Stop::AttemptLimit;
            let attempt_start = Instant::now();
            if let Some(current) = &mut route {
                if !keys.iter().all(|k| current.binding.contains(k)) {
                    report.outcome = RoutedOutcome::Refused {
                        failure: Failure::CrossRange,
                    };
                    report.stop = Stop::Terminal;
                    break;
                }
                let peer = self.choose(current, &tried);
                let attempt_deadline = if operation.kind().is_read() {
                    deadline
                        .min(Instant::now() + Duration::from_millis(self.0.config.probe_timeout_ms))
                } else {
                    deadline
                };
                let result = timeout_at(
                    attempt_deadline,
                    self.dispatch(&peer, current, &operation, attempt_deadline),
                )
                .await
                .unwrap_or_else(|_| Err(rpc(Reason::Deadline)));
                report.attempts.push(RoutedAttempt {
                    ordinal,
                    phase: Phase::Data,
                    route: Some((&current.binding).into()),
                    node_id: peer.node_id,
                    elapsed_ns: nanos(attempt_start.elapsed()),
                    failure: result.as_ref().err().cloned(),
                });
                match result {
                    Ok(value) => {
                        self.0
                            .suspect
                            .lock()
                            .expect("suspects poisoned")
                            .remove(&peer.address);
                        current.leader = Some(peer.node_id);
                        self.cache(current.clone());
                        report.outcome = RoutedOutcome::Success { value };
                        report.stop = Stop::Terminal;
                        break;
                    }
                    Err(failure @ Failure::ScopeRefused) => {
                        self.invalidate(current.binding.digest());
                        route = None;
                        tried.clear();
                        report.outcome = RoutedOutcome::Refused { failure };
                    }
                    Err(
                        failure @ Failure::Rpc {
                            reason: Reason::NotLeader { .. },
                        },
                    ) => {
                        if let Failure::Rpc {
                            reason: Reason::NotLeader { leader },
                        } = &failure
                        {
                            current.leader = *leader;
                        }
                        tried.insert(peer.node_id);
                        report.outcome = RoutedOutcome::Refused { failure };
                        if tried.len() == current.replicas.len() {
                            self.invalidate(current.binding.digest());
                            route = None;
                            tried.clear();
                        }
                    }
                    Err(
                        failure @ (Failure::CrossRange
                        | Failure::RequestBounds
                        | Failure::Rpc {
                            reason:
                                Reason::AdmissionCount
                                | Reason::AdmissionBytes
                                | Reason::AdmissionOversize,
                        }),
                    ) => {
                        report.outcome = RoutedOutcome::Refused { failure };
                        report.stop = Stop::Terminal;
                        break;
                    }
                    Err(failure) => {
                        self.suspect(&peer);
                        self.invalidate(current.binding.digest());
                        if operation.kind().is_read() {
                            report.outcome = RoutedOutcome::ReadFailure { failure };
                            tried.insert(peer.node_id);
                            if tried.len() == current.replicas.len() {
                                route = None;
                                tried.clear();
                            }
                        } else {
                            // UNKNOWN_WRITE_TERMINAL: never cross a retry edge.
                            report.outcome = RoutedOutcome::UnknownWrite { failure };
                            report.stop = Stop::Terminal;
                            break;
                        }
                    }
                }
            } else {
                let peer = {
                    let candidates = self.0.metadata.lock().expect("metadata routes poisoned");
                    if candidates.iter().all(|p| lookup_tried.contains(&p.node_id)) {
                        lookup_tried.clear();
                    }
                    let peer = candidates
                        .iter()
                        .cycle()
                        .skip(lookup_cursor % candidates.len())
                        .take(candidates.len())
                        .find(|p| !lookup_tried.contains(&p.node_id))
                        .expect("untried metadata candidate")
                        .clone();
                    lookup_tried.insert(peer.node_id);
                    peer
                };
                let probe_deadline = deadline
                    .min(Instant::now() + Duration::from_millis(self.0.config.probe_timeout_ms));
                let result = self.lookup_once(&peer, key, probe_deadline).await;
                report.attempts.push(RoutedAttempt {
                    ordinal,
                    phase: Phase::Lookup,
                    route: None,
                    node_id: peer.node_id,
                    elapsed_ns: nanos(attempt_start.elapsed()),
                    failure: result.as_ref().err().cloned(),
                });
                match result {
                    Ok((metadata, leader, found)) => {
                        self.learn_metadata(metadata);
                        let candidates = self.0.metadata.lock().expect("metadata routes poisoned");
                        lookup_cursor = leader
                            .and_then(|id| {
                                candidates
                                    .iter()
                                    .position(|p| p.node_id == id && !lookup_tried.contains(&id))
                            })
                            .unwrap_or((lookup_cursor + 1) % candidates.len());
                        if let Some(found) = found {
                            self.0.metadata_preferred.store(
                                candidates
                                    .iter()
                                    .position(|p| p.node_id == peer.node_id)
                                    .unwrap_or(0),
                                Ordering::Relaxed,
                            );
                            self.cache(found.clone());
                            route = Some(found);
                        }
                        report.outcome = RoutedOutcome::LookupFailure {
                            failure: rpc(Reason::NotLeader { leader }),
                        };
                    }
                    Err(failure) => {
                        lookup_cursor += 1;
                        report.outcome = RoutedOutcome::LookupFailure { failure };
                    }
                }
            }
            if ordinal < self.0.config.max_attempts {
                sleep_until(
                    deadline.min(
                        Instant::now() + Duration::from_millis(self.0.config.retry_backoff_ms),
                    ),
                )
                .await;
            }
        }
        finish_routed(report, start)
    }
}

fn keys(operation: &RawOperation) -> Vec<&[u8]> {
    match operation {
        RawOperation::Get { key }
        | RawOperation::Put { key, .. }
        | RawOperation::Delete { key } => vec![key],
        RawOperation::BatchGet { keys } => keys.iter().map(Vec::as_slice).collect(),
        RawOperation::BatchPut { pairs } => pairs.iter().map(|(key, _)| key.as_slice()).collect(),
    }
}
fn peers(values: Vec<proto::NodeEndpoint>, data: bool) -> Result<Vec<Peer>, Failure> {
    if values.is_empty() || values.len() > MAX_PEERS || (data && ![3, 5, 7].contains(&values.len()))
    {
        return Err(Failure::InvalidRoute);
    }
    let mut ids = BTreeSet::new();
    let mut addresses = BTreeSet::new();
    values
        .into_iter()
        .map(|value| {
            let endpoint =
                crate::endpoints::decode_endpoint(value).map_err(|_| Failure::InvalidRoute)?;
            if !endpoint.active || !ids.insert(endpoint.node) || !addresses.insert(endpoint.address)
            {
                return Err(Failure::InvalidRoute);
            }
            Ok(Peer {
                node_id: endpoint.node.0,
                address: endpoint.address,
            })
        })
        .collect()
}
fn overlaps(a: &DataRange, b: &DataRange) -> bool {
    (a.end.is_empty() || b.start < a.end) && (b.end.is_empty() || a.start < b.end)
}
fn rpc(reason: Reason) -> Failure {
    Failure::Rpc { reason }
}
pub(crate) fn classify(status: &Status, read: bool) -> Failure {
    if status.code() == Code::FailedPrecondition && exclusive(status.metadata(), &[ROUTE_REFUSED]) {
        match single(status.metadata(), ROUTE_REFUSED) {
            Some("scope") => return Failure::ScopeRefused,
            Some("cross_range") => return Failure::CrossRange,
            _ => {}
        }
    }
    rpc(classify_status(status, read))
}
pub(crate) fn valid_reply(
    response: &tonic::Response<proto::RoutedRawResponse>,
    count: usize,
) -> bool {
    if has_control(response.metadata()) || response.get_ref().binding_digest.len() != 32 {
        return false;
    }
    match &response.get_ref().result {
        Some(proto::routed_raw_response::Result::Values(values)) => {
            count > 0 && values.values.len() == count && values.values.iter().all(valid_optional)
        }
        Some(proto::routed_raw_response::Result::Applied(at)) => {
            count == 0 && at.applied_term > 0 && at.applied_index > 0
        }
        None => false,
    }
}
fn finish_routed(mut report: RoutedReport, start: Instant) -> RoutedReport {
    report.elapsed_ns = nanos(start.elapsed());
    report
}
