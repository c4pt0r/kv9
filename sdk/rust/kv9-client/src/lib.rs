//! Client SDK for the kv9 distributed raw key-value store.
//!
//! ```no_run
//! use kv9_client::{Client, Mutation};
//!
//! # async fn demo() -> Result<(), kv9_client::Error> {
//! let client = Client::new("127.0.0.1:20161", "my-client-token")?
//!     .with_peers([(1, "127.0.0.1:20161"), (2, "127.0.0.1:20162"), (3, "127.0.0.1:20163")]);
//! let ks = client.keyspace(client.create_keyspace("app", 0).await?);
//! let applied = ks.put(b"k".to_vec(), b"v".to_vec()).await?;
//! assert!(applied.term > 0 && applied.index > 0);
//! ks.batch(vec![
//!     Mutation::Put { key: b"a".to_vec(), value: b"1".to_vec() },
//!     Mutation::Delete { key: b"k".to_vec() },
//! ])
//! .await?;
//! assert_eq!(ks.get(b"k".to_vec()).await?, None);
//! # Ok(()) }
//! ```
//!
//! The contract shared by every kv9 SDK (auth header, request context and epoch
//! refresh, typed refusal markers, bounds, redirect policy) is documented in
//! `sdk/README.md`. This crate speaks exactly the proto the server in this
//! repository serves; it is generated at build time from `crates/server/proto`.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tonic::metadata::{Ascii, KeyRef, MetadataMap, MetadataValue};
use tonic::transport::{Channel, Endpoint};
use tonic::{Code, Request, Response, Status};

/// Generated protocol types (`kv9.v1`).
pub mod proto {
    tonic::include_proto!("kv9.v1");
}

use proto::kv9_client::Kv9Client;

pub const MAX_KEY_BYTES: usize = 4096;
pub const MAX_VALUE_BYTES: usize = 65_536;
pub const MAX_BATCH_ITEMS: usize = 256;
pub const MAX_MESSAGE_BYTES: usize = 1_048_576;
pub const MAX_DEADLINE: Duration = Duration::from_secs(30);

const NOT_LEADER_KEY: &str = "kv9-not-leader";
const LEADER_HINT_KEY: &str = "kv9-leader-node-id";
const READ_UNCONFIRMED_KEY: &str = "kv9-read-unconfirmed";
const ADMISSION_REFUSED_KEY: &str = "kv9-admission-refused";
const PARTIAL_WRITE_KEY: &str = "kv9-partial-write";
const COMMITTED_CHUNKS_KEY: &str = "kv9-committed-chunks";
const LAST_APPLIED_TERM_KEY: &str = "kv9-last-applied-term";
const LAST_APPLIED_INDEX_KEY: &str = "kv9-last-applied-index";

/// The region epoch a request believes in. Keyspaces served by the metadata group
/// live at `(1, 1)`, which is also the default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Epoch {
    pub conf_ver: u64,
    pub version: u64,
}

impl Default for Epoch {
    fn default() -> Self {
        Self {
            conf_ver: 1,
            version: 1,
        }
    }
}

