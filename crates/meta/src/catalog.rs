//! Keyspace catalog & tenants (DESIGN §5.1, §3.1–§3.2, §3.6).

use std::collections::HashMap;

use kv9_common::codec::validate_keyspace_id;
use kv9_common::{ApiType, Error, Keyspace, KeyspaceId, Result, Tenant, TenantId, TxnGroupId};

/// The keyspace catalog + tenant registry (DESIGN §5.1). Stored as ordinary KV in the
/// system keyspace (in L1 meta-regions once split — DESIGN §5.1.1).
#[derive(Debug, Default)]
pub struct Catalog {
    tenants: HashMap<TenantId, Tenant>,
    keyspaces: HashMap<KeyspaceId, Keyspace>,
    by_name: HashMap<String, KeyspaceId>,
}

impl Catalog {
    pub fn new() -> Self {
        Catalog::default()
    }

    pub fn upsert_tenant(&mut self, tenant: Tenant) {
        self.tenants.insert(tenant.id, tenant);
    }

    pub fn tenant(&self, id: TenantId) -> Option<&Tenant> {
        self.tenants.get(&id)
    }

    /// Create a keyspace (DESIGN §3.2, §10 `CreateKeyspace`).
    ///
    /// Validates the keyspace-id width (DESIGN §3.4, §13 principle 4), rejects duplicate names,
    /// and requires the owning tenant to exist. For `raw` keyspaces the txn group is
    /// ignored; for `txn` keyspaces it defaults to [`TxnGroupId::DEFAULT`] unless given.
    pub fn create_keyspace(
        &mut self,
        id: KeyspaceId,
        name: impl Into<String>,
        tenant: TenantId,
        api_type: ApiType,
        txn_group: TxnGroupId,
    ) -> Result<&Keyspace> {
        validate_keyspace_id(id)?;
        let name = name.into();
        if self.by_name.contains_key(&name) {
            return Err(Error::Config(format!(
                "keyspace name '{name}' already exists"
            )));
        }
        if !self.tenants.contains_key(&tenant) {
            return Err(Error::Config(format!("tenant {tenant:?} does not exist")));
        }
        let ks = Keyspace {
            id,
            name: name.clone(),
            tenant,
            api_type,
            txn_group,
        };
        self.by_name.insert(name, id);
        self.keyspaces.insert(id, ks);
        Ok(self.keyspaces.get(&id).expect("just inserted"))
    }

    pub fn keyspace(&self, id: KeyspaceId) -> Result<&Keyspace> {
        self.keyspaces.get(&id).ok_or(Error::KeyspaceNotFound(id))
    }

    pub fn keyspace_by_name(&self, name: &str) -> Option<&Keyspace> {
        self.by_name.get(name).and_then(|id| self.keyspaces.get(id))
    }

    pub fn list_keyspaces(&self) -> impl Iterator<Item = &Keyspace> {
        self.keyspaces.values()
    }
}
