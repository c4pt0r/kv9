//! Hardcoded catalog schema — table & index descriptors (METADATA-CATALOG §2, §6).
//!
//! The schema of the catalog tables is **hardcoded in the `meta` crate** (like Postgres
//! `pg_catalog` / CockroachDB system descriptors): there is no "create the catalog by
//! querying the catalog" regress (METADATA-CATALOG §6). `table_id` / `index_id` are
//! stable small integers used by the row/index key codec (§3, [`crate::codec`]).
//!
//! The schema is **fixed and versioned** — no user DDL (§1). Adding a column is
//! forward-compatible via the tag-length value encoding ([`crate::codec`]); adding a
//! table or index is a migration step ([`crate::migrate`], §7).

/// The current hardcoded schema version (METADATA-CATALOG §2, §7). New binaries carry
/// migration steps `vN → vN+1`; the persisted `schema_version` row gates which run.
pub const SCHEMA_VERSION: u32 = 1;

/// Stable identifier of a catalog table (METADATA-CATALOG §3). Small integers so the
/// physical key prefix stays compact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TableId(pub u32);

/// Stable identifier of a secondary index within a table (METADATA-CATALOG §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IndexId(pub u8);

/// Stable identifier of a column within a table (METADATA-CATALOG §3). Also the *tag*
/// in the tag-length row value encoding, so a column id is never reused (§3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ColumnId(pub u16);

/// The logical type of a column (used by the value codec and by memcomparable index
/// key encoding). Deliberately small — the catalog stores only these shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnType {
    /// Unsigned integer (u8..=u64), encoded big-endian / memcomparable.
    Uint,
    /// UTF-8 text.
    Text,
    /// Opaque byte string.
    Bytes,
}

/// A column descriptor (METADATA-CATALOG §2).
#[derive(Debug, Clone, Copy)]
pub struct ColumnDesc {
    pub id: ColumnId,
    pub name: &'static str,
    pub ty: ColumnType,
    /// Whether the column participates in the primary key.
    pub pk: bool,
}

/// A secondary index descriptor (METADATA-CATALOG §2, §3).
#[derive(Debug, Clone, Copy)]
pub struct IndexDesc {
    pub id: IndexId,
    pub name: &'static str,
    /// The column ids that make up the index key, in order (memcomparable, §3).
    pub columns: &'static [ColumnId],
    /// A `UNIQUE` index stores the pk as its value and enforces uniqueness on insert;
    /// a non-unique index appends the pk to the key (METADATA-CATALOG §3).
    pub unique: bool,
}

/// A table descriptor (METADATA-CATALOG §2). Columns include the primary-key columns
/// (`pk = true`); `indexes` are the auto-maintained secondary indexes (§4).
#[derive(Debug, Clone, Copy)]
pub struct TableDesc {
    pub id: TableId,
    pub name: &'static str,
    pub columns: &'static [ColumnDesc],
    pub indexes: &'static [IndexDesc],
}

impl TableDesc {
    /// The primary-key columns of this table, in declaration order.
    pub fn pk_columns(&self) -> impl Iterator<Item = &ColumnDesc> {
        self.columns.iter().filter(|c| c.pk)
    }

    /// Look up a column descriptor by id.
    pub fn column(&self, id: ColumnId) -> Option<&ColumnDesc> {
        self.columns.iter().find(|c| c.id == id)
    }

    /// Look up an index descriptor by id.
    pub fn index(&self, id: IndexId) -> Option<&IndexDesc> {
        self.indexes.iter().find(|i| i.id == id)
    }
}

// ---------------------------------------------------------------------------
// Stable table ids (METADATA-CATALOG §2). Never reused.
// ---------------------------------------------------------------------------

pub const TENANTS: TableId = TableId(1);
pub const KEYSPACES: TableId = TableId(2);
pub const TXN_GROUPS: TableId = TableId(3);
pub const TSO_TIMELINES: TableId = TableId(4);
pub const NODES: TableId = TableId(5);
pub const REGIONS: TableId = TableId(6);
pub const REGION_PEERS: TableId = TableId(7);
pub const SST_FILES: TableId = TableId(8);
pub const PLACEMENT_RULES: TableId = TableId(9);
pub const TASKS: TableId = TableId(10);
pub const GAC_ALLOTMENTS: TableId = TableId(11);
pub const SCHEMA_VERSION_TABLE: TableId = TableId(12);

// ---------------------------------------------------------------------------
// Column descriptors, grouped per table. Column ids are table-local tags.
// ---------------------------------------------------------------------------

mod cols {
    use super::{ColumnDesc, ColumnId, ColumnType::*};