/// The exact applied position of a write. Compare both fields, never the index alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Applied {
    pub term: u64,
    pub index: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionLocation {
    pub region_id: u64,
    pub epoch: Epoch,
    pub leader_node_id: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyspaceInfo {
    pub id: u32,
    pub name: String,
    pub tenant_id: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClusterInfo {
    pub node_count: u64,
    pub keyspace_count: u64,
    pub region_count: u64,
}

/// One page of a scan. A nonempty `resume_from` means "continue exactly here", even
/// when `pairs` is empty.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ScanPage {
    pub pairs: Vec<(Vec<u8>, Vec<u8>)>,
    pub resume_from: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteRangeResult {
    pub committed_chunks: u64,
    pub last_applied: Applied,
    pub resume_from: Vec<u8>,
}

/// One entry of an atomic range batch, in application order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mutation {
    Put { key: Vec<u8>, value: Vec<u8> },
    Delete { key: Vec<u8> },
}

impl Mutation {
    pub fn key(&self) -> &[u8] {
        match self {
            Self::Put { key, .. } | Self::Delete { key } => key,
        }
    }
}

/// Which stage of the read barrier did not establish.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadPhase {
    Quorum,
    Apply,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionReason {
    RequestCount,
    EncodedBytes,
    RequestTooLarge,
}

/// Every error this SDK raises. Typed refusals come from the server's trailing
/// metadata markers, never from message text; anything unmarked is [`Error::Rpc`],
/// and for a write that means the outcome is unknown.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// A local bound was violated; nothing was sent.
    #[error("request violates a bound: {0}")]
    Bounds(&'static str),
    /// The node is not the leader; nothing executed. Retry at the hinted node.
    #[error("not leader (hint: {leader:?})")]
    NotLeader { leader: Option<u64> },
    /// The read barrier did not establish; nothing was answered.
    #[error("read unconfirmed ({phase:?})")]
    ReadUnconfirmed { phase: ReadPhase },
    /// Refused before execution; back off and retry.
    #[error("admission refused ({reason:?})")]
    AdmissionRefused { reason: AdmissionReason },
    /// A `delete_range` committed some chunks, then failed. They stay committed.
    #[error(
        "partial write: {committed_chunks} chunk(s) committed through {last_applied:?}: {cause}"
    )]
    PartialWrite {
        committed_chunks: u64,
        last_applied: Applied,
        cause: String,
    },
    /// An unmarked RPC status. `FAILED_PRECONDITION` covers a stale epoch, a batch
    /// crossing a region and an API-type mismatch.
    #[error("rpc failed: {code:?}: {message}")]
    Rpc { code: Code, message: String },
    /// The server sent a marker the SDK cannot interpret.
    #[error("protocol error: {0}")]
    Protocol(String),
    /// The endpoint could not be dialled or the token is not valid metadata.
    #[error("transport: {0}")]
    Transport(String),
}

impl Error {
    /// `FAILED_PRECONDITION` without a marker: refresh the epoch before retrying.
    pub fn is_precondition(&self) -> bool {
        matches!(
            self,
            Self::Rpc {
                code: Code::FailedPrecondition,
                ..
            }
        )
    }
}

fn single<'a>(metadata: &'a MetadataMap, key: &str) -> Result<Option<&'a str>, Error> {
    let mut values = metadata.get_all(key).iter();
    let Some(first) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(Error::Protocol(format!("marker {key} repeated")));
    }
    first
        .to_str()
        .map(Some)
        .map_err(|_| Error::Protocol(format!("marker {key} is not ascii")))
}

fn integer(metadata: &MetadataMap, key: &str) -> Result<u64, Error> {
    let text =
        single(metadata, key)?.ok_or_else(|| Error::Protocol(format!("marker {key} missing")))?;
    let value: u64 = text
        .parse()
        .map_err(|_| Error::Protocol(format!("marker {key} is not an integer")))?;
    if value.to_string() != text {
        return Err(Error::Protocol(format!("marker {key} is not canonical")));
    }
    Ok(value)
}

/// Decode a failed status into the typed error its markers describe.
pub fn classify(status: &Status, read: bool) -> Error {
    match classify_inner(status, read) {
        Ok(error) | Err(error) => error,
    }
}

