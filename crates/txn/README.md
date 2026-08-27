# kv9-txn

The transaction executors (DESIGN §9).

- `percolator` — the Percolator 2PC executor for `txn` keyspaces: `Get/Prewrite/Commit`
  over the `default/lock/write` MVCC layout (DESIGN §9.1), plus the **txn-group
  confinement check** that rejects a transaction whose keys resolve to two different txn
  groups (DESIGN §3.6, §9.1) — the invariant that lets each group use its own sharded
  TSO timeline.
- `raw` — the direct-KV executor for `raw` keyspaces (`RawPut/RawGet/RawDelete/RawScan`),
  optional TTL / causal timestamps, no locks, no 2PC (DESIGN §9.2).

The keyspace's `api_type` selects the executor at the routing layer (DESIGN §9.2, §11).