    macro_rules! col {
        ($id:expr, $name:expr, $ty:expr, pk) => {
            ColumnDesc { id: ColumnId($id), name: $name, ty: $ty, pk: true }
        };
        ($id:expr, $name:expr, $ty:expr) => {
            ColumnDesc { id: ColumnId($id), name: $name, ty: $ty, pk: false }
        };
    }

    pub const TENANTS: &[ColumnDesc] = &[
        col!(1, "id", Uint, pk),
        col!(2, "name", Text),
        col!(3, "quota_tokens", Uint),
        col!(4, "state", Uint),
    ];

    pub const KEYSPACES: &[ColumnDesc] = &[
        col!(1, "id", Uint, pk),
        col!(2, "name", Text),
        col!(3, "tenant_id", Uint),
        col!(4, "api_type", Uint),
        col!(5, "start_key", Bytes),
        col!(6, "end_key", Bytes),
        col!(7, "state", Uint),
        col!(8, "config", Bytes),
    ];

    pub const TXN_GROUPS: &[ColumnDesc] = &[
        col!(1, "id", Uint, pk),
        col!(2, "keyspace_id", Uint),
        col!(3, "name", Text),
        col!(4, "sub_start", Bytes),
        col!(5, "sub_end", Bytes),
    ];

    pub const TSO_TIMELINES: &[ColumnDesc] = &[
        col!(1, "id", Uint, pk),
        col!(2, "txn_group_id", Uint),
        col!(3, "provider_node", Uint),
        col!(4, "window_hi", Uint),
    ];

    pub const NODES: &[ColumnDesc] = &[
        col!(1, "id", Uint, pk),
        col!(2, "addr", Text),
        col!(3, "state", Uint),
        col!(4, "last_heartbeat", Uint),
        col!(5, "capacity", Bytes),
    ];

    pub const REGIONS: &[ColumnDesc] = &[
        col!(1, "id", Uint, pk),
        col!(2, "keyspace_id", Uint),
        col!(3, "start_key", Bytes),
        col!(4, "end_key", Bytes),
        col!(5, "epoch_conf", Uint),
        col!(6, "epoch_ver", Uint),
        col!(7, "leader_node", Uint),
    ];

    pub const REGION_PEERS: &[ColumnDesc] = &[
        col!(1, "region_id", Uint, pk),
        col!(2, "node_id", Uint, pk),
        col!(3, "role", Uint),
    ];

    pub const SST_FILES: &[ColumnDesc] = &[
        col!(1, "file_id", Uint, pk),
        col!(2, "region_id", Uint),
        col!(3, "level", Uint),
        col!(4, "refcount", Uint),
        col!(5, "smallest", Bytes),
        col!(6, "biggest", Bytes),
        col!(7, "bytes", Uint),
    ];

    pub const PLACEMENT_RULES: &[ColumnDesc] = &[
        col!(1, "id", Uint, pk),
        col!(2, "keyspace_id", Uint),
        col!(3, "replicas", Uint),
        col!(4, "constraints", Bytes),
    ];

    pub const TASKS: &[ColumnDesc] = &[
        col!(1, "id", Uint, pk),
        col!(2, "kind", Uint),
        col!(3, "target", Bytes),
        col!(4, "state", Uint),
        col!(5, "created", Uint),
    ];

    pub const GAC_ALLOTMENTS: &[ColumnDesc] = &[
        col!(1, "tenant_id", Uint, pk),
        col!(2, "tokens", Uint),
        col!(3, "refreshed", Uint),
    ];

    pub const SCHEMA_VERSION: &[ColumnDesc] = &[
        // Single-row table; a fixed pk of 0 keys the one row.
        col!(1, "singleton", Uint, pk),
        col!(2, "v", Uint),
    ];
}

// ---------------------------------------------------------------------------
// Index descriptors (METADATA-CATALOG §2).
// ---------------------------------------------------------------------------

mod idx {
    use super::{ColumnId, IndexDesc, IndexId};

    /// keyspaces.by_tenant(tenant_id) — column id 3.
    pub const KEYSPACES_BY_TENANT: &[IndexDesc] = &[IndexDesc {
        id: IndexId(1),
        name: "by_tenant",
        columns: &[ColumnId(3)],
        unique: false,
    }];

    /// txn_groups.by_keyspace(keyspace_id) — column id 2.
    pub const TXN_GROUPS_BY_KEYSPACE: &[IndexDesc] = &[IndexDesc {
        id: IndexId(1),
        name: "by_keyspace",
        columns: &[ColumnId(2)],
        unique: false,
    }];

