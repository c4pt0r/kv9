# kv9

A modern, **multi-tenant-first**, cloud-native distributed key-value engine — inspired by TiKV, but delivered as a
**single binary** with **no separate control plane** and **self-hosted, self-scaling metadata**.

> Status: **v0 — design + compilable skeleton.** APIs and internals are not stable. This repo currently contains the
> design, the architecture diagrams, and a Rust workspace skeleton with real module boundaries (method bodies are
> stubs). See the milestones in [`DESIGN.md`](DESIGN.md#15-milestones).

## Why kv9

- **Multi-tenancy is the core, not a feature.** Every layer is tenant-aware — namespace, capacity, QoS, timestamp
  ordering, blast radius, and billing are all scoped per tenant/keyspace. (`DESIGN.md` §1.1)
- **Single binary, no placement driver.** One `kv9` process is every role (storage node, metadata member, router).
  A cluster is N identical processes — no external PD/etcd/lock service.
- **Self-hosted, layered metadata.** Cluster metadata (region routing, keyspace catalog, placement) is just data in a
  reserved *system keyspace*, replicated by the same Raft as user data. It is **multi-level and sharded**, so it
  scales like data instead of living in one node's memory.
- **Elastic throughput.** Range-sharded **regions** with split/merge; **split is driven by consumed throughput**, not
  just size. **TSO is sharded per txn group** and the **WAL is a pool of streams**, so the hot serialization points
  scale out.

## Design principles (see `DESIGN.md` §13)

1. No separate control plane — metadata is self-hosted, layered, sharded.
2. Tenant-first everywhere; tenant/keyspace/txn-group are first-class types.
3. Region boundaries align to keyspace boundaries (contained blast radius).
4. Validate encoded ids — cross-tenant misrouting must be impossible.
5. Shard the hot serialization points (per-group TSO, WAL stream pool).
6. Backpressure, not collapse.
7. Consumption-aware placement + split-by-throughput.
8. Warm, steadily-refreshed metadata caches (no bimodal cold start).
9. Control/management-plane auth from day one.
10. Keyspace-aware deadlock detection.

## Concepts

- **Tenant** — isolation & accounting boundary; owns keyspaces.
- **Keyspace** — namespace unit, declared with a tenant and an API type: `txn` (MVCC + Percolator 2PC + Snapshot
  Isolation) or `raw` (direct KV). Encoded as a key prefix.
- **Txn group** — a transaction/timestamp domain (default `default`). A transaction never crosses a group; each group
  has its own sharded TSO timeline.
- **Region** — range shard = a Raft group; the unit of replication, placement, split/merge. Never spans a keyspace.

## Influences

Bigtable (OSDI 2006) · Spanner (OSDI 2012) · Amazon DynamoDB (USENIX ATC 2022) · TiKV (open source). See the
influences table in [`DESIGN.md`](DESIGN.md#2-influences-and-what-we-take-from-each).

## Layout

```
DESIGN.md              full architecture & rationale (source of truth for module boundaries)
docs/ARCHITECTURE.md   ASCII architecture diagrams
crates/
  common/  meta/  engine/  raft/  region/  txn/  server/
src/                   the single `kv9` binary
```

Build (v0 skeleton): `cargo check --workspace`.

## License

Apache-2.0 — see [`LICENSE`](LICENSE).
