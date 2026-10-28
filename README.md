# kv9

A **multi-tenant-first**, cloud-native distributed **raw key-value** engine — inspired by TiKV, but delivered as a
**single binary** with **no separate control plane**, **self-hosted metadata**, and **object storage as the source
of truth**.

> Status: **v0, a working distributed Raw KV with multiple Raft groups; APIs and on-disk formats are not stable.**
>
> **Working today**
> - **Self-hosted, highly available metadata.** The catalog (tenants, keyspaces, regions, peers, membership) is
>   replicated by kv9's own `META_REGION_0` Raft group across three voters: elect first, then the winner initializes.
>   Learner admission and promotion, leader failover and full restart are covered by multi-process tests.
> - **Raw KV:** `RawPut/Get/Scan/Delete/DeleteRange/BatchGet/BatchPut`, optional TTL, and the **atomic range
>   batch** `RawBatch` (mixed puts/deletes applied as one Raft entry inside one range — see below). Reads are
>   linearizable through a quorum ReadIndex barrier; writes answer with the exact applied `(term, index)`.
> - **Multiple data groups with automatic split.** User data lives in data groups (regions) separate from the
>   metadata group; a region splits automatically by WAL bytes (`KV9_AUTO_SPLIT_BYTES`), cascades, and preserves TTL
>   state through the split. Epoch fences and leader hints drive client re-routing.
> - **Object storage backend.** With `KV9_STORAGE=minio`, leaders upload immutable SST checkpoints (streamed, no
>   longer capped at 48 MiB), publish manifests by CAS, replicas adopt them after local ordered apply, the covered
>   WAL is reclaimed, and cold recovery reads MinIO plus the surviving WAL tail. Raft logs are compacted by entry
>   count, bytes or age and the log file is physically rewritten; end-to-end write backpressure bounds the retained
>   log.
> - **Data-group migration** (capture, learner attach, evidence, voter promotion, source removal/truncation) with
>   typed abort paths.
>
> **Limits:** the full dataset still resides in memory (SSTs are checkpoints, not a read path — no incremental LSM
> and no block cache yet); range **merge** is not implemented; split is by size, not by consumed throughput;
> metadata is one group (L0/L1 sharding is design only); tenant admission/fairness is partial; TLS is not
> implemented. Acknowledged unflushed writes depend on the Raft replicas' local disks.
>
> **Transactions were removed from the design (2026-10-28).** There is no MVCC, TSO or 2PC; the multi-key write
> primitive is the atomic range batch. See `DESIGN.md` §1 (non-goals), §9.1 and §14.
>
> Delivery order: [ROADMAP](docs/ROADMAP.md). Detailed work packages: [DEVELOPMENT-PATH](docs/DEVELOPMENT-PATH.md),
> tracked in [GitHub issue #9](https://github.com/c4pt0r/kv9/issues/9).

## Run it

```bash
# Three kv9 processes + a real MinIO container; includes failover and full restart.
./scripts/minio-kv-e2e.sh

# Raw client, including an atomic range batch (hex-encoded keys/values):
KV9_CLIENT_TOKEN=... kv9 client raw-batch --addr 127.0.0.1:20161 --keyspace 100 \
    --mutation put:6b31:7631 --mutation put:6b32:7632 --mutation delete:6b30
```

See [QUICKSTART](docs/QUICKSTART.md) for prerequisites, MinIO configuration and a persistent cluster.
Node-local queue, backend, Raft wait and WAL timing are described in [LATENCY-OBSERVABILITY](docs/LATENCY-OBSERVABILITY.md).

## Target architecture

The following are design goals; the status above describes what the current binary implements.

- **Object storage is THE source of truth; compute is stateless.** All durable data lives in object storage
  (S3/GCS/Azure); compute holds only cache + a transient, raft-replicated WAL that stages the not-yet-flushed tail.
  This makes nodes **elastic** (add/kill in seconds, scale idle tenants to zero, instant failover), scales storage
  and compute **independently**, and inherits object storage's durability and ~10× lower cost. (`DESIGN.md` §6.5)
- **Scale-out is a metadata rearrange, not a data move.** Moving a region ships only a conf-change + a tiny
  **manifest** (file references) + the small log tail; the new replica shares the same object-storage files and pulls
  blocks lazily. Cost is `O(metadata)`, not `O(region data)`.
- **Multi-tenancy is the core, not a feature.** Every layer is tenant-aware — namespace, capacity, QoS, cache,
  blast radius, and billing are all scoped per tenant/keyspace. (`DESIGN.md` §1.1)
- **Single binary, no placement driver.** One `kv9` process is every role (storage node, metadata member, router).
  A cluster is N identical processes — no external PD/etcd/lock service.
- **Self-hosted, layered metadata.** Cluster metadata is just data in a reserved *system keyspace*, replicated by the
  same Raft as user data, and **multi-level and sharded** (L0 root → L1 meta-regions → L2 user regions) so it scales
  like data. Today: one highly available metadata group; the L1 sharding is the next step.
- **Automatic sharding.** Range-sharded **regions** with split (implemented, by size) and merge (planned), **split
  by consumed throughput** (planned) and DynamoDB-style pre-sharding; regions never span a keyspace.
- **Atomic range batches instead of transactions.** The only multi-key write is one fenced Raft entry inside one
  region: no timestamps, no locks, no cross-region commit. (`DESIGN.md` §9.1)
- **One token-based flow-control system.** Capacity, fairness and **backpressure** in a single token currency, so an
  overloaded pipeline throttles the tenant *causing* it. (`DESIGN.md` §7)

## Design principles (see [`DESIGN.md`](DESIGN.md#13-design-principles-distilled) §13)

0. **Object storage is the single source of truth; compute is stateless.**
1. No separate control plane — metadata is self-hosted, layered, sharded.
2. Tenant-first everywhere; tenant/keyspace are first-class types.
3. Region boundaries align to keyspace boundaries (contained blast radius).
4. Validate encoded ids — cross-tenant misrouting must be impossible.
5. Shard the hot serialization points (per-region Raft logs, WAL stream pool).
6. Backpressure, not collapse (credit-based token flow control).
7. Consumption-aware placement + split-by-throughput (not disk-capacity/size).
8. Warm, steadily-refreshed metadata caches (no bimodal cold start).
9. Control/management-plane auth from day one.
10. Multi-key writes are atomic range batches, never cross-region transactions.
11–17. Quiesce idle regions · forward-compatible formats / never panic on the unknown · no unquota'd in-memory path ·
    watermark discipline · atomic idempotent metadata · enforced (not hoped) invariants · per-tenant observability +
    tests first-class.

## Concepts

- **Tenant** — isolation & accounting boundary; owns keyspaces.
- **Keyspace** — namespace unit, declared with a tenant. Every keyspace is `raw` (direct KV). Encoded as a key prefix.
- **Region** — range shard = a Raft group; the unit of replication, placement and split. Never spans a keyspace.
  Its durable data is immutable SSTs in object storage; its **manifest** (the mutable pointer) lives in raft.
- **Range batch** — an ordered list of puts/deletes whose keys all lie in one region, applied as one Raft entry.

### Write semantics — atomic within a range

`RawBatch` (and the single-key `RawPut`/`RawDelete`, and the put-only `RawBatchPut`) is **one fenced Raft entry**:
every mutation applies at one `(term, index)` or none does. The boundaries are explicit:

- **All keys must be in one region of one keyspace.** The server resolves the first key's region and checks every
  other key against that region's `[start, end)`. A key outside it fails the whole batch with
  `RangeCrossesRegion`; nothing is proposed. The server never splits a batch across regions, because that would
  silently drop the atomicity you asked for — a client that needs several regions sends one batch per region.
- **The region epoch is checked twice.** Once at admission against the request's `(conf_ver, version)`
  (`StaleEpoch` → refresh the route and retry), and again on every replica at apply time against the region row
  current at that log position. A split that commits between the two makes the entry a typed no-op, never a partial
  write.
- **Order matters, duplicates are allowed:** later mutations for the same key win; deleting an absent key is a no-op.
- **The response is the applied position**, so what you read back is a committed fact.

`RawDeleteRange` is the deliberate exception: it applies in bounded chunks and reports how many committed.

### Read semantics — linearizable

Raw reads are **linearizable**. Every read first establishes a ReadIndex quorum round-trip; only then is the state it
answers from taken. *"The write returned success, therefore the next read observes it"* holds, including across a
leadership change.

- **A node that cannot establish that round-trip does not answer.** It fails instead, and the failure is one of two
  named, machine-readable outcomes: `read_unconfirmed` (the barrier did not fully establish within the deadline) or
  `not_leader` (carrying the leader's id when this node knows it). Retry or re-route on either.
- **An ordinary transport error or timeout is not one of those.** Neither marker is present, so it cannot be mistaken
  for a refusal. Client code that needs to tell "the cluster refused" from "I did not get an answer" must match the
  named fields rather than the message text.
- **This is held by a partition test, not by this paragraph.** See [`DESIGN.md`](DESIGN.md) §9.2.

The asynchronous persistent raw client and its bounded retry contract are described in
[PERSISTENT-CLIENT.md](docs/PERSISTENT-CLIENT.md); the range-aware routed client in
[RAW-BATCH-CLIENT.md](docs/RAW-BATCH-CLIENT.md).

## Influences

Bigtable (OSDI 2006) · Spanner (OSDI 2012) · Amazon DynamoDB (USENIX ATC 2022) · TiKV (open source); the
storage-compute-disaggregation lineage of Aurora / Neon / Snowflake. See the influences table in
[`DESIGN.md`](DESIGN.md#2-influences-and-what-we-take-from-each).

## Layout

```
DESIGN.md              full architecture & rationale (source of truth for module boundaries)
docs/ARCHITECTURE.md   ASCII architecture diagrams
crates/
  common/  meta/  engine/  raft/  region/  raw/  server/
src/                   the single `kv9` binary
proofs/                Lean / TLA+ models pinned to the sources they describe
```

Build: `cargo build --workspace`. Test: `cargo test --workspace` (the qualifying run is serial:
`-- --test-threads=1`); real MinIO tests are explicitly selected in CI. Local automation and experiment output
placement follow [LOCAL-ARTIFACTS](docs/LOCAL-ARTIFACTS.md).

## License

Apache-2.0 — see [`LICENSE`](LICENSE).
