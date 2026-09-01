# kv9

A modern, **multi-tenant-first**, cloud-native distributed key-value engine — inspired by TiKV, but delivered as a
**single binary** with **no separate control plane**, **self-hosted metadata**, and **object storage as the single
source of truth**.

> Status: **v0 — design + compilable skeleton.** APIs and internals are not stable. This repo currently contains the
> design, the architecture diagrams, and a Rust workspace skeleton with real module boundaries (method bodies are
> stubs). See the milestones in [`DESIGN.md`](DESIGN.md#15-milestones).

## Why kv9

- **Object storage is THE source of truth; compute is stateless.** All durable data lives in object storage
  (S3/GCS/Azure); compute holds only cache + a transient, raft-replicated WAL that stages the not-yet-flushed tail.
  This is the defining cloud-native property — it makes nodes **elastic** (add/kill in seconds, scale idle tenants to
  zero, instant failover), scales storage and compute **independently**, and inherits object storage's durability and
  ~10× lower cost. (`DESIGN.md` §6.5)
- **Scale-out is a metadata rearrange, not a data move.** Moving a region ships only a conf-change + a tiny
  **manifest** (file references) + the small log tail; the new replica shares the same object-storage files and pulls
  blocks lazily. Cost is `O(metadata)`, not `O(region data)` — seconds, no bulk copy. (The only real cost is
  cold-cache warm-up, mitigated by learner-first + prefetch.)
- **Multi-tenancy is the core, not a feature.** Every layer is tenant-aware — namespace, capacity, QoS, cache,
  timestamp ordering, blast radius, and billing are all scoped per tenant/keyspace. (`DESIGN.md` §1.1)
- **Single binary, no placement driver.** One `kv9` process is every role (storage node, metadata member, router).
  A cluster is N identical processes — no external PD/etcd/lock service.
- **Self-hosted, layered metadata.** Cluster metadata (region routing, keyspace catalog, placement) is just data in a
  reserved *system keyspace*, replicated by the same Raft as user data. It is **multi-level and sharded** (L0 root →
  L1 meta-regions → L2 user regions), so it scales like data instead of living in one node's memory.
- **One token-based flow-control system.** Capacity (per-tenant admission), fairness (weighted fair queue), and
  **backpressure** (credit feedback from flush/upload/compaction progress) are unified in a single token currency —
  so an overloaded pipeline throttles the tenant *causing* it, not a neighbor. (`DESIGN.md` §7)
- **Elastic throughput at the hot spots.** Range-sharded **regions** with split/merge (**split by consumed
  throughput**, not just size, + DynamoDB-style pre-sharding); **TSO sharded per txn group**; **WAL is a pool of
  streams** — the serialization points scale out.

## Design principles (see [`DESIGN.md`](DESIGN.md#13-design-principles-distilled) §13)

0. **Object storage is the single source of truth; compute is stateless.**
1. No separate control plane — metadata is self-hosted, layered, sharded.
2. Tenant-first everywhere; tenant/keyspace/txn-group are first-class types.
3. Region boundaries align to keyspace boundaries (contained blast radius).
4. Validate encoded ids — cross-tenant misrouting must be impossible.
5. Shard the hot serialization points (per-group TSO, WAL stream pool).
6. Backpressure, not collapse (credit-based token flow control).
7. Consumption-aware placement + split-by-throughput (not disk-capacity/size).
8. Warm, steadily-refreshed metadata caches (no bimodal cold start).
9. Control/management-plane auth from day one.
10. Keyspace-aware deadlock detection.
11–17. Quiesce idle regions · forward-compatible formats / never panic on the unknown · no unquota'd in-memory path ·
    watermark discipline · atomic idempotent metadata · enforced (not hoped) invariants · per-tenant observability +
    tests first-class.

## Concepts

- **Tenant** — isolation & accounting boundary; owns keyspaces.
- **Keyspace** — namespace unit, declared with a tenant and an API type: `txn` (MVCC + Percolator 2PC + Snapshot
  Isolation) or `raw` (direct KV). Encoded as a key prefix.
- **Txn group** — a transaction/timestamp domain (default `default`). A transaction never crosses a group; each group
  has its own sharded TSO timeline. Cross-group transactions require an opt-in stronger timestamp tier (HLC / Spanner
  TrueTime — `DESIGN.md` §8.2).
- **Region** — range shard = a Raft group; the unit of replication, placement, split/merge. Never spans a keyspace.
  Its durable data is immutable SSTs in object storage; its **manifest** (the mutable pointer) lives in raft.

### Read semantics — linearizable

Raw reads are **linearizable**. Every read first establishes a ReadIndex quorum round-trip; only then is the state it
answers from taken. *"The write returned success, therefore the next read observes it"* holds, including across a
leadership change.

- **A node that cannot establish that round-trip does not answer.** It fails instead, and the failure is one of two
  named, machine-readable outcomes: `read_unconfirmed` (the barrier did not fully establish within the deadline) or
  `not_leader` (carrying the leader's id when this node knows it). Retry or re-route on either.
- **An ordinary transport error or timeout is not one of those.** What you can conclude from one is that *you did not
  receive* a typed verdict — not that the server failed to reach one; a timed-out or lost response may sit on either
  side of a decision that was actually made. Neither marker is present, so it cannot be mistaken for a refusal.
  Client code that needs to tell "the cluster refused" from "I did not get an answer" must match the named fields
  rather than the message text.
- **Which of the two named outcomes you get is not guaranteed** at any given moment — both are correct refusals, and
  which one appears depends on where the cluster is in reacting. Do not build logic that requires a particular one.
- **This is held by a partition test, not by this paragraph.** A leader is cut from its peers and every public raw
  read is required to fail, as one of the named outcomes, for the *whole* isolation. See
  [`DESIGN.md`](DESIGN.md) §9.2 for the mechanism and the acceptance split.

## Influences

Bigtable (OSDI 2006) · Spanner (OSDI 2012) · Amazon DynamoDB (USENIX ATC 2022) · TiKV (open source); the
storage-compute-disaggregation lineage of Aurora / Neon / Snowflake. See the influences table in
[`DESIGN.md`](DESIGN.md#2-influences-and-what-we-take-from-each).

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
