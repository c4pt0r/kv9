# kv9 — Design

> A modern, **multi-tenant-first**, cloud-native distributed key-value engine.
> Inspired by TiKV, but with a **single binary**, **no external control plane (no placement driver)**,
> and **self-hosted metadata** bootstrapped inside a system keyspace.

Status: **v0 design + skeleton milestone.** This document defines the architecture and the crate layout the
skeleton implements. It is the source of truth for module boundaries. Diagrams: `docs/ARCHITECTURE.md`.

---

## 1. Goals & non-goals

### Goals
1. **Multi-tenancy is the core.** kv9 is designed *around* tenants from the first principle, not as an add-on.
   Every layer — namespace, capacity, performance, timestamp ordering, blast radius, placement, billing — is
   tenant-aware. See §1.1.
2. **Single binary.** One `kv9` executable is *every* role: storage node, metadata member, request router. A
   cluster is N identical processes. No separate placement driver, no external etcd, no lock service.
3. **Keyspaces.** The unit of namespacing and configuration, declared with a **tenant** and an **API type**
   (`txn` or `raw`).
4. **Horizontal throughput scaling** via **range-sharded regions** with **split/merge**, where **split is driven
   by consumed throughput, not just size** (DynamoDB 2022).
5. **Familiar API.** The common TiKV surface: transactional and raw.
6. **Correctness first.** Snapshot Isolation for `txn` keyspaces via Percolator-style 2PC over a monotonically
   ordered timestamp; a log-backed WAL; continuous replica verification.

### Non-goals (for now)
- SQL, coprocessor push-down, secondary indexes (kv9 is the storage engine; a SQL layer is out of scope).
- Cross-tenant / cross-group external consistency on commodity clocks at Spanner's TrueTime level (§8).
- Storage-compute disaggregation over object storage (v0 uses local LSM; a shared-storage backend is a documented
  future direction — §12).

### 1.1 Multi-tenancy is the organizing principle
A tenant owns keyspaces; a keyspace has an API type and belongs to a txn group. **Isolation is enforced on every
axis**, and each axis has a concrete mechanism:

