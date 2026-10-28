# kv9 — Metadata Catalog (a small embedded SQL engine)

A minimal relational engine that manages **all cluster metadata** as tables, running on kv9's **own** replicated
KV (the system keyspace). See `DESIGN.md` §5.0/§5.1. This doc is the concrete, simple design.

---

## 1. Goal & scope

- Manage metadata (tenants, keyspaces, regions, peers, placement, SST refcounts, quotas) as **relational tables**
  with **auto-maintained indexes** and **transactions**, so control-plane consistency is structural, not by convention.
- **Small on purpose.** Fixed, versioned schema (no user DDL). Point get, range scan, index scan, and a handful of
  code-level joins over the *known* query set. **No** SQL parser (v0), **no** cost-based optimizer, **no** planner.
  SQL text is an optional admin/debug frontend later.
- Runs on the `system` keyspace. No external etcd; a catalog mutation is one atomic Raft-committed batch
  (`CatalogTxn`) of the metadata group.

Not in scope: user tables/data, SQL for user workloads (kv9 is a KV engine).

---

## 2. Tables (fixed schema v1)

```
tenants        (id u32 PK, name text UNIQUE, quota_tokens u64, state)
keyspaces      (id u32 PK, name text UNIQUE, tenant_id → tenants, api_type,          -- api_type = raw (code 1)
                start_key bytes, end_key bytes, state, config blob)              INDEX by_tenant(tenant_id)
-- table ids 3 (txn_groups) and 4 (tso_timelines) are retired with transactions; never reused
nodes          (id u64 PK, addr text, state, last_heartbeat u64, capacity blob)  -- membership
regions        (id u64 PK, keyspace_id → keyspaces, start_key bytes, end_key bytes,
                epoch_conf u64, epoch_ver u64, leader_node u64)                   INDEX by_range(keyspace_id,start_key)
region_peers   (region_id → regions, node_id → nodes, role, PK(region_id,node_id)) INDEX by_node(node_id)
sst_files      (file_id u64 PK, keyspace_id → keyspaces, bytes u64, refcount u32,
                state, created u64)          -- GC/billing view ONLY; LSM structure
                                             -- (level/bounds/region set) lives in each
                                             -- region's raft-replicated manifest
placement_rules(id u32 PK, keyspace_id → keyspaces, replicas u8, constraints blob)
tasks          (id u64 PK, kind, target, state, created u64)                     -- rebalance/split/merge/GC ops
gac_allotments (tenant_id → tenants PK, tokens u64, refreshed u64)
schema_version (v u32)                                                           -- single row
```

Constraints (`UNIQUE`, FK) make illegal states unrepresentable. Hierarchy: **tenant → keyspace → region**; the
keyspace is the write boundary (an atomic range batch never crosses a keyspace or a region, DESIGN §9.1).

---

## 3. Row ↔ KV encoding (TiDB/CockroachDB-style)

The whole catalog lives under the system keyspace prefix. Every physical key is memcomparable so range scans work.

```
row       :  <sys-prefix> <table_id:u32> 'r' <pk-cols memcmp>            → value = <row: tagged column set>
sec-index :  <sys-prefix> <table_id:u32> 'i' <index_id:u8> <idx-cols>   → <pk memcmp>   (unique index: value=pk)
```

- `table_id` / `index_id` are stable small integers from the hardcoded catalog (§5).
- Row values are **tag-length encoded** columns (protobuf-ish), so **adding a column is forward-compatible** — old
  readers skip unknown tags, new readers default missing ones. This is how schema migrations stay non-breaking.
- A point get = one key. A `by_node` lookup = index scan `region_peers/by_node/<node>` → pks → get rows.

---

## 4. Query interface (v0 = typed ops, not SQL text)

A tiny typed API on `MetaStore` (all inside a `system`-group transaction):