fn classify_inner(status: &Status, read: bool) -> Result<Error, Error> {
    let metadata = status.metadata();
    if metadata.contains_key(NOT_LEADER_KEY) {
        if single(metadata, NOT_LEADER_KEY)? != Some("true")
            || status.code() != Code::FailedPrecondition
        {
            return Err(Error::Protocol("malformed not-leader marker".into()));
        }
        let leader = if metadata.contains_key(LEADER_HINT_KEY) {
            let id = integer(metadata, LEADER_HINT_KEY)?;
            if id == 0 {
                return Err(Error::Protocol("leader hint names node 0".into()));
            }
            Some(id)
        } else {
            None
        };
        return Ok(Error::NotLeader { leader });
    }
    if metadata.contains_key(ADMISSION_REFUSED_KEY) {
        let reason = match single(metadata, ADMISSION_REFUSED_KEY)? {
            Some("request_count") => AdmissionReason::RequestCount,
            Some("encoded_bytes") => AdmissionReason::EncodedBytes,
            Some("request_too_large") => AdmissionReason::RequestTooLarge,
            other => {
                return Err(Error::Protocol(format!(
                    "unknown admission refusal {other:?}"
                )))
            }
        };
        return Ok(Error::AdmissionRefused { reason });
    }
    if metadata.contains_key(PARTIAL_WRITE_KEY) {
        if single(metadata, PARTIAL_WRITE_KEY)? != Some("true") {
            return Err(Error::Protocol("malformed partial-write marker".into()));
        }
        return Ok(Error::PartialWrite {
            committed_chunks: integer(metadata, COMMITTED_CHUNKS_KEY)?,
            last_applied: Applied {
                term: integer(metadata, LAST_APPLIED_TERM_KEY)?,
                index: integer(metadata, LAST_APPLIED_INDEX_KEY)?,
            },
            cause: status.message().to_owned(),
        });
    }
    if metadata.contains_key(READ_UNCONFIRMED_KEY) {
        let phase = match single(metadata, READ_UNCONFIRMED_KEY)? {
            Some("quorum") if read => ReadPhase::Quorum,
            Some("apply") if read => ReadPhase::Apply,
            _ => return Err(Error::Protocol("malformed read-unconfirmed marker".into())),
        };
        if status.code() != Code::Unavailable {
            return Err(Error::Protocol(
                "read-unconfirmed with a foreign code".into(),
            ));
        }
        return Ok(Error::ReadUnconfirmed { phase });
    }
    if metadata.keys().any(|key| match key {
        KeyRef::Ascii(key) => key.as_str().starts_with("kv9-"),
        KeyRef::Binary(key) => key.as_str().starts_with("kv9-"),
    }) {
        return Err(Error::Protocol("unexpected control marker".into()));
    }
    Ok(Error::Rpc {
        code: status.code(),
        message: status.message().to_owned(),
    })
}

fn check_key(key: &[u8]) -> Result<(), Error> {
    if key.len() > MAX_KEY_BYTES {
        return Err(Error::Bounds("key exceeds 4096 bytes"));
    }
    Ok(())
}

fn check_value(value: &[u8]) -> Result<(), Error> {
    if value.len() > MAX_VALUE_BYTES {
        return Err(Error::Bounds("value exceeds 65536 bytes"));
    }
    Ok(())
}

fn check_count(count: usize) -> Result<(), Error> {
    if !(1..=MAX_BATCH_ITEMS).contains(&count) {
        return Err(Error::Bounds("a batch carries 1..=256 items"));
    }
    Ok(())
}

fn check_message<M: prost::Message>(message: &M) -> Result<(), Error> {
    if message.encoded_len() > MAX_MESSAGE_BYTES {
        return Err(Error::Bounds("encoded request exceeds 1 MiB"));
    }
    Ok(())
}

struct Inner {
    address: Mutex<String>,
    token: MetadataValue<Ascii>,
    timeout: Duration,
    peers: HashMap<u64, String>,
    max_redirects: usize,
    channels: Mutex<HashMap<String, Channel>>,
}

/// An authenticated client for one kv9 cluster. Cheap to clone; one lazily dialled
/// channel per node address it has spoken to.
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

