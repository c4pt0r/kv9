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

/// A foreign-key declaration: `column` must reference an existing pk in `references`
/// (METADATA-CATALOG §2 — "Constraints (`UNIQUE`, FK) make illegal states
/// unrepresentable"). Enforced on insert/update against the txn's merged view.
#[derive(Debug, Clone, Copy)]
pub struct FkDesc {
    pub column: ColumnId,
    pub references: TableId,
}

/// A table descriptor (METADATA-CATALOG §2). Columns include the primary-key columns
/// (`pk = true`); `indexes` are the auto-maintained secondary indexes (§4); `fks` are
/// the declared foreign keys.
#[derive(Debug, Clone, Copy)]
pub struct TableDesc {
    pub id: TableId,
    pub name: &'static str,
    pub columns: &'static [ColumnDesc],
    pub indexes: &'static [IndexDesc],
    pub fks: &'static [FkDesc],
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
pub const ID_SEQUENCES: TableId = TableId(13);
/// Singleton cluster identity (task #24, gate 2): one row, pk 0, holding the
/// immutable 16-byte ClusterId minted by the bootstrap winner.
pub const CLUSTER_META: TableId = TableId(14);
/// Node-admission records (task #24, gate 3): a leader-committed approval that
/// binds `cluster + node_id + address + role` BEFORE the node may join.
/// A valid cluster token alone is never admission.
pub const NODE_ADMISSIONS: TableId = TableId(15);

// ---------------------------------------------------------------------------
// Column descriptors, grouped per table. Column ids are table-local tags.
// ---------------------------------------------------------------------------

mod cols {
    use super::{ColumnDesc, ColumnId, ColumnType::*};

