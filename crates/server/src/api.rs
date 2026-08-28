//! The v0 API surface as Rust traits (DESIGN §11).
//!
//! Transport is gRPC; these traits are the synchronous core contract behind tonic's
//! blocking boundary. Every data request
//! carries `(keyspace_id, region_epoch)` so the router can resolve keyspace→region,
//! epoch-check, and validate the API type against the keyspace declaration.

use kv9_common::{KeyspaceId, Result, TimeStamp, UserKey, Value};
use kv9_region::RegionEpoch;

/// Context threaded on every data request (DESIGN §11).
#[derive(Debug, Clone)]
pub struct RequestContext {
    pub keyspace: KeyspaceId,
    pub region_epoch: RegionEpoch,
    /// Authenticated caller identity (auth is in scope from day one — DESIGN §11,
    /// §13 principle 9).
    pub caller: Option<String>,
}

/// The transactional API for `txn` keyspaces (DESIGN §11 Txn surface).
pub trait TxnApi {
    fn kv_get(
        &self,
        ctx: &RequestContext,
        key: &[u8],
        start_ts: TimeStamp,
    ) -> Result<Option<Value>>;
    fn kv_batch_get(
        &self,
        ctx: &RequestContext,
        keys: &[UserKey],
        start_ts: TimeStamp,
    ) -> Result<Vec<Option<Value>>>;
    fn kv_scan(
        &self,
        ctx: &RequestContext,
        start: &[u8],
        end: &[u8],
        limit: usize,
        start_ts: TimeStamp,
    ) -> Result<Vec<(UserKey, Value)>>;
    fn kv_prewrite(
        &self,
        ctx: &RequestContext,
        mutations: &[(UserKey, Option<Value>)],
        primary: &[u8],
        start_ts: TimeStamp,
    ) -> Result<()>;
    fn kv_commit(
        &self,
        ctx: &RequestContext,
        keys: &[UserKey],
        start_ts: TimeStamp,
        commit_ts: TimeStamp,
    ) -> Result<()>;
    fn kv_pessimistic_lock(
        &self,
        ctx: &RequestContext,
        keys: &[UserKey],
        start_ts: TimeStamp,
    ) -> Result<()>;
    fn kv_pessimistic_rollback(
        &self,
        ctx: &RequestContext,
        keys: &[UserKey],
        start_ts: TimeStamp,
    ) -> Result<()>;
    fn kv_resolve_lock(
        &self,
        ctx: &RequestContext,
        start_ts: TimeStamp,
        commit_ts: Option<TimeStamp>,
    ) -> Result<()>;
    fn kv_cleanup(&self, ctx: &RequestContext, key: &[u8], start_ts: TimeStamp) -> Result<()>;
    fn kv_check_txn_status(
        &self,
        ctx: &RequestContext,
        primary: &[u8],
        lock_ts: TimeStamp,
    ) -> Result<()>;
}

/// The raw API for `raw` keyspaces (DESIGN §11 Raw surface).
pub trait RawApi {
    fn raw_get(&self, ctx: &RequestContext, key: &[u8]) -> Result<Option<Value>>;
    fn raw_batch_get(&self, ctx: &RequestContext, keys: &[UserKey]) -> Result<Vec<Option<Value>>>;
    fn raw_put(&self, ctx: &RequestContext, key: UserKey, value: Value)
        -> Result<AppliedPosition>;
    fn raw_batch_put(
        &self,
        ctx: &RequestContext,
        kvs: &[(UserKey, Value)],
    ) -> Result<AppliedPosition>;
    fn raw_delete(&self, ctx: &RequestContext, key: &[u8]) -> Result<AppliedPosition>;
    fn raw_scan(
        &self,
        ctx: &RequestContext,
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<(UserKey, Value)>>;
    fn raw_delete_range(
        &self,
        ctx: &RequestContext,
        start: &[u8],
        end: &[u8],
    ) -> Result<DeleteRangeReceipt>;
}

/// How far a chunked range delete got.
///
/// Returned on success; on partial failure the same numbers travel in
/// [`Error::PartialDeleteRange`](kv9_common::Error::PartialDeleteRange). Either way the
/// caller can tell "nothing happened" from "some of it happened", which a bare error or a
/// bare `()` cannot express.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeleteRangeReceipt {
    pub committed_chunks: u64,
    /// Position of the last chunk that applied; `None` when no chunk was needed.
    pub last_applied: Option<AppliedPosition>,
}

/// A resolved region location handed back by routing (DESIGN §11 `GetRegion`).
#[derive(Debug, Clone)]
pub struct RegionLocation {
    pub region: kv9_common::RegionId,
    pub epoch: RegionEpoch,
    pub leader: Option<kv9_common::NodeId>,
}

/// Result of creating a keyspace. Production returns the exact Raft proposal
/// identity so acceptance and clients can correlate the write across failover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreateKeyspaceResult {
    pub keyspace: KeyspaceId,
    pub proposed: Option<AppliedPosition>,
}

/// A Raft position is identified by term and index; index alone is unsafe after
/// leader failover because the new leader may overwrite an uncommitted slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppliedPosition {
    pub term: u64,
    pub index: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MembershipChangeResult {
    pub applied: AppliedPosition,
    pub voters: Vec<u64>,
    pub learners: Vec<u64>,
}

/// The admin / meta API (DESIGN §11 Admin surface). Authenticated from day one.
pub trait AdminApi {
    fn create_keyspace(
        &self,
        caller: &str,
        name: &str,
        tenant: kv9_common::TenantId,
        api_type: kv9_common::ApiType,
        txn_group: kv9_common::TxnGroupId,
    ) -> Result<CreateKeyspaceResult>;
    fn list_keyspaces(&self, caller: &str) -> Result<Vec<kv9_common::Keyspace>>;
    fn get_region(&self, caller: &str, keyspace: KeyspaceId, key: &[u8]) -> Result<RegionLocation>;
    fn split_region(
        &self,
        caller: &str,
        region: kv9_common::RegionId,
        split_key: UserKey,
    ) -> Result<()>;
    fn cluster_info(&self, caller: &str) -> Result<ClusterInfo>;
    fn admit_node(
        &self,
        _caller: &str,
        _node: kv9_common::NodeId,
        _addr: &str,
        _ttl_seconds: u64,
    ) -> Result<MembershipChangeResult> {
        Err(kv9_common::Error::NotImplemented("AdminApi::admit_node"))
    }
    fn promote_node(
        &self,
        _caller: &str,
        _node: kv9_common::NodeId,
    ) -> Result<MembershipChangeResult> {
        Err(kv9_common::Error::NotImplemented("AdminApi::promote_node"))
    }
}

/// A snapshot of cluster state (DESIGN §11 `ClusterInfo`).
#[derive(Debug, Clone, Default)]
pub struct ClusterInfo {
    pub node_count: usize,
    pub keyspace_count: usize,
    pub region_count: usize,
}

/// The router API: locate a region for a key (DESIGN §11 Router surface).
pub trait RouterApi {
    fn locate(&self, keyspace: KeyspaceId, key: &[u8]) -> Result<RegionLocation>;
}