impl Client {
    /// `address` is `host:port` of any node's public endpoint; `token` one of the
    /// server's `KV9_CLIENT_TOKENS`. Nothing is dialled until the first call.
    pub fn new(address: impl Into<String>, token: &str) -> Result<Self, Error> {
        if token.is_empty() {
            return Err(Error::Bounds("client token must be non-empty"));
        }
        let token = format!("Bearer {token}")
            .parse()
            .map_err(|_| Error::Transport("client token is not valid metadata".into()))?;
        Ok(Self {
            inner: Arc::new(Inner {
                address: Mutex::new(address.into()),
                token,
                timeout: Duration::from_secs(5),
                peers: HashMap::new(),
                max_redirects: 3,
                channels: Mutex::new(HashMap::new()),
            }),
        })
    }

    fn rebuild(self, f: impl FnOnce(&mut Inner)) -> Self {
        let mut inner = Inner {
            address: Mutex::new(self.address()),
            token: self.inner.token.clone(),
            timeout: self.inner.timeout,
            peers: self.inner.peers.clone(),
            max_redirects: self.inner.max_redirects,
            channels: Mutex::new(HashMap::new()),
        };
        f(&mut inner);
        Self {
            inner: Arc::new(inner),
        }
    }

    /// Per-call deadline (clamped to 30 s, the server's maximum).
    pub fn with_timeout(self, timeout: Duration) -> Self {
        self.rebuild(|inner| inner.timeout = timeout.min(MAX_DEADLINE))
    }

    /// `node_id -> address` used to follow `NotLeader` hints.
    pub fn with_peers<A: Into<String>>(self, peers: impl IntoIterator<Item = (u64, A)>) -> Self {
        let peers = peers.into_iter().map(|(id, a)| (id, a.into())).collect();
        self.rebuild(|inner| inner.peers = peers)
    }

    pub fn with_max_redirects(self, hops: usize) -> Self {
        self.rebuild(|inner| inner.max_redirects = hops)
    }

    /// The node currently addressed (moves with followed hints).
    pub fn address(&self) -> String {
        self.inner.address.lock().expect("address").clone()
    }

    fn channel(&self, address: &str) -> Result<Channel, Error> {
        let mut channels = self.inner.channels.lock().expect("channels");
        if let Some(channel) = channels.get(address) {
            return Ok(channel.clone());
        }
        let channel = Endpoint::from_shared(format!("http://{address}"))
            .map_err(|e| Error::Transport(e.to_string()))?
            .connect_timeout(self.inner.timeout)
            .connect_lazy();
        channels.insert(address.to_owned(), channel.clone());
        Ok(channel)
    }

    async fn invoke<Req, Resp, F, Fut>(
        &self,
        read: bool,
        request: Req,
        call: F,
    ) -> Result<Resp, Error>
    where
        Req: Clone,
        F: Fn(Kv9Client<Channel>, Request<Req>) -> Fut,
        Fut: Future<Output = Result<Response<Resp>, Status>>,
    {
        let mut hops = 0;
        loop {
            let address = self.address();
            let client = Kv9Client::new(self.channel(&address)?)
                .max_decoding_message_size(MAX_MESSAGE_BYTES)
                .max_encoding_message_size(MAX_MESSAGE_BYTES);
            let mut wire = Request::new(request.clone());
            wire.metadata_mut()
                .insert("authorization", self.inner.token.clone());
            wire.set_timeout(self.inner.timeout);
            match call(client, wire).await {
                Ok(response) => return Ok(response.into_inner()),
                Err(status) => {
                    let error = classify(&status, read);
                    if let Error::NotLeader {
                        leader: Some(leader),
                    } = &error
                    {
                        if hops < self.inner.max_redirects {
                            if let Some(target) = self.inner.peers.get(leader) {
                                if *target != address {
                                    *self.inner.address.lock().expect("address") = target.clone();
                                    hops += 1;
                                    continue;
                                }
                            }
                        }
                    }
                    return Err(error);
                }
            }
        }
    }

    // ------------------------------------------------------------------ admin

    /// Create a raw keyspace served by the metadata group; returns its id.
    pub async fn create_keyspace(&self, name: &str, tenant_id: u64) -> Result<u32, Error> {
        let response = self
            .invoke(
                false,
                proto::CreateKeyspaceRequest {
                    name: name.to_owned(),
                    tenant_id,
                    api_type: proto::ApiType::Raw as i32,
                },
                |mut c, r| async move { c.create_keyspace(r).await },
            )
            .await?;
        Ok(response.keyspace_id)
    }

