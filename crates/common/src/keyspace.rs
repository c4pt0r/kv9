//! Tenant, keyspace, and API-type model (DESIGN §3.1–§3.2).

use serde::{Deserialize, Serialize};

use crate::ids::{KeyspaceId, TenantId};

/// The API surface a keyspace exposes (DESIGN §3.2).
///
/// kv9 is a raw key-value store: there is exactly one user-facing API type. The enum is
/// kept so the catalog row and the wire keep an explicit, validated field instead of an
/// implicit default, and so a stored code that is not `raw` (for example the retired
/// transactional type) is rejected loudly rather than silently reinterpreted.
///
/// The mode byte in the physical key encoding is derived from this (DESIGN §3.4):
/// `Raw` → `'r'`. The reserved system keyspace uses `'s'` (see
/// [`crate::codec::KeyMode::System`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ApiType {
    /// Direct KV with atomic range batches and optional TTL (DESIGN §3.2, §9).
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
    /// Always `raw` (DESIGN §3.2).
    pub api_type: ApiType,
}

impl Keyspace {
    /// The key mode byte for this keyspace's api type (DESIGN §3.4).
    pub fn key_mode(&self) -> crate::codec::KeyMode {
        match self.api_type {
            ApiType::Raw => crate::codec::KeyMode::Raw,
        }
    }
}