| Isolation axis | Mechanism in kv9 | Section |
|----------------|------------------|---------|
| **Namespace** (no tenant sees another's keys) | keyspace-id key prefix; every region lives inside one keyspace's range | §3.4, §4 |
| **Blast radius** (a tenant's failure/mistake stays local) | region boundaries **must align to keyspace boundaries**; per-keyspace GC / encryption / backup | §3.3, §10 |
| **Capacity** (no noisy neighbor) | **Global Admission Control**: per-tenant / per-keyspace token buckets handed out by the metadata plane | §10 |
| **Performance / QoS** | per-keyspace fair scheduling of CPU on the serving threads; per-tenant request budgets | §7, §10 |
| **Timestamp / commit throughput** | **sharded TSO**: per-txn-group timelines served by a pool of providers | §3.6, §8 |
| **Correctness under contention** | **keyspace-aware deadlock detection** (per-tenant wait-for graphs) | §9 |
| **Accounting** | keyspace = the billing/metering unit | §10 |

The design consequence: *tenant / keyspace / txn-group identifiers are first-class types threaded through the whole
stack* (see the `common` crate, §11), and the default path (`default` tenant/group) degrades gracefully to a
simple single-tenant system.

---

## 2. Influences (and what we take from each)

kv9 is a synthesis of four well-understood systems. We cite them where a specific mechanism is borrowed. All are
public; kv9's own design rationale is stated directly.

| System | Paper | What kv9 takes |
|--------|-------|----------------|
| **Bigtable** | Chang et al., OSDI 2006 | Tablets = **regions** (range shards). SSTable = LSM on-disk format. The tablet-location **3-level hierarchy** (root → METADATA → user), which kv9 adapts for self-hosted metadata. Master election → **embedded Raft** (no Chubby). |
| **Spanner** | Corbett et al., OSDI 2012 | **One consensus group per shard** (Paxos → Raft). **2PC across groups** for multi-region transactions, one participant as coordinator. Directory/placement → keyspace placement. TrueTime → a discussed timestamp option. |
| **DynamoDB (2022)** | Elhemali et al., USENIX ATC 2022 | **Split by consumed throughput**, split key chosen from observed access (not the midpoint). **Global Admission Control** (token buckets) for predictable multi-tenant capacity. **Leader leases** + **gray-failure suspicion-before-failover**. **Metadata as a first-class service (MemDS)** with *always-consult + stable load* to avoid bimodal cold-start storms. Log-based durability + **continuous verification**. |
| **TiKV** | open source (github.com/tikv/tikv) | Multi-raft; Region epoch; Percolator 2PC (`default/lock/write` CFs); the **API v2 keyspace key-prefix encoding**; split/merge apply semantics; the raft log serving as the memtable WAL. kv9 adopts these and *improves on* two well-known limits of TiKV's centralized placement driver (metadata held in one process's memory; scheduling scored by local-disk capacity) — see §5, §10. |

**The novel synthesis:** Spanner/TiKV keep metadata in a placement driver; DynamoDB keeps it in an internal
service. kv9 removes the separate control plane entirely: **metadata is just data living in regions of a reserved
*system keyspace*, replicated by the same Raft as user data, and the cluster bootstraps itself.** That is what
makes kv9 a true single binary.

---

## 3. Core concepts & data model

### 3.1 Tenant
An isolation and accounting boundary. Owns keyspaces. Capacity (read/write units), QoS, and blast radius are scoped
per tenant. A tenant never sees another tenant's keys. `default` tenant exists out of the box.

### 3.2 Keyspace
The namespace unit, declared once with immutable core attributes:

```
CREATE KEYSPACE <name> WITH tenant = <tenant_id>, api = txn | raw [, txn_group = <g>, config...]
```

- `api = txn`  → MVCC + Percolator 2PC + Snapshot Isolation. The engine manages versions; keys carry no user ts.
- `api = raw`  → direct KV, optional TTL, optional causal timestamps. No transactions.

A keyspace maps to a numeric `keyspace_id` and a **contiguous key range** in the global keyspace via a prefix
encoding (§3.4). All of a keyspace's regions live within that range.

### 3.3 Region (range shard)
The unit of replication, placement, and scaling. A region owns a half-open key range `[start, end)` and is a
**Raft group** with R replicas (default 3) on R different nodes. Regions **split** and **merge**. Every key belongs
to exactly one region. **Regions never span keyspace boundaries** (enforced in split logic) — this is what keeps a
tenant's blast radius contained and lets keyspace-id be derived unambiguously from a region's start key.

### 3.4 Key encoding (multi-tenant physical isolation)
Adopted from TiKV's API v2 and hardened. Every stored key is prefixed:

```
mode_byte (1)  keyspace_id (3 bytes, big-endian)  user_key...
mode_byte ∈ { 't' = txn, 'r' = raw, 's' = system }
```

- 3-byte keyspace id ⇒ up to 2^24 keyspaces. kv9 **validates** the id range at encode time (a prefix scheme that
  silently truncates an out-of-range id would misroute keys across tenants).
- The prefix makes keyspaces contiguous ranges, so region routing, GC, backup, and encryption all key off the same
  prefix. **Region split points are constrained never to cross a keyspace boundary.**
- `txn` keys carry an internal MVCC suffix (`user_key + inverted(commit_ts)`) in the write CF.

### 3.5 Node / Store
One `kv9` process = one node, simultaneously: (1) a **Store** hosting region replicas (engine + Raft state),
(2) a **member** of the system-keyspace Raft groups (candidate MetaLeader), (3) a **router**. One binary, one node
type; roles are behaviors, not deployables.

### 3.6 Txn group (transaction domain = timestamp shard)
The transaction model is **identical to TiKV's** (Percolator 2PC, Snapshot Isolation, MVCC — §9). kv9 adds a
first-class knob to make the timestamp path scale per tenant: the **txn group**.

- A **txn group** is a transaction/consistency domain. Every `txn` keyspace belongs to exactly one txn group; the
  default is `default`.
- **Invariant: a single transaction never crosses a txn group.** Cross-group transactions are rejected. (A txn may
  still span multiple regions and keyspaces *within one group* — full TiKV semantics there.)
- Because no transaction spans two groups, each group's transactions need ordering only among themselves — so **each
  txn group gets its own independent, sharded TSO timeline** (§8). No cross-group timestamp comparison ever happens,
  which is what removes the single-global-TSO bottleneck.

Opt-in trade: stay in `default` for one global-ish timestamp domain (simple, like TiKV today); declare txn groups to
shard the TSO and scale commit throughput, at the cost of not transacting across a group boundary. Tenants pick the
scheme matching their access pattern (per-tenant, per-shard-key, per-service).

---

## 4. Architecture overview

Two planes, one binary (see `docs/ARCHITECTURE.md` §1–§2 for the box diagrams):
- **Data plane:** user regions (Raft groups) + local engines. Scales out linearly with nodes/regions.
- **Metadata plane:** the system keyspace's regions — themselves Raft groups — plus one elected **MetaLeader** that
  performs placement, split/merge decisions, and hosts the timestamp-oracle providers.

Clients reach any node; the router resolves keyspace→region (epoch-checked), validates the API type, and forwards to
the region leader (or serves from a cached routing table).

---

## 5. Self-hosted metadata (no placement driver)

Instead of a separate placement driver, metadata is data in a reserved **system keyspace** (`keyspace_id = 0`,
mode `'s'`).

### 5.1 What lives in the system keyspace
Stored as ordinary KV under the system prefix, replicated by ordinary Raft:
- **Membership:** node id → address, state, heartbeat, capacity units.
- **Keyspace catalog:** keyspace_id → {name, tenant, api_type, txn_group, config, key range}.
- **Region routing table:** region_id → {range, epoch, peers, leader hint} + a `key → region` range index.
- **Placement/scheduler state:** rebalance queue, split/merge tasks, operator log.
- **Timestamp-oracle state:** per-txn-group persisted timestamp windows (§8).

### 5.1.1 Metadata is *layered* and scales like data (the key design point)
A single in-memory metadata table does not scale — a well-known limit of a centralized placement driver (e.g.,
TiKV's PD holds all region metadata in one process's memory, which bounds cluster-wide metadata throughput to one
node and strains memory at very large region counts).

kv9 refuses that. Metadata is **multi-level**, and the bulk of it is **sharded and self-hosted** (Bigtable's 3-level
tablet-location hierarchy, adapted — see `docs/ARCHITECTURE.md` §3):

- **L0 Root** — a tiny, fixed, well-known Raft group (`META_REGION_0`). NEVER grows. Holds only the *locations of the
  L1 meta-regions* ("meta of meta"), the membership root, TSO windows, and the MetaLeader lease. Always in memory.
- **L1 Meta-regions** — the region routing table + keyspace catalog + placement state, stored as **ordinary sharded
  KV in the system keyspace that split/merge like user data** and spread across nodes. As the cluster grows to
  millions of regions, routing metadata throughput and memory scale **horizontally**.
- **L2 User regions** — tenant data.

**After self-bootstrap, kv9 stores kv9's own metadata (region distribution, catalog, placement) inside kv9's own
sharded regions.** Lookup walks `L0 → L1 → L2` with per-level client caching; the root is near-static so nearly all
lookups hit L1/L2 cache. The bootstrap election (§5.2) only creates the bounded L0; L1 grows on its own. Metadata
never becomes a single-node bottleneck.

### 5.2 Bootstrap — *elect first, then the winner initializes*
A node that joins an *uninitialized* cluster does not assume a pre-assigned role; the joining nodes first **elect the
metadata server**, and the winner performs metadata initialization and a simple self-bootstrap. (State machine:
`docs/ARCHITECTURE.md` §4.)

- `META_REGION_0` has a **fixed, well-known region id** and covers the system key range. Its initial member set is
  the join-set, so no routing lookup is needed to form it.
- **BootstrapElection = Raft leader election over the (empty) `META_REGION_0` log** — the same Raft that replicates
  data. The winner is the metadata server.
- The winner **initializes metadata** as the first Raft-committed entries: create the system keyspace, the default
  tenant, the `META_REGION_0` record (L0), the first L1 meta-region, and the default TSO window. Because these are
  ordinary committed entries, init is crash-safe and idempotent (a crashed initializer re-elects and continues).
- Non-winners wait for the catalog, then **register into membership**. Nodes that join *later* skip election: they
  find the cluster initialized, learn `META_REGION_0`'s members from any peer, and register. Data-driven thereafter.

### 5.3 MetaLeader election
The **MetaLeader** = the Raft leader of `META_REGION_0` (extended to a coordinator that owns the scheduler singleton
via a lease). Election is plain Raft — **no external lock service**. Availability discipline (from DynamoDB):
- **Leader lease:** a new MetaLeader acts only after the previous lease is known-expired (conservative clock bound),
  preventing split-brain during failover.
- **Gray-failure handling:** a follower that suspects the leader **asks a quorum before forcing an election**, so a
  one-way network glitch does not cause needless failover.

### 5.4 Metadata scaling & the cold-start trap (MemDS discipline)
The metadata keyspace can itself split into more L1 regions as the cluster grows, so metadata scales like data.
Routers **cache** the routing table but, following DynamoDB MemDS:
- serve from cache but **refresh on a steady background cadence** (not only on miss), so a cold router or a metadata
  leader change does not trigger a synchronized miss-storm;
- **refuse to serve routing before the cache reaches a freshness watermark** (a router must not open its serving
  surface with an empty/stale routing view), then keep it warm.

---

## 6. Data plane: region, raft, engine

### 6.1 Region = Raft group
Each region runs an independent Raft group (multi-raft), driven by a per-node batch scheduler. Region metadata
carries an **epoch** `(conf_ver, version)`; every request is epoch-checked, so stale-routed requests are rejected and
retried after a routing refresh (TiKV semantics).

### 6.2 Storage engine abstraction
`Engine` is a trait; each region owns a logical LSM keyed within its range. v0 ships `MemEngine` (in-memory BTree);
a real `LsmEngine` (RocksDB via thin FFI, or a native Rust LSM) with `default/lock/write` CFs for txn keyspaces is
planned. As in TiKV, the **raft log is the WAL** for the memtable: a committed raft entry is applied into the memtable;
a memtable flush produces an SSTable and advances a persisted data watermark that bounds raft-log truncation. kv9
keeps this coupling explicit and **adds backpressure** so a slow flush/storage layer throttles ingestion *before* the
log backs up.

### 6.3 Durability & verification (DynamoDB)
- Raft replication across R nodes is the primary durability mechanism.
- A background **scrubber** periodically compares replicas' committed state (range checksums) to catch silent
  divergence ("continuous verification"); divergence triggers re-snapshot from the leader.
- Peer-bootstrap snapshots ship a **file/metadata manifest** (not bytes) once an `LsmEngine` with shared file
  references exists (future); `MemEngine` ships the range.

### 6.4 Raft log vs. WAL stream — a two-layer log
The single most important write-path clarification: the **raft log** and the **WAL stream** are *two layers of the
same data*, not two copies.

- **Raft log** — a **logical, per-region, indexed** entry sequence (`[term, index] → entry`). It is what the Raft
  *protocol* operates on: append, commit-index, replicate to followers, apply, truncate, snapshot. Every region
  (including system-keyspace/meta regions) has its own.
- **WAL stream** — a **physical, append-only, fsync'd** stream on local disk where durability actually happens. It is
  **shared by many regions**, and a node runs a **pool of K** of them.
- **Relationship:** the WAL stream is the durable **write ingress**; the per-region raft log is a read-optimized
  **view materialized on top of it**. Many regions' appends *multiplex* into one WAL stream (group commit); a
  background worker *demultiplexes* the stream into per-region log segments + index for protocol reads. There is no
  separate physical "raft-log stream", and no third log — the LSM memtable has no WAL of its own; **the raft log is
  the memtable's WAL** (applied entries replay on restart).

```
   WRITE (shared+sharded ingress)                     READ / PROTOCOL (per-region materialized view)
   R1,R4 ─┐                                            R1 log [i..j]  append / commit / apply / truncate
   R2,R5 ─┼─▶ WAL stream k ─ fsync ─▶ disk ─ demux ─▶  R4 log [i..j]  (fast, indexed, per-region —
   R3,R6 ─┘   (group commit)          (background)     R2 log [i..j]   never a scan of the interleaved WAL)
```

**Why this shape (performance):** *share* → group-commit amortizes the expensive fsync across many regions (never
one-WAL-per-region = an fsync storm); *shard* into **K** independent streams (default one per high-IOPS volume) →
parallel fsync lifts the single-writer ceiling; the per-region log is materialized from already-durable WAL bytes, so
no double-fsync on the hot path. The knob is **K**: `K=1` = max amortization / min parallelism; `K=#regions` = fsync
storm. Sweet spot: **K ≈ number of independent IO devices**, regions hashed across them (K ≪ #regions but K > 1).

**Safety (durability + correctness):**
- The **durability boundary is WAL fsync + raft majority** — nothing commits before that; lazy materialization is
  safe because the WAL is already durable.
- **Crash-consistent demux:** on restart, replay each stream's *uncompacted tail* to rebuild in-memory per-region
  indices; a region's entries must be exactly reconstructable despite being interleaved.
- **Stable region→stream mapping is an invariant** (deterministic or recorded) so recovery knows which stream holds a
  region's entries. **Moving a region between streams needs a checkpoint/barrier** (drain past a point on the old
  stream before writing the new), else entries straddle two streams.
- **Shared lifecycle coupling:** a WAL segment recycles only after **all** regions sharing it have truncated past it
  → one slow region can pin a segment for its stream-mates (the backpressure point guarded by §6.2). Fewer regions
  per stream = weaker coupling.

**Global scalability & multi-tenancy:** the per-region raft log is never a global bottleneck (each region is an
independent group, spread across nodes by sharding); per-node throughput scales with K; **global write throughput ≈
nodes × streams_per_node** — two orthogonal axes (region distribution across nodes; WAL sharding within a node).
Metadata regions use the same machinery, so metadata writes scale too. For **blast-radius isolation**, stream
assignment may be **aligned to tenant/keyspace tiers** (a tenant's regions on their own stream[s]), trading some
fsync amortization for containment — the WAL pool is itself a multi-tenancy control surface.

---

## 7. Serving & per-tenant QoS
The serving threads (read/coprocessor pools) are **tenant-fair**: work is scheduled so one keyspace cannot starve
another's CPU, and per-tenant/keyspace CPU-and-request budgets bound a tenant's footprint. This complements the
capacity admission control in §10 (which bounds *how much* a tenant may do) with fairness (which bounds *when* they get
scheduled). Backpressure (§6.2) and overload protection shed load predictably rather than collapsing.

---

## 8. Time & consistency

`txn` keyspaces need Snapshot Isolation, which needs a timestamp order. kv9's key move: this order is **per txn group,
not global** — which is what makes the timestamp path scale.

### 8.1 Sharded TSO — a pool of providers serving many keyspaces / txn groups
Because a transaction never crosses a txn group (§3.6), each group needs timestamps ordered only within itself. So
kv9 runs a **pool of TSO providers** (diagram: `docs/ARCHITECTURE.md` §5). Mapping (all metadata in the system
keyspace):

```
   keyspace ──N:1──▶ txn group ──1:1──▶ TSO timeline ──N:1──▶ TSO provider (pool member)
```

- A **cluster has many TSO providers, not one.** Each provider **hosts one or more TSO timelines** and serves the
  keyspaces / txn groups assigned to those timelines — a single provider serves *different* keyspaces/groups at once.
- A **txn group owns exactly one timeline** (its own monotonic clock + persisted window). Keyspaces map N:1 onto txn
  groups. The `default` group is one timeline and behaves like a single classic TSO.
- **Assignment is data, and rebalanceable:** the metadata plane assigns each group's timeline to a provider and can
  move it — a hot group gets its own provider; many cold groups share one. Adding providers spreads oracle load, so
  commit-timestamp throughput scales horizontally.
- Each provider is **elected and lease-held** via the metadata plane; its timelines recover from the system keyspace
  on failover. Standard TSO anti-regression rules apply **per timeline** (never hand out ≤ the persisted bound; on
  provider failover start above the persisted window; refuse to serve until the new lease is confirmed; a
  clock-regression monitor guards monotonicity). The keyspace→group→provider mapping has a **single authoritative
  source** (no stale secondary copy) and per-timeline state (no single global lock).

This is TiKV/PD's keyspace-group TSO generalized into a user-facing concept and folded into the single binary.

### 8.2 Timestamp tiers — pluggable, with Spanner as the top tier
The default (sharded TSO per group, no cross-group txn) is the scalability-optimal, clock-agnostic choice for the
common case. But some workloads need cross-group / global transactions. Spanner shows how to get a **global timeline
without a central allocator** — from synchronized clocks + commit-wait — so kv9 makes the timestamp source a
**pluggable tier** behind a TrueTime-shaped `TimeSource: now() → [earliest, latest]`:

1. **Tier 1 — sharded embedded TSO (default).** Per-group timeline from a persisted window served in memory. *No
   clock assumptions*, lowest latency, best scale; transactions are **confined to a txn group** (§3.6). Returns a
   point (ε=0) valid only within its group.
2. **Tier 2 — HLC + bounded max-offset (CockroachDB-style).** Hybrid logical clocks + an assumed max clock offset,
   with uncertainty intervals and read-restart. Enables **cross-group serializable** transactions with *no special
   hardware*; safety rests on the clock-offset bound; occasional restarts. No central allocator.
3. **Tier 3 — TrueTime + commit-wait (Spanner).** With a bounded-ε clock source (GPS/atomic, or a *measured*-bound
   PTP/NTP): pick commit ts `s ≥ now().latest`, then **commit-wait** until `now().earliest > s` (~2ε) before
   releasing locks ⇒ **cross-group external consistency**. Cost: ~2ε commit latency.

The key point: **"no cross-group transaction" is a Tier-1 restriction, not a kv9 axiom.** A tenant/group that needs
global transactions opts into Tier 2/3 and accepts the clock requirement + commit-wait; everyone else stays on the
scalable default. Spanner's within-group mechanisms — leader leases (§5.3) and 2PC participant-coordinator (§9.1) —
kv9 already uses.

v0 skeleton defines a per-group `TimestampOracle`/`TimeSource` trait with an `EmbeddedTso` (Tier 1) stub and a
`TxnGroupId` on the handle; the interval-returning shape leaves room for Tiers 2/3.

### 8.3 Reads — adopt Spanner's read side now (tier-independent)
These are pure read-scalability wins and need no special clocks (within a group's timeline); kv9 adopts them from
the start:
- **Lock-free consistent snapshot reads.** Read-only transactions take **no locks**; they read a snapshot at a
  timestamp.
- **Safe-time follower reads.** Each replica tracks a **safe-time** = the max timestamp at which it has applied all
  writes with no pending prepare below it. A follower may serve a read at `ts` once `safe_time ≥ ts` ⇒ read
  throughput scales with **replicas**, and a client reads the **nearest** replica (latency), offloading leaders.
- **Bounded-staleness reads.** Read at `now − δ` for cheaper follower reads when slight staleness is acceptable.

---

## 9. Transactions & the raw path

### 9.1 Txn keyspaces — Percolator 2PC over MVCC (TiKV model)
- `start_ts` from the group's oracle; snapshot reads see versions ≤ start_ts.
- **Prewrite** locks the primary then secondaries (intents in the `lock` CF, data in `default`).
- **Commit** takes `commit_ts`, commits the primary (atomic point) then secondaries lazily (`lock`→`write`).
- **Cross-region** transactions **within one txn group**: 2PC where one region's primary lock is the atomic commit
  point (Spanner's participant-coordinator). ResolveLock cleans up on failure. A txn whose keys resolve to two
  different txn groups is **rejected at begin** (§3.6) — the confinement that makes per-group TSO timelines correct.
- **Deadlock detection is keyspace-aware:** the wait-for graph is partitioned per tenant. (TiKV's detector is a
  single global graph; kv9 partitions it so tenants are isolated and the detector scales.)

### 9.2 Raw keyspaces — direct KV
`RawPut/RawGet/RawDelete/RawScan/RawBatchGet`, optional TTL, optional causal timestamps for ordering without full
transactions. No locks, no 2PC. The keyspace's `api_type` selects the executor at routing; a keyspace cannot mix.

---

## 10. Sharding & multi-tenant capacity (throughput-first)

Global throughput scaling (goal #4) and capacity isolation (goal #1) meet here.

- **Pre-sharding at keyspace creation (DynamoDB-style).** Reactive split is not enough for a keyspace *known* to be
  hot — starting as one region and waiting for splits is a cold-start bottleneck. So a keyspace may be **pre-split**
  up front: `CREATE KEYSPACE … WITH pre_split = { by: hash|range, shards: N | split_keys: [...] }`. The MetaLeader
  creates N regions covering the keyspace range at creation and scatters them across nodes. A declared **shard-prefix
  function** (e.g. leading hash byte of the user key) distributes writes across the pre-created regions from t=0 —
  the DynamoDB write-sharding pattern for sequential / low-cardinality keys. Pre-shard for known load; reactive split
  (below) for emergent hotspots.
- **Split triggers on consumed throughput/CPU, not only size** (DynamoDB). A hot region is a split candidate even if
  small; the **split key is chosen from the observed access distribution** so both halves shed load. A region hot on
  a single key is flagged (splitting can't help) and handled by capacity/adaptive routing.
- **Merge** low-traffic adjacent regions within a keyspace to reclaim raft overhead.
- **Placement / rebalance** is a MetaLeader responsibility and is **consumption-aware from day one** (WCU/RCU/CPU/
  region-count), *not* local-disk-capacity-first. (A centralized driver that scores by disk capacity and treats every
  peer move as an equal fixed cost — as TiKV/PD does — misjudges placement in a scale-out world; kv9 scores by real
  consumption from the start.)
- **Global Admission Control (per-tenant / per-keyspace):** capacity is enforced with token buckets handed out by the
  metadata plane (DynamoDB GAC). This is the capacity axis of §1.1 — multi-tenant throughput stays predictable and no
  tenant starves another. Combined with per-tenant fair scheduling (§7), it gives both *how much* and *when*.

---

## 11. API surface (v0)

Transport is gRPC, but the skeleton defines the surface as Rust traits so it compiles without a protoc toolchain; a
thin wire adapter is added next.

- **Txn:** `KvGet, KvBatchGet, KvScan, KvPrewrite, KvCommit, KvPessimisticLock, KvPessimisticRollback,
  KvResolveLock, KvCleanup, KvCheckTxnStatus`.
- **Raw:** `RawGet, RawBatchGet, RawPut, RawBatchPut, RawDelete, RawScan, RawDeleteRange`.
- **Admin/meta:** `CreateKeyspace, ListKeyspaces, GetRegion (routing), SplitRegion, ClusterInfo`.
- **Auth is in scope:** admin/meta and any destructive endpoint authenticate and authorize the caller from day one
  (not network/mTLS alone).

Every data request carries `(keyspace_id, region_epoch)`; the router resolves keyspace→region and checks the API type.

---

## 12. Crate layout (what the skeleton implements)

A Cargo **workspace**. Boundaries mirror this document. Dependencies are kept minimal and **pure-Rust / protoc-free**
for v0 (heavy native build deps — protoc/cmake/grpc/rocksdb — are painful and slow; consensus/LSM/wire crates are
introduced behind traits and added incrementally).

```
kv9/
├── DESIGN.md                     ← this document          docs/ARCHITECTURE.md ← diagrams
├── Cargo.toml                    ← workspace
├── crates/
│   ├── common/   ← ids (Node/Region/Keyspace/Tenant/TxnGroup), Keyspace/Tenant/ApiType, key codec,
│   │               TimeStamp/HLC, errors, config          (multi-tenant types are here, §1.1/§3)
│   ├── meta/     ← membership, keyspace catalog, region routing, placement/scheduler, multi-level
│   │               metadata (L0/L1), election-first Bootstrap FSM, MetaLeader, TSO provider pool
│   ├── engine/   ← Engine trait + MemEngine; MVCC layout (default/lock/write); WriteBatch
│   ├── raft/     ← RaftGroup trait + single-node stub
│   ├── region/   ← Region, RegionRouter, epoch, split/merge (throughput-aware), WalStream/WalPool (§6.4)
│   ├── txn/      ← Percolator 2PC (txn keyspaces) + txn-group confinement; raw executor
│   └── server/   ← Node assembly, API traits (Txn/Raw/Admin/Router), request routing
└── src/ (bin `kv9`) ← main.rs: single binary; CLI (--join, --data-dir, --addr, --txn-groups)
```

`cargo check --workspace` passing is the milestone's definition of done. Method bodies may be `unimplemented!()`
/typed errors, but types, traits, and module boundaries are real and reflect this design.

---

## 13. Design principles distilled
The choices above, as standalone principles:
1. **No separate control plane.** Metadata is self-hosted, layered, and sharded — it scales like data and never
   becomes a single-node bottleneck.
2. **Tenant-first everywhere.** Namespace, capacity, QoS, timestamp order, blast radius, and billing are all
   per-tenant; identifiers are first-class through the stack.
3. **Region boundaries align to keyspace boundaries** — an invariant, so keyspace-id derives unambiguously from a
   region's start key and a tenant's blast radius is contained.
4. **Validate encoded ids** (never silently truncate a keyspace id) — misrouting across tenants must be impossible.
5. **Shard the hot serialization points.** TSO (per txn group) and the WAL (a pool of streams) are sharded so
   per-node and per-cluster throughput are not pinned to one writer/allocator.
6. **Backpressure, not collapse.** Storage/flush slowness throttles ingestion before the log backs up.
7. **Consumption-aware placement + split-by-throughput**, not disk-capacity-and-size-first.
8. **Warm, steadily-refreshed metadata caches** that never serve before a freshness watermark (no bimodal cold start).
9. **Auth on the control/management plane from day one.**
10. **Keyspace-aware deadlock detection** (per-tenant isolation and scale).

---

## 14. Future directions (documented, not in v0)
- **Shared-storage (disaggregated) backend:** an `Engine`/DFS backend persisting SSTables to object storage so regions
  rebalance by shipping file *references* (meta-only snapshots). Caveats to design for: object-store backpressure,
  cold-cache tails, per-provider integrity checks.
- **Coprocessor / pushdown**, **CDC / change feeds**, **PITR / native backup** — layers above the region.
- **HLC / TrueTime** timestamp sources behind `TimeSource`.

---

## 15. Milestones
- **M0 (this milestone):** DESIGN.md + `docs/ARCHITECTURE.md` + compilable workspace skeleton with real module
  boundaries. ✅ target
- **M1:** Single-node runnable: create keyspace (txn/raw), MemEngine, raw + txn happy-path, embedded TSO stub,
  in-process router. `kv9` boots and serves `RawPut/RawGet` and a `txn` Get/Prewrite/Commit.
- **M2:** Real Raft (one group), persistence (LsmEngine), system-keyspace bootstrap with seed nodes.
- **M3:** Multi-region: routing table in system keyspace, split/merge (throughput-aware), rebalance.
- **M4:** Multi-node clustering, MetaLeader election + lease, membership join/leave; sharded WAL pool; sharded TSO.
- **M5:** Cross-region 2PC, keyspace-aware deadlock detection, GAC token buckets + per-tenant fair scheduling, scrubber.
```