    pub async fn list_keyspaces(&self) -> Result<Vec<KeyspaceInfo>, Error> {
        let response = self
            .invoke(
                true,
                proto::ListKeyspacesRequest {},
                |mut c, r| async move { c.list_keyspaces(r).await },
            )
            .await?;
        Ok(response
            .keyspaces
            .into_iter()
            .map(|k| KeyspaceInfo {
                id: k.id,
                name: k.name,
                tenant_id: k.tenant_id,
            })
            .collect())
    }

    pub async fn cluster_info(&self) -> Result<ClusterInfo, Error> {
        let r = self
            .invoke(true, proto::ClusterInfoRequest {}, |mut c, r| async move {
                c.cluster_info(r).await
            })
            .await?;
        Ok(ClusterInfo {
            node_count: r.node_count,
            keyspace_count: r.keyspace_count,
            region_count: r.region_count,
        })
    }

    pub async fn get_region(&self, keyspace_id: u32, key: &[u8]) -> Result<RegionLocation, Error> {
        check_key(key)?;
        let r = self
            .invoke(
                true,
                proto::GetRegionRequest {
                    keyspace_id,
                    key: key.to_vec(),
                },
                |mut c, r| async move { c.get_region(r).await },
            )
            .await?;
        let region = r
            .region
            .ok_or_else(|| Error::Protocol("GetRegion answered without a region".into()))?;
        let epoch = region
            .epoch
            .ok_or_else(|| Error::Protocol("GetRegion answered without an epoch".into()))?;
        Ok(RegionLocation {
            region_id: region.region_id,
            epoch: Epoch {
                conf_ver: epoch.conf_ver,
                version: epoch.version,
            },
            leader_node_id: region.leader_node_id,
        })
    }

    /// A handle for raw operations on `keyspace_id`, starting at epoch `(1, 1)` with
    /// automatic one-shot epoch refresh.
    pub fn keyspace(&self, keyspace_id: u32) -> Keyspace {
        Keyspace {
            client: self.clone(),
            id: keyspace_id,
            epoch: Arc::new(Mutex::new(Epoch::default())),
            auto_refresh: true,
        }
    }
}

/// Raw operations on one keyspace under one region epoch.
#[derive(Clone)]
pub struct Keyspace {
    client: Client,
    id: u32,
    epoch: Arc<Mutex<Epoch>>,
    auto_refresh: bool,
}

impl Keyspace {
    pub fn with_epoch(self, epoch: Epoch) -> Self {
        *self.epoch.lock().expect("epoch") = epoch;
        self
    }

    pub fn with_auto_refresh(mut self, enabled: bool) -> Self {
        self.auto_refresh = enabled;
        self
    }

    pub fn id(&self) -> u32 {
        self.id
    }

    pub fn epoch(&self) -> Epoch {
        *self.epoch.lock().expect("epoch")
    }

    fn context(&self) -> Option<proto::RequestContext> {
        let epoch = self.epoch();
        Some(proto::RequestContext {
            keyspace_id: self.id,
            region_epoch: Some(proto::RegionEpoch {
                conf_ver: epoch.conf_ver,
                version: epoch.version,
            }),
        })
    }

    /// Re-read the region epoch from the cluster and adopt it.
    pub async fn refresh_epoch(&self, key: &[u8]) -> Result<Epoch, Error> {
        let epoch = self.client.get_region(self.id, key).await?.epoch;
        *self.epoch.lock().expect("epoch") = epoch;
        Ok(epoch)
    }