    /// regions.by_range(keyspace_id, start_key) — column ids 2, 3.
    pub const REGIONS_BY_RANGE: &[IndexDesc] = &[IndexDesc {
        id: IndexId(1),
        name: "by_range",
        columns: &[ColumnId(2), ColumnId(3)],
        unique: false,
    }];

    /// region_peers.by_node(node_id) — column id 2.
    pub const REGION_PEERS_BY_NODE: &[IndexDesc] = &[IndexDesc {
        id: IndexId(1),
        name: "by_node",
        columns: &[ColumnId(2)],
        unique: false,
    }];

    /// sst_files.by_region(region_id) — column id 2.
    pub const SST_FILES_BY_REGION: &[IndexDesc] = &[IndexDesc {
        id: IndexId(1),
        name: "by_region",
        columns: &[ColumnId(2)],
        unique: false,
    }];

    pub const NONE: &[IndexDesc] = &[];
}

// ---------------------------------------------------------------------------
// The table descriptors (METADATA-CATALOG §2).
// ---------------------------------------------------------------------------

pub const TENANTS_DESC: TableDesc = TableDesc {
    id: TENANTS,
    name: "tenants",
    columns: cols::TENANTS,
    indexes: idx::NONE,
};

pub const KEYSPACES_DESC: TableDesc = TableDesc {
    id: KEYSPACES,
    name: "keyspaces",
    columns: cols::KEYSPACES,
    indexes: idx::KEYSPACES_BY_TENANT,
};

pub const TXN_GROUPS_DESC: TableDesc = TableDesc {
    id: TXN_GROUPS,
    name: "txn_groups",
    columns: cols::TXN_GROUPS,
    indexes: idx::TXN_GROUPS_BY_KEYSPACE,
};

pub const TSO_TIMELINES_DESC: TableDesc = TableDesc {
    id: TSO_TIMELINES,
    name: "tso_timelines",
    columns: cols::TSO_TIMELINES,
    indexes: idx::NONE,
};

pub const NODES_DESC: TableDesc = TableDesc {
    id: NODES,
    name: "nodes",
    columns: cols::NODES,
    indexes: idx::NONE,
};

pub const REGIONS_DESC: TableDesc = TableDesc {
    id: REGIONS,
    name: "regions",
    columns: cols::REGIONS,
    indexes: idx::REGIONS_BY_RANGE,
};

pub const REGION_PEERS_DESC: TableDesc = TableDesc {
    id: REGION_PEERS,
    name: "region_peers",
    columns: cols::REGION_PEERS,
    indexes: idx::REGION_PEERS_BY_NODE,
};

pub const SST_FILES_DESC: TableDesc = TableDesc {
    id: SST_FILES,
    name: "sst_files",
    columns: cols::SST_FILES,
    indexes: idx::SST_FILES_BY_REGION,
};

pub const PLACEMENT_RULES_DESC: TableDesc = TableDesc {
    id: PLACEMENT_RULES,
    name: "placement_rules",
    columns: cols::PLACEMENT_RULES,
    indexes: idx::NONE,
};

pub const TASKS_DESC: TableDesc = TableDesc {
    id: TASKS,
    name: "tasks",
    columns: cols::TASKS,
    indexes: idx::NONE,
};

pub const GAC_ALLOTMENTS_DESC: TableDesc = TableDesc {
    id: GAC_ALLOTMENTS,
    name: "gac_allotments",
    columns: cols::GAC_ALLOTMENTS,
    indexes: idx::NONE,
};

pub const SCHEMA_VERSION_DESC: TableDesc = TableDesc {
    id: SCHEMA_VERSION_TABLE,
    name: "schema_version",
    columns: cols::SCHEMA_VERSION,
    indexes: idx::NONE,
};

/// Every catalog table descriptor, in stable id order (METADATA-CATALOG §2). Used to
/// bootstrap the catalog and to drive generic codec/migration passes.
pub const ALL_TABLES: &[TableDesc] = &[
    TENANTS_DESC,
    KEYSPACES_DESC,
    TXN_GROUPS_DESC,
    TSO_TIMELINES_DESC,
    NODES_DESC,
    REGIONS_DESC,
    REGION_PEERS_DESC,
    SST_FILES_DESC,
    PLACEMENT_RULES_DESC,
    TASKS_DESC,
    GAC_ALLOTMENTS_DESC,
    SCHEMA_VERSION_DESC,
];

/// Look up a table descriptor by its stable id (METADATA-CATALOG §2).
pub fn table_desc(id: TableId) -> Option<&'static TableDesc> {
    ALL_TABLES.iter().find(|t| t.id == id)
}