```
get(table, pk) -> Option<Row>
insert(table, row)         // maintains all indexes; UNIQUE/FK checked
update(table, pk, changes) // re-maintains affected indexes
delete(table, pk)          // removes index rows too
scan(table, range)              -> Iter<Row>
index_scan(table, index, range) -> Iter<pk>
```

The **known joins** the scheduler/router need are hand-written as index-driven nested lookups, e.g.:

- *regions on node N* → `index_scan(region_peers, by_node, N)` → `get(regions, region_id)`
- *keyspaces of tenant T* → `index_scan(keyspaces, by_tenant, T)`
- *which region owns key K in keyspace KS* → `index_scan(regions, by_range, (KS, ≤K))` last ≤ K, check end_key
- *rebalance candidates* → scan `nodes` for load, `index_scan(region_peers, by_node, hot)` for movable regions

(An optional SQL-text frontend can compile a fixed grammar to these ops for `pd-ctl`-style admin/debug; not needed to run.)

---

## 5. Transactions

A metadata mutation = one **atomic batch of the metadata group** (`begin()` → typed reads/writes across tables
against a stable snapshot → one proposed `CatalogTxn` entry in the planning term). Multi-table changes (e.g. a split writes `regions` + `region_peers`
+ routing) are **atomic**; a failed step rolls back *everything including index rows* — the class of "partial write
left a dangling index" bugs cannot occur. Retries are idempotent (operation carries an id checked against `tasks`).

Today every catalog row lives in one metadata group, so this is exactly one Raft entry. Once the system keyspace is
sharded across L1 regions, a mutation spanning two of them needs an intent-row + idempotent-task protocol inside the
metadata plane (DESIGN §5.1); user data never participates.

---

## 6. Bootstrap (self-describing catalog)

- The **schema of the catalog tables is hardcoded** in the `meta` crate (table_ids, columns, indexes) — like Postgres
  `pg_catalog` / CockroachDB system descriptors. There is no "create the catalog by querying the catalog" regress.
- **L0** (the fixed bootstrap region, plain KV — not SQL) holds: the locations of the L1 metadata regions, and
  `schema_version`. The SQL engine reads its own schema from code, finds the L1 regions via L0, and operates there.
- Cluster init (election winner, `DESIGN.md` §5.2) = write the seed rows: `schema_version=1`, the `default` tenant,
  the system keyspace + its regions. All as one atomic catalog batch.

---

## 7. Schema versioning & migrations (forward-compatible)

- `schema_version` holds the current version. New binaries carry migration steps `vN → vN+1` (add table / add column /
  add index), each run **once** as a `system` transaction, gated by cluster version so a mixed-version cluster stays
  compatible (rolling upgrade — principle 12).
- Column adds are non-breaking by construction (tag-length row encoding, §3). Index adds backfill in a task.

---

## 8. Mapping to code (`meta` crate)

`membership / catalog / routing / placement` stop being four hand-rolled structs and become **one `MetaStore`**:

```
meta/
  store.rs      MetaStore: begin()/batch API over the system keyspace KV
  schema.rs     hardcoded table & index descriptors (table_id, columns, indexes) + schema_version
  codec.rs      row/index key + tag-length value encoding (§3)
  tables.rs     typed row structs + typed accessors/queries (§4) — tenants/keyspaces/regions/peers/...
  migrate.rs    vN→vN+1 migrations (§7)
```

The scheduler (§5.3), router, placement, and GC all read/write through `MetaStore` — one consistent, queryable,
transactional source, sharded and scaled like data.

---

## 9. What this buys (vs hand-rolled KV metadata)

| Hand-rolled KV control plane (the failure modes we studied) | With the catalog engine |
|---|---|
| a failed create leaves an orphaned name→id index | indexes roll back atomically in the txn |
| duplicated "keyspace→group" field goes stale | one FK/join, always current |
| multi-step metadata ops, partial-rollback leaks | one atomic transaction |
| key layout by convention, padding drift, no typed CAS | typed schema + encoding + txn CAS |
| cluster state only in one process's memory | queryable tables, sharded across L1 |