    async fn call<Req, Resp, F, Fut>(
        &self,
        read: bool,
        anchor: &[u8],
        build: impl Fn(Option<proto::RequestContext>) -> Req,
        call: F,
    ) -> Result<Resp, Error>
    where
        Req: Clone + prost::Message,
        F: Fn(Kv9Client<Channel>, Request<Req>) -> Fut + Clone,
        Fut: Future<Output = Result<Response<Resp>, Status>>,
    {
        let request = build(self.context());
        check_message(&request)?;
        match self.client.invoke(read, request, call.clone()).await {
            Err(error) if error.is_precondition() && self.auto_refresh => {
                let before = self.epoch();
                if self.refresh_epoch(anchor).await? == before {
                    return Err(error);
                }
                self.client.invoke(read, build(self.context()), call).await
            }
            other => other,
        }
    }

    fn applied(r: proto::RawWriteResponse) -> Result<Applied, Error> {
        if r.applied_term == 0 || r.applied_index == 0 {
            return Err(Error::Protocol(
                "write acknowledged without an applied position".into(),
            ));
        }
        Ok(Applied {
            term: r.applied_term,
            index: r.applied_index,
        })
    }

    // ------------------------------------------------------------------ reads

    pub async fn get(&self, key: Vec<u8>) -> Result<Option<Vec<u8>>, Error> {
        check_key(&key)?;
        let r = self
            .call(
                true,
                &key,
                |context| proto::RawGetRequest {
                    context,
                    key: key.clone(),
                },
                |mut c, r| async move { c.raw_get(r).await },
            )
            .await?;
        Ok(r.value.and_then(|v| v.found.then_some(v.value)))
    }

    /// One quorum-confirmed view; results align with `keys`, duplicates included.
    pub async fn batch_get(&self, keys: Vec<Vec<u8>>) -> Result<Vec<Option<Vec<u8>>>, Error> {
        check_count(keys.len())?;
        for key in &keys {
            check_key(key)?;
        }
        let r = self
            .call(
                true,
                &keys[0],
                |context| proto::RawBatchGetRequest {
                    context,
                    keys: keys.clone(),
                },
                |mut c, r| async move { c.raw_batch_get(r).await },
            )
            .await?;
        if r.values.len() != keys.len() {
            return Err(Error::Protocol(
                "batch_get returned a different cardinality".into(),
            ));
        }
        Ok(r.values
            .into_iter()
            .map(|v| v.found.then_some(v.value))
            .collect())
    }

    /// One page of `[start, end)`; an empty `end` means "to the keyspace end".
    pub async fn scan(&self, start: Vec<u8>, end: Vec<u8>, limit: u32) -> Result<ScanPage, Error> {
        check_key(&start)?;
        check_key(&end)?;
        if limit == 0 {
            return Err(Error::Bounds("scan limit must be positive"));
        }
        let r = self
            .call(
                true,
                &start,
                |context| proto::RawScanRequest {
                    context,
                    start: start.clone(),
                    end: end.clone(),
                    limit,
                },
                |mut c, r| async move { c.raw_scan(r).await },
            )
            .await?;
        Ok(ScanPage {
            pairs: r.pairs.into_iter().map(|p| (p.key, p.value)).collect(),
            resume_from: r.resume_from,
        })
    }

