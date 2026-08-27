//! Tenant, keyspace, and API-type model (DESIGN §3.1–§3.2, §3.6).

use serde::{Deserialize, Serialize};

use crate::ids::{KeyspaceId, TenantId, TxnGroupId};

/// The API surface a keyspace exposes (DESIGN §3.2). A keyspace cannot mix the two.
///
/// The mode byte in the physical key encoding is derived from this (DESIGN §3.4):
/// `Txn` → `'t'`, `Raw` → `'r'`. The reserved system keyspace uses `'s'` (see
/// [`KeyMode::System`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ApiType {
    /// MVCC + Percolator 2PC + Snapshot Isolation (DESIGN §3.2, §9.1).
    Txn,
    /// Direct KV, optional TTL / causal timestamps, no transactions (DESIGN §3.2, §9.2).
    Raw,
}

/// An isolation and accounting boundary that owns keyspaces (DESIGN §3.1).
///
/// Capacity (read/write units) and blast radius are scoped per tenant; a tenant
/// never sees another tenant's keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tenant {
    pub id: TenantId,
    pub name: String,
    /// Provisioned read-capacity units (DynamoDB-style GAC input — DESIGN §10).
    pub read_capacity_units: u64,
    /// Provisioned write-capacity units (DESIGN §10).
    pub write_capacity_units: u64,
}

/// The namespace unit, declared once with immutable core attributes (DESIGN §3.2).
///
/// A keyspace maps to a numeric [`KeyspaceId`] and a contiguous key range in the
/// global keyspace via the prefix encoding in [`crate::codec`]. All of a keyspace's
/// regions live within that range (DESIGN §3.2, §3.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keyspace {
    pub id: KeyspaceId,
    pub name: String,
    /// Owning tenant (DESIGN §3.1).
    pub tenant: TenantId,
    /// `txn` or `raw` (DESIGN §3.2).
    pub api_type: ApiType,
    /// The txn group this keyspace belongs to (DESIGN §3.6). Ignored for `raw`
    /// keyspaces; every `txn` keyspace belongs to exactly one group, defaulting to
    /// [`TxnGroupId::DEFAULT`].
    pub txn_group: TxnGroupId,
}

impl Keyspace {
    /// The key mode byte for this keyspace's api type (DESIGN §3.4).
    pub fn key_mode(&self) -> crate::codec::KeyMode {
        match self.api_type {
            ApiType::Txn => crate::codec::KeyMode::Txn,
            ApiType::Raw => crate::codec::KeyMode::Raw,
        }
    }
}