    macro_rules! col {
        ($id:expr, $name:expr, $ty:expr, pk) => {
            ColumnDesc {
                id: ColumnId($id),
                name: $name,
                ty: $ty,
                pk: true,
            }
        };
        ($id:expr, $name:expr, $ty:expr) => {
            ColumnDesc {
                id: ColumnId($id),
                name: $name,
                ty: $ty,
                pk: false,
            }
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

    // GC/billing view ONLY (agreed design ruling): which files a region holds is
    // answered by that region's raft-replicated manifest, the single authority for
    // LSM structure. `keyspace_id` is stable even when a split shares a file across
    // regions (regions never span keyspaces) and drives per-tenant billing.
    // Column ids 2/3/5/6 (region_id/level/smallest/biggest) are RETIRED — never reuse.
    pub const SST_FILES: &[ColumnDesc] = &[
        col!(1, "file_id", Uint, pk),
        col!(4, "refcount", Uint),
        col!(7, "bytes", Uint),
        col!(8, "keyspace_id", Uint),
        col!(9, "state", Uint),
        col!(10, "created", Uint),
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

    /// System id sequences (one row per [`crate::store::SequenceKind`]).
    pub const CLUSTER_META: &[ColumnDesc] = &[
        col!(1, "id", Uint, pk),      // always 0: singleton
        col!(2, "cluster_id", Bytes), // 16 raw bytes (kv9_common::ClusterId)
        col!(3, "created_unix", Uint),
    ];

    pub const NODE_ADMISSIONS: &[ColumnDesc] = &[
        col!(1, "node_id", Uint, pk),
        // Row-level binding to THIS cluster (16 raw ClusterId bytes): the
        // gate-3 contract binds every admission to (cluster_id, node_id,
        // address) — inferring the cluster from the singleton table would
        // leave rows meaningful outside their cluster (Tess's review).
        col!(2, "cluster_id", Bytes),
        col!(3, "addr", Text),
        col!(4, "role", Uint),  // AdmittedRole: 1 learner
        col!(5, "state", Uint), // AdmissionState: 1 pending, 2 consumed, 3 revoked
        // SHA-256 of the one-time join-ticket nonce — NOT a checksum hash
        // (FNV would cap a credential at 64 bits). The ticket seam is NOT
        // implemented in this block: the column is declared for the schema's
        // final shape, no code writes or compares it yet, and the minimal
        // admission mode is mandatory --cluster-id. When implemented the
        // compare must be constant-time and never logged.
        col!(6, "nonce_sha256", Bytes),
        col!(7, "expires_unix", Uint),
    ];

    pub const ID_SEQUENCES: &[ColumnDesc] = &[col!(1, "kind", Uint, pk), col!(2, "next", Uint)];
}

// ---------------------------------------------------------------------------
// Index descriptors (METADATA-CATALOG §2).
// ---------------------------------------------------------------------------

mod idx {
    use super::{ColumnId, IndexDesc, IndexId};

    /// tenants.by_name(name) UNIQUE — column id 2 (METADATA-CATALOG §2 `name text UNIQUE`).
    pub const TENANTS_IDX: &[IndexDesc] = &[IndexDesc {
        id: IndexId(1),
        name: "by_name",
        columns: &[ColumnId(2)],
        unique: true,
    }];

    /// keyspaces.by_tenant(tenant_id) — column id 3 — and by_name(name) UNIQUE —
    /// column id 2 (METADATA-CATALOG §2 `name text UNIQUE`).
    pub const KEYSPACES_IDX: &[IndexDesc] = &[
        IndexDesc {
            id: IndexId(1),
            name: "by_tenant",
            columns: &[ColumnId(3)],
            unique: false,
        },
        IndexDesc {
            id: IndexId(2),
            name: "by_name",
            columns: &[ColumnId(2)],
            unique: true,
        },
    ];

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

    pub const NONE: &[IndexDesc] = &[];
}

// ---------------------------------------------------------------------------
// Foreign keys (METADATA-CATALOG §2 arrows). Insert/update-side enforced.
// ---------------------------------------------------------------------------

mod fk {
    use super::{ColumnId, FkDesc, TableId};

    pub const NONE: &[FkDesc] = &[];
    pub const KEYSPACES: &[FkDesc] = &[FkDesc {
        column: ColumnId(3),
        references: super::TENANTS,
    }];
    pub const TXN_GROUPS: &[FkDesc] = &[FkDesc {
        column: ColumnId(2),
        references: super::KEYSPACES,
    }];
    pub const TSO_TIMELINES: &[FkDesc] = &[
        FkDesc {
            column: ColumnId(2),
            references: super::TXN_GROUPS,
        },
        FkDesc {
            column: ColumnId(3),
            references: super::NODES,
        },
    ];
    pub const REGIONS: &[FkDesc] = &[FkDesc {
        column: ColumnId(2),
        references: super::KEYSPACES,
    }];
    pub const REGION_PEERS: &[FkDesc] = &[
        FkDesc {
            column: ColumnId(1),
            references: super::REGIONS,
        },
        FkDesc {
            column: ColumnId(2),
            references: super::NODES,
        },
    ];
    pub const SST_FILES: &[FkDesc] = &[FkDesc {
        column: ColumnId(8),
        references: super::KEYSPACES,
    }];
    pub const PLACEMENT_RULES: &[FkDesc] = &[FkDesc {
        column: ColumnId(2),
        references: super::KEYSPACES,
    }];
    pub const GAC_ALLOTMENTS: &[FkDesc] = &[FkDesc {
        column: ColumnId(1),
        references: super::TENANTS,
    }];
    // Suppress an unused warning if a table stops declaring FKs.
    const _: TableId = super::TENANTS;
}

// ---------------------------------------------------------------------------
// The table descriptors (METADATA-CATALOG §2).
// ---------------------------------------------------------------------------

pub const TENANTS_DESC: TableDesc = TableDesc {
    id: TENANTS,
    name: "tenants",
    columns: cols::TENANTS,
    indexes: idx::TENANTS_IDX,
    fks: fk::NONE,
};

pub const KEYSPACES_DESC: TableDesc = TableDesc {
    id: KEYSPACES,
    name: "keyspaces",
    columns: cols::KEYSPACES,
    indexes: idx::KEYSPACES_IDX,
    fks: fk::KEYSPACES,
};

pub const TXN_GROUPS_DESC: TableDesc = TableDesc {
    id: TXN_GROUPS,
    name: "txn_groups",
    columns: cols::TXN_GROUPS,
    indexes: idx::TXN_GROUPS_BY_KEYSPACE,
    fks: fk::TXN_GROUPS,
};

pub const TSO_TIMELINES_DESC: TableDesc = TableDesc {
    id: TSO_TIMELINES,
    name: "tso_timelines",
    columns: cols::TSO_TIMELINES,
    indexes: idx::NONE,
    fks: fk::TSO_TIMELINES,
};

pub const NODES_DESC: TableDesc = TableDesc {
    id: NODES,
    name: "nodes",
    columns: cols::NODES,
    indexes: idx::NONE,
    fks: fk::NONE,
};

pub const REGIONS_DESC: TableDesc = TableDesc {
    id: REGIONS,
    name: "regions",
    columns: cols::REGIONS,
    indexes: idx::REGIONS_BY_RANGE,
    fks: fk::REGIONS,
};

pub const REGION_PEERS_DESC: TableDesc = TableDesc {
    id: REGION_PEERS,
    name: "region_peers",
    columns: cols::REGION_PEERS,
    indexes: idx::REGION_PEERS_BY_NODE,
    fks: fk::REGION_PEERS,
};

pub const SST_FILES_DESC: TableDesc = TableDesc {
    id: SST_FILES,
    name: "sst_files",
    columns: cols::SST_FILES,
    indexes: idx::NONE,
    fks: fk::SST_FILES,
};

pub const PLACEMENT_RULES_DESC: TableDesc = TableDesc {
    id: PLACEMENT_RULES,
    name: "placement_rules",
    columns: cols::PLACEMENT_RULES,
    indexes: idx::NONE,
    fks: fk::PLACEMENT_RULES,
};

pub const TASKS_DESC: TableDesc = TableDesc {
    id: TASKS,
    name: "tasks",
    columns: cols::TASKS,
    indexes: idx::NONE,
    fks: fk::NONE,
};

pub const GAC_ALLOTMENTS_DESC: TableDesc = TableDesc {
    id: GAC_ALLOTMENTS,
    name: "gac_allotments",
    columns: cols::GAC_ALLOTMENTS,
    indexes: idx::NONE,
    fks: fk::GAC_ALLOTMENTS,
};

pub const SCHEMA_VERSION_DESC: TableDesc = TableDesc {
    id: SCHEMA_VERSION_TABLE,
    name: "schema_version",
    columns: cols::SCHEMA_VERSION,
    indexes: idx::NONE,
    fks: fk::NONE,
};

pub const ID_SEQUENCES_DESC: TableDesc = TableDesc {
    id: ID_SEQUENCES,
    name: "id_sequences",
    columns: cols::ID_SEQUENCES,
    indexes: idx::NONE,
    fks: fk::NONE,
};

pub const CLUSTER_META_DESC: TableDesc = TableDesc {
    id: CLUSTER_META,
    name: "cluster_meta",
    columns: cols::CLUSTER_META,
    indexes: idx::NONE,
    fks: fk::NONE,
};

pub const NODE_ADMISSIONS_DESC: TableDesc = TableDesc {
    id: NODE_ADMISSIONS,
    name: "node_admissions",
    columns: cols::NODE_ADMISSIONS,
    indexes: idx::NONE,
    fks: fk::NONE,
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
    ID_SEQUENCES_DESC,
    CLUSTER_META_DESC,
    NODE_ADMISSIONS_DESC,
];

/// Look up a table descriptor by its stable id (METADATA-CATALOG §2).
pub fn table_desc(id: TableId) -> Option<&'static TableDesc> {
    ALL_TABLES.iter().find(|t| t.id == id)
}