    /// Walk `[start, end)` page by page, following `resume_from`.
    pub async fn scan_all(
        &self,
        start: Vec<u8>,
        end: Vec<u8>,
        page: u32,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, Error> {
        let mut out = Vec::new();
        let mut cursor = start;
        loop {
            let result = self.scan(cursor, end.clone(), page).await?;
            let count = result.pairs.len();
            out.extend(result.pairs);
            if !result.resume_from.is_empty() {
                cursor = result.resume_from;
            } else if count < page as usize {
                return Ok(out);
            } else {
                let mut next = out.last().expect("nonempty page").0.clone();
                next.push(0);
                cursor = next;
            }
        }
    }

    // ----------------------------------------------------------------- writes

    pub async fn put(&self, key: Vec<u8>, value: Vec<u8>) -> Result<Applied, Error> {
        check_key(&key)?;
        check_value(&value)?;
        let r = self
            .call(
                false,
                &key,
                |context| proto::RawPutRequest {
                    context,
                    key: key.clone(),
                    value: value.clone(),
                },
                |mut c, r| async move { c.raw_put(r).await },
            )
            .await?;
        Self::applied(r)
    }

    pub async fn delete(&self, key: Vec<u8>) -> Result<Applied, Error> {
        check_key(&key)?;
        let r = self
            .call(
                false,
                &key,
                |context| proto::RawDeleteRequest {
                    context,
                    key: key.clone(),
                },
                |mut c, r| async move { c.raw_delete(r).await },
            )
            .await?;
        Self::applied(r)
    }

    /// Put-only atomic batch: every pair lands at one applied position.
    pub async fn batch_put(&self, pairs: Vec<(Vec<u8>, Vec<u8>)>) -> Result<Applied, Error> {
        check_count(pairs.len())?;
        for (key, value) in &pairs {
            check_key(key)?;
            check_value(value)?;
        }
        let wire: Vec<_> = pairs
            .iter()
            .map(|(key, value)| proto::KeyValue {
                key: key.clone(),
                value: value.clone(),
            })
            .collect();
        let r = self
            .call(
                false,
                &pairs[0].0,
                |context| proto::RawBatchPutRequest {
                    context,
                    pairs: wire.clone(),
                },
                |mut c, r| async move { c.raw_batch_put(r).await },
            )
            .await?;
        Self::applied(r)
    }

    /// The atomic range batch: ordered puts and deletes applied as ONE Raft entry.
    ///
    /// Every key must lie in the same region; a batch crossing a region is refused
    /// whole (`FAILED_PRECONDITION`), nothing is proposed, and the SDK never splits it.
    /// A later mutation for the same key wins; deleting an absent key is a no-op.
    pub async fn batch(&self, mutations: Vec<Mutation>) -> Result<Applied, Error> {
        check_count(mutations.len())?;
        let mut wire = Vec::with_capacity(mutations.len());
        for mutation in &mutations {
            check_key(mutation.key())?;
            wire.push(proto::RawMutation {
                op: Some(match mutation {
                    Mutation::Put { key, value } => {
                        check_value(value)?;
                        proto::raw_mutation::Op::Put(proto::KeyValue {
                            key: key.clone(),
                            value: value.clone(),
                        })
                    }
                    Mutation::Delete { key } => proto::raw_mutation::Op::Delete(key.clone()),
                }),
            });
        }
        let r = self
            .call(
                false,
                mutations[0].key(),
                |context| proto::RawBatchRequest {
                    context,
                    mutations: wire.clone(),
                },
                |mut c, r| async move { c.raw_batch(r).await },
            )
            .await?;
        Self::applied(r)
    }

    /// Delete `[start, end)` in bounded chunks. Not atomic as a whole: see
    /// [`Error::PartialWrite`] and [`DeleteRangeResult::resume_from`].
    pub async fn delete_range(
        &self,
        start: Vec<u8>,
        end: Vec<u8>,
    ) -> Result<DeleteRangeResult, Error> {
        check_key(&start)?;
        check_key(&end)?;
        let r = self
            .call(
                false,
                &start,
                |context| proto::RawDeleteRangeRequest {
                    context,
                    start: start.clone(),
                    end: end.clone(),
                },
                |mut c, r| async move { c.raw_delete_range(r).await },
            )
            .await?;
        Ok(DeleteRangeResult {
            committed_chunks: r.committed_chunks,
            last_applied: Applied {
                term: r.last_applied_term,
                index: r.last_applied_index,
            },
            resume_from: r.resume_from,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(code: Code, markers: &[(&str, &str)]) -> Status {
        let mut metadata = MetadataMap::new();
        for (key, value) in markers {
            let key = tonic::metadata::MetadataKey::<Ascii>::from_bytes(key.as_bytes()).unwrap();
            metadata.append(key, value.parse().unwrap());
        }
        Status::with_metadata(code, "message", metadata)
    }

    #[test]
    fn markers_decode_to_typed_errors() {
        assert_eq!(
            classify(
                &status(
                    Code::FailedPrecondition,
                    &[("kv9-not-leader", "true"), ("kv9-leader-node-id", "3")]
                ),
                false
            ),
            Error::NotLeader { leader: Some(3) }
        );
        assert_eq!(
            classify(
                &status(Code::FailedPrecondition, &[("kv9-not-leader", "true")]),
                true
            ),
            Error::NotLeader { leader: None }
        );
        assert_eq!(
            classify(
                &status(Code::Unavailable, &[("kv9-read-unconfirmed", "apply")]),
                true
            ),
            Error::ReadUnconfirmed {
                phase: ReadPhase::Apply
            }
        );
        assert_eq!(
            classify(
                &status(
                    Code::ResourceExhausted,
                    &[("kv9-admission-refused", "request_too_large")]
                ),
                false
            ),
            Error::AdmissionRefused {
                reason: AdmissionReason::RequestTooLarge
            }
        );
        assert_eq!(
            classify(
                &status(
                    Code::Unavailable,
                    &[
                        ("kv9-partial-write", "true"),
                        ("kv9-committed-chunks", "2"),
                        ("kv9-last-applied-term", "4"),
                        ("kv9-last-applied-index", "77"),
                    ]
                ),
                false
            ),
            Error::PartialWrite {
                committed_chunks: 2,
                last_applied: Applied { term: 4, index: 77 },
                cause: "message".into(),
            }
        );
    }

    #[test]
    fn malformed_or_foreign_markers_are_protocol_errors() {
        for (code, markers, read) in [
            (
                Code::FailedPrecondition,
                vec![("kv9-not-leader", "yes")],
                false,
            ),
            (Code::Unavailable, vec![("kv9-not-leader", "true")], false),
            (
                Code::FailedPrecondition,
                vec![("kv9-not-leader", "true"), ("kv9-leader-node-id", "0")],
                false,
            ),
            (
                Code::FailedPrecondition,
                vec![("kv9-not-leader", "true"), ("kv9-leader-node-id", "03")],
                false,
            ),
            (
                Code::Unavailable,
                vec![("kv9-read-unconfirmed", "quorum")],
                false,
            ),
            (
                Code::Unavailable,
                vec![("kv9-read-unconfirmed", "later")],
                true,
            ),
            (
                Code::ResourceExhausted,
                vec![("kv9-admission-refused", "mood")],
                false,
            ),
            (Code::Internal, vec![("kv9-something-new", "1")], false),
        ] {
            assert!(
                matches!(classify(&status(code, &markers), read), Error::Protocol(_)),
                "{markers:?}"
            );
        }
    }

    #[test]
    fn unmarked_statuses_stay_plain_and_only_precondition_refreshes() {
        let stale = classify(&status(Code::FailedPrecondition, &[]), false);
        assert!(stale.is_precondition());
        let deadline = classify(&status(Code::DeadlineExceeded, &[]), false);
        assert!(!deadline.is_precondition());
        assert_eq!(
            deadline,
            Error::Rpc {
                code: Code::DeadlineExceeded,
                message: "message".into()
            }
        );
    }

    #[tokio::test]
    async fn local_bounds_refuse_before_any_dial() {
        let client = Client::new("127.0.0.1:1", "t").unwrap();
        let ks = client.keyspace(7);
        assert_eq!(
            ks.put(vec![0; MAX_KEY_BYTES + 1], vec![]).await,
            Err(Error::Bounds("key exceeds 4096 bytes"))
        );
        assert_eq!(
            ks.batch(vec![]).await,
            Err(Error::Bounds("a batch carries 1..=256 items"))
        );
        assert_eq!(
            ks.scan(vec![], vec![], 0).await,
            Err(Error::Bounds("scan limit must be positive"))
        );
        assert!(Client::new("127.0.0.1:1", "").is_err());
    }
}
