# kv9-meta

The self-hosted metadata plane — no external placement driver (DESIGN §5, §10; docs/METADATA-CATALOG.md).
Metadata is just data in a reserved system keyspace, replicated by the same Raft as user data.

- `store`, `schema`, `codec`, `tables`, `migrate` — `MetaStore`: a small relational layer over the engine
  with a hardcoded, versioned schema (tenants, keyspaces, nodes, regions, region peers, tasks, …),
  auto-maintained indexes and PK/UNIQUE/FK checks. One metadata mutation is one `CatalogTxn` Raft entry.
- `catalog`, `membership`, `routing`, `endpoint`, `admission` — keyspaces and tenants, node membership and
  learner admission, the region routing snapshot, node endpoints.
- `bootstrap` — the **election-first** state machine
  (`Discovering → BootstrapElection → Initializing | WaitForBootstrap → Serving`), DESIGN §5.2.
- `root`, `layered`, `leader` — the certified root descriptor, the `L0`/`L1` metadata layering (one group
  today; sharding is design) and `MetaLeader` of `META_REGION_0`.
- `data_groups` — committed task history for data groups: creation and activation, range bindings, the
  migration protocol (capture, evidence, learner attach, abort, truncation, removal, compaction floors) and
  store history. Range split (task kind 107) is retired here and lives on `experimental/range-split`.
- `retention`, `checkpoint`, `recovery`, `task_history`, `placement` — the retention ledger, checkpoint
  selection, recovery helpers and placement scoring.
