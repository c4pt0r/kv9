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
4. **Object storage is THE source of truth (storage-compute disaggregation).** All durable state lives in object
   storage (S3/GCS/Azure); **compute holds no authoritative data** — local NVMe is only cache + a *transient,
   raft-replicated WAL* staging the not-yet-flushed tail. Stateless compute is what makes nodes elastic (add/kill in
   seconds, scale idle tenants to zero, instant failover) and rebalancing near-free (regions move by shipping *file
   references*, not bytes). Core, not optional — see §6.5.
5. **Horizontal throughput scaling** via **range-sharded regions** with **split/merge**, where **split is driven
   by consumed throughput, not just size** (DynamoDB 2022).
6. **Familiar API.** The common TiKV surface: transactional and raw.
7. **Correctness first.** Snapshot Isolation for `txn` keyspaces via Percolator-style 2PC over a monotonically
   ordered timestamp; a log-backed WAL; continuous replica verification.

### Non-goals (for now)
- SQL, coprocessor push-down, secondary indexes (kv9 is the storage engine; a SQL layer is out of scope).
- Cross-tenant / cross-group external consistency on commodity clocks at Spanner's TrueTime level (§8).

### 1.1 Multi-tenancy is the organizing principle
A tenant owns keyspaces; a keyspace has an API type and belongs to a txn group. **Isolation is enforced on every
axis**, and each axis has a concrete mechanism:

| Isolation axis | Mechanism in kv9 | Section |
|----------------|------------------|---------|
| **Namespace** (no tenant sees another's keys) | keyspace-id key prefix; every region lives inside one keyspace's range | §3.4, §4 |
| **Blast radius** (a tenant's failure/mistake stays local) | region boundaries **must align to keyspace boundaries**; per-keyspace GC / encryption / backup | §3.3, §10 |
| **Capacity** (no noisy neighbor) | **Global Admission Control**: per-tenant / per-keyspace token buckets handed out by the metadata plane | §10 |
| **Performance / QoS** | per-keyspace fair scheduling of CPU on the serving threads; per-tenant request budgets | §7 |
| **Cache** (dominates latency under disaggregation) | per-tenant **cache-fill tokens** so a scan can't evict a neighbor's hot set | §6.5, §7.1 |
| **Backpressure fairness** | shared pipeline tokens are allocated by a per-tenant weighted fair queue | §7 |
| **Timestamp / commit throughput** | **sharded TSO**: per-txn-group timelines served by a pool of providers | §3.6, §8 |
| **Correctness under contention** | **keyspace-aware deadlock detection** (per-tenant wait-for graphs) | §9 |
| **Physical data isolation** | per-keyspace/tenant object-storage prefixes (opt. per-tenant buckets) + per-tenant object encryption (CMEK) | §6.5 |
| **Accounting** | keyspace = the billing/metering unit (incl. object-storage usage) | §6.5, §10 |

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
- `api = raw`  → direct KV, optional TTL, optional causal timestamps. **No transactions.**

**The keyspace is the absolute transaction boundary.** A `txn` keyspace supports Snapshot-Isolation transactions
*within itself only* — a transaction **never** crosses a keyspace (not even within one tenant); a `raw` keyspace
supports none. Keyspace > txn group: a keyspace *contains* txn groups (§3.6), which merely shard its TSO.

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

### 3.6 Txn group (a TSO shard *inside* a keyspace)
The transaction model is Percolator 2PC / Snapshot Isolation / MVCC (§9). The concept hierarchy is
**tenant → keyspace → txn group → timeline**, and the keyspace is the hard boundary:

- **The keyspace is the absolute transaction boundary — a transaction NEVER crosses a keyspace** (not even within one
  tenant). Each keyspace is an independent transaction/consistency domain; this is a hard multi-tenant isolation
  guarantee.
- **Only a `txn` keyspace has txn groups.** A `raw` keyspace supports **no transactions at all** (§3.2) — it has no
  txn group and no TSO timeline (at most a per-key causal timestamp). The txn group is strictly a sub-concept of a
  `txn` keyspace, which is another reason the keyspace is the larger concept.
- A `txn` keyspace contains **one or more txn groups** (default: exactly one). A **txn group** is a *subdivision of a
  txn keyspace's transaction domain that shards its TSO* — each txn group owns its own timeline (§8), covering a
  sub-range of the keyspace. A transaction is confined to one txn group, hence to one keyspace.
- **Default:** a txn keyspace = one txn group = one timeline → transactions range freely over the whole keyspace with
  full Snapshot Isolation (the common case). **Opt-in:** a single very hot keyspace may shard into several txn groups
  (e.g. by user-id range) to scale commit-timestamp throughput, at the cost of not transacting across those sub-groups.

Two nested confinements, the outer one absolute:
- **cross-keyspace transaction → always rejected** (the hard boundary — never relaxed by any timestamp tier);
- **cross-txn-group transaction *within* a keyspace → rejected** (only relevant if the keyspace opted into
  subdivision; this is what lets each group own an independent timeline with no cross-group timestamp comparison).

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

### 5.0 The metadata catalog is a small embedded SQL engine
> Concrete simple design: [`docs/METADATA-CATALOG.md`](METADATA-CATALOG.md).

Metadata is inherently **relational** (tenants→keyspaces→txn-groups→timelines; regions↔peers↔nodes; files↔refcounts).
kv9 models it as **relational tables managed by a small embedded SQL engine that runs on kv9's own transactional KV**
— the system keyspace (its own txn group `system`, §3.6). This is deliberate dogfooding: kv9 manages its own metadata
through its own MVCC+2PC, with a thin relational layer on top; no external etcd, no bespoke key encoding, no
hand-rolled indexes.

Why relational, not ad-hoc KV records: it makes a whole class of control-plane bugs **structurally impossible** —
auto-maintained secondary indexes rolled back atomically within the transaction (no orphaned `name→id` index on a
failed create); one table + FK/join instead of a duplicated field (no stale dual-source `keyspace→group` mapping);
multi-table mutations that are atomic + idempotent transactions (principle 15); a **versioned schema + migrations**
(forward-compat, principle 12); typed FK/unique/check constraints that make illegal states unrepresentable
(principle 16); and a **queryable** surface for the scheduler (`SELECT … JOIN … WHERE load > x` instead of hand-rolled
in-memory trees), for observability, and for admin/debug.

**Scope — small on purpose:** a *fixed, versioned* schema (not user DDL); PK + a few secondary indexes; single-table
scans + simple joins; parametrized/prepared queries (internally a typed query API; SQL *text* mainly for
admin/tools/debug). No cost-based optimizer or general planner — the query set is known and tiny. Transactions reuse
the `system` group's MVCC+2PC.

**Illustrative schema:** `tenants`, `keyspaces`(idx name, tenant), `txn_groups`, `tso_timelines`, `nodes`
(membership), `regions`(idx keyspace, range), `region_peers`(idx node → "regions on node"), `sst_files`
(the GC refcount table, §6.5), `placement_rules`, `tasks`, `gac_allotments`, `schema_version`.

**Bootstrap — self-describing catalog, no circularity:** the core catalog tables' schemas are **hardcoded** (à la
Postgres `pg_catalog` / CockroachDB system descriptors), anchored in **L0** (the bounded bootstrap region, which is
plain KV, not SQL). L0 locates the L1 metadata regions; the L1 regions hold the SQL tables (including the *user-region*
routing table). The SQL engine operates at L1, anchored by the fixed L0 — the bottom turtle isn't SQL, so no infinite
regress. Because the tables live in L1 regions, the catalog **shards and scales** like everything else (§5.1.1).

### 5.1 What lives in the system keyspace
The metadata tables (§5.0), each ultimately rows in the system keyspace, replicated by ordinary Raft:
- **Membership:** node id → address, state, heartbeat, capacity units.
- **Keyspace catalog:** keyspace_id → {name, tenant, api_type, txn_group, config, key range}.
- **Region routing table:** region_id → {range, epoch, peers, leader hint} + a `key → region` range index.
- **Placement/scheduler state:** rebalance queue, split/merge tasks, operator log.
- **Timestamp-oracle state:** per-txn-group persisted timestamp windows (§8).
- **SST reference counts:** per-file refcount for object-storage GC (§6.5). The **manifest is authoritative**; the
  refcount is a **conservative upper bound** maintained by ordering, not by cross-group atomicity (none exists
  between a region's raft state and the system keyspace): **+ref commits *before* the manifest change that
  references the file; −ref only *after* the manifest change that drops it.** Hence `refcount = 0` is at any moment
  a sufficient condition for safe deletion. Crashes only leak over-counts (never endanger data); a
  manifest-verifying leak scan revises counts **downward only**.

The **system keyspace is its own txn group (`system`)** with its own timeline; metadata operations that span multiple
meta-regions (e.g., a split updates the parent region *and* the routing L1 region) use **2PC within `system`** for
atomicity. **Meta regions (L0/L1) run a higher replication factor (R=5)** and flush L0 aggressively, so the root of
all metadata survives 2 seed failures even before its first object-storage flush.

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
- **Fencing — bootstrap must be unforkable:** (a) *unreachable ≠ uninitialized* — a node enters BootstrapElection
  only on a positive "uninitialized" answer from a **quorum of its declared seed set**, never on silence or
  timeouts; (b) the election counts votes only within the declared seed set and requires a **majority of that
  set**, so two disjoint seed lists cannot both initialize; (c) initialization is once-per-lifetime — a node whose
  data-dir carries an initialized marker (or non-empty raft state) refuses to re-initialize and rejoins via
  Joining; a wiped node is a *new* node. (raft-rs `initialize()` requires a pristine node, enforcing (c) at the
  library layer; (a)/(b) live in the Discovering/BootstrapElection FSM.)

### 5.3 MetaLeader election & distributed scheduling
The **MetaLeader** = the Raft leader of `META_REGION_0`. Election is plain Raft — **no external lock service**.
Availability discipline (from DynamoDB):
- **Leader lease:** a new MetaLeader acts only after the previous lease is known-expired (conservative clock bound),
  preventing split-brain during failover.
- **Gray-failure handling:** a follower that suspects the leader **asks a quorum before forcing an election**, so a
  one-way network glitch does not cause needless failover.

**Scheduling is not a singleton** (or kv9 would re-inherit the centralized-scheduler ceiling it set out to fix).
Three tiers: (1) **distributed detection** — each node spots its own split/hotspot/merge candidates from local stats
and *proposes* operations, O(local regions); (2) **sharded placement** — scheduling authority for a keyspace range is
held by the **leader of the L1 meta-region owning that range**, so scheduling scales with L1 shards, not one node;
(3) the **MetaLeader arbitrates only cross-shard/global policy** (cluster-wide invariants, capacity), doing O(1) work
per decision, never scanning all regions.

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

**Idle-region quiescing (essential at multi-tenant scale).** A multi-tenant cluster has **huge numbers of mostly-idle
regions** (many keyspaces × pre-shard × split), and per-region raft heartbeat/election ticks are a real background
cost that would otherwise cap how many regions a node can hold. So an idle region **quiesces**: it stops ticking (no
heartbeats/elections) and holds no timers; a write, read, or membership event **wakes** it. A quiesced leader keeps
its lease implicitly (peers don't campaign because they too are quiesced with a valid last-heard time). This lets a
node hold millions of cold regions cheaply — merge (§10) reclaims raft groups where possible, quiescing handles the
rest. (Learned from the cost of *not* having this in comparable engines.)

### 6.2 Storage engine abstraction (disaggregated LSM over object storage)
`Engine` is a trait; each region owns a logical LSM keyed within its range. Two backends: `MemEngine` (in-memory
BTree; skeleton/tests) and the real **disaggregated `LsmEngine`** — memtable + local block cache on NVMe, **SSTs on
object storage** (§6.5), with `default/lock/write` CFs for txn keyspaces. As in TiKV, the **raft log is the WAL** for
the memtable: a committed raft entry is applied into the memtable; a memtable flush builds an SSTable, **uploads it to
object storage**, and advances a persisted data watermark that bounds raft-log truncation. kv9 keeps this coupling
explicit and **adds backpressure** so a slow flush/upload throttles ingestion *before* the log backs up.

*Design rule:* the `Engine` / `ObjectStore` traits are shaped **around the disaggregated model** — manifests,
object-file references, meta-only snapshots, and safe-time reads are first-class in the trait surface. A classic
local-`KvEngine`-shaped abstraction would not fit the disaggregated reality and would rot into dead
`unimplemented!()` methods (a failure mode seen in engines that bolted disaggregation onto a local-engine trait).

### 6.3 Durability & verification (two-tier)
- **Durability = raft-acked ingress → object-storage source of truth.** A write is durable when raft-majority
  accepted (local WAL); it then drains into object storage, the **single source of truth** (§6.5). The local tail is
  *transient staging*, not a second authoritative copy; cold data is never N×-replicated across compute.
- **Peer-bootstrap / rebalance snapshots ship a manifest (file references), not bytes** — the receiver attaches by
  reading the manifest and lazily pulling blocks from object storage. (`MemEngine` ships the range for tests.)
- A background **scrubber** does continuous verification: manifest ↔ object existence, per-object checksums, and
  tail consistency across replicas; divergence triggers re-fetch/re-snapshot.

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
*Caveat:* the fsync-*parallelism* win is device-bound — on a single (often network-attached) disk, K>1 buys
lock-contention relief + pipelined/`io_uring`-batched fsync, not raw throughput; for real parallelism map streams to
**separate volumes**. Benefit = min(K, #independent volumes).

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

### 6.5 Storage-compute disaggregation — object storage is the source of truth
**Object storage (S3/GCS/Azure) is the single source of truth for all durable state. Compute holds no authoritative
data** — only a cache and a *transient, raft-replicated write-ahead log* that continuously drains into the source of
truth. This is a core pillar (goal #4) and the reason kv9 is cloud-native: stateless compute → seconds-scale
elasticity, scale-to-zero, instant failover; independent storage/compute scaling; object storage's durability and
~10× lower cost inherited for free; and backup/PITR/branch/clone become reference operations on the one truth.

**The WAL is ingress, not a competing truth.** Object storage is ~tens-of-ms latency, so writes are not put into the
source of truth synchronously. Instead a write is **acked at raft-majority acceptance** (fast, local WAL) and is
**continuously flushed into object storage**; the local tail is the "acked-but-not-yet-landed, can't-be-lost" staging
buffer, always draining toward the one source of truth. There is no data-loss window (raft majority protects the
in-flight tail until it lands), and there is no second authoritative copy (the tail is transient). *(This is the
Aurora/Neon "log in front of durable storage" shape; §14 can push the log itself into a shared service, but even in
v1 object storage is already the sole source of truth for everything that has landed.)*

**Two invariants everything follows from:**
1. **Immutability boundary.** SSTs are **immutable, write-once objects** on object storage. The only mutable state —
   each region's **manifest** (file-id list + LSM structure) — lives in the **raft-replicated region state, not on
   object storage**. Object storage therefore sees only immutable *creates* and *deletes*; all ordering/mutation is
   in raft. This sidesteps object-store consistency weaknesses (SSTs are never updated in place; the mutable pointer
   is the raft-committed manifest).
2. **Split replication.** **Raft replicates the recent log tail** (unflushed writes — low-latency durability +
   failover); **object storage holds the flushed bulk** with its own durability. kv9 does **not** N×-replicate cold
   data across compute nodes. A "replica" is *log + cache*, not a full data copy. (Diagram: `docs/ARCHITECTURE.md`
   §9.)

**Flush ownership — the leader flushes; followers adopt (this is what keeps the bulk single-copy).** Every replica
applies committed entries into its own memtable (that's how followers stay warm and can become leader), but **only
the leader builds and uploads SSTs.** The flush is a **raft-committed manifest change** (the new file-ids + the range
of log indices it subsumes). When a follower applies that entry, it **drops the corresponding memtable range and
adopts the leader's file references** — it does **not** flush its own copy. So object storage holds **one** set of
objects per region, and a "follower" is materially *log tail + cache + manifest*, not a full data copy. A follower
promoted to leader already has the manifest and resumes flushing from the tail. (This is what makes §6.5's
"no N× cold-data copies" true; without it, R replicas would each upload.)

**Write path handoff:** leader memtable flush → build SSTable → **upload to object storage** → **propose the manifest
change through raft** → on commit, all replicas adopt the file-ids and **the WAL/raft-log tail it subsumes may
truncate**. Upload latency thus gates truncation → the backpressure point (§6.2/§6.4). A committed write is durable
*immediately* via raft-majority WAL (does not wait for object storage); it becomes object-storage-durable at flush.

**Read path:** memtable + local block cache; miss → fetch SST blocks from object storage (tens of ms). Working set
should fit cache. **Any compute node holding a region's manifest can serve its reads** from object storage + cache —
so read-replicas / safe-time follower reads (§8.3) are cheap to spin up, and a cold region's compute can scale toward
zero (data is safe on object storage).

**Elasticity — scale-out is a metadata rearrange, not a data move (the payoff, feeds §10).** Because SSTs are
shared-addressable and immutable and object storage is the single source of truth, moving a region onto a new node
ships only **metadata**: a raft conf-change + routing update, the region's **manifest** (a tiny list of file-ids),
and the small **recent log tail**. The new replica **references the same object-storage files** as the others and
**lazily pulls blocks on demand** into its cache — there is **no node→node bulk copy**. Cost per moved region is
`O(metadata + manifest + log-tail)`, not `O(region data)` — seconds, bandwidth-light (contrast shared-nothing, which
streams GB per region via raft snapshot). Split/merge are likewise meta-only (children reference the parent's
immutable files until compaction rewrites). Two decoupled axes: **compute** scales out by meta-only region attach;
**storage capacity** is a non-event (object storage is elastic — no storage nodes to add).

*The one honest caveat:* a meta-only-attached region is **available immediately** but its cache is **cold** — first
reads miss to object storage (~tens of ms) until the working set warms. So placement is instant; peak performance
ramps. Mitigate by adding the peer as a **learner first** (catch up manifest+tail off the quorum path, warm cache,
then promote to voter) and by prefetching hot blocks. This cold-cache tail — not data transfer — is the real cost of
fast scale-out.

**Object storage engineering (a real latency/rate/cost/availability domain):**
- **Prefix/hash key layout:** map file-id → object key spread across many prefixes so request load doesn't hit a
  single object-store partition's rate limit.
- **Multipart upload** for large SSTs; **read-block granularity** with a local block cache.
- **GC = epoch-anchored mark-and-sweep with a metadata-plane ref table.** Every SST has a **refcount in the system
  keyspace**, maintained under the **conservative two-phase ordering** (+ref-before-use / −ref-after-drop, §5.1):
  a split that shares a straddling file commits its +ref **before** the child manifests reference it; compaction
  commits −ref only **after** the manifest swap dropping the inputs. A file is deletable only when **refcount = 0, no live epoch references it, and a
  grace period ≥ max snapshot/read staleness has elapsed**. Flush/compaction **never delete inline** — they propose
  manifest swaps; a **GC worker** does logical-delete → lifecycle-expiry, with a slow **orphan scan** (objects lacking
  any metadata ref) as a backstop. Idempotent and lag-tolerant.
- **Own the integrity checks:** per-object checksums; do not assume the store validates. Immutable objects make this
  simple (verify once on read/ingest).
- **Compaction = raft-committed manifest swap.** The leader (or an offloaded **stateless worker**) reads inputs /
  writes outputs on object storage, then the leader **proposes `{remove: inputs, add: outputs}` through raft,
  version-checked against the current manifest** (reject if inputs changed under it — optimistic concurrency). Workers
  hold no authority; the raft commit is the only truth. Default to **tiered compaction** (less write-amp ⇒ less
  object-store $, read-amp absorbed by cache).
- **Flush sizing (avoid tiny objects):** flush on `(size ∨ time ∨ memory-pressure ∨ WAL-retention-cap)`, target a
  minimum object size; cold low-write regions accept **longer WAL retention** (they aren't memory-pressured) rather
  than emit tiny objects. This is the WAL-retention ↔ object-efficiency tension, made explicit.
- **Availability (a shared fate to own):** a brief object-store blip does not stop **writes** (local WAL + raft
  majority) but **cold reads degrade**. Mitigations: size the cache to the **hot-set SLO** so an outage hits only cold
  data; an opt-in **per-tenant cross-region/cross-bucket replication tier** (write 2 buckets, read-failover) for
  higher read availability; **read-through-stale** where the API permits. Read availability is a per-tenant *choice*,
  not a global fate.
- **Cost is a scheduling input:** compaction I/O = object-store GET/PUT = money; the placement/compaction policy
  weighs request cost, not just write-amplification.
- Pluggable backends behind an `ObjectStore` trait (S3/GCS/Azure/local-for-tests).

**Multi-tenancy over object storage (strengthens §1.1):** object keys are **per-keyspace/tenant-prefixed** (optionally
per-tenant buckets) for blast-radius isolation and billing; **per-tenant encryption** (CMEK — tenant-scoped keys wrap
their objects); **object-storage usage is a billing/capacity dimension** per tenant.

**Metadata over object storage:** system-keyspace (metadata) regions use the same disaggregated engine, so the
routing table / catalog are durable on object storage and scale as L1 splits. Bootstrap ordering: **object-store
config (endpoint/bucket/credentials) comes from node flags** before any metadata exists; the L0 root is durable via
raft-majority local WAL on the seed nodes until its first flush.

**Decided — the raft log and WAL live on local disk.** A committed write is durable via **local fsync (NVMe) + raft
majority**, low-latency and with **no object-storage dependency on the write path**. Local disk holds exactly two
things: the **sharded WAL / raft-log pool** (§6.4) and the **block cache** for object-storage SSTs; the durable
*bulk* is on object storage. A node loss loses only its cache and its local log tail — which the raft majority on
peer nodes still holds and object storage still holds the flushed bulk. Compute nodes are therefore *nearly*
stateless: the only local durable state is the recent, bounded, raft-replicated log tail.

**The durability point is raft-group acceptance.** The moment a log entry is accepted by a majority (persisted to
their local WALs), the record is *safe* — that is the **sole** durability boundary; the object-storage flush is
asynchronous and off the write-path critical path. **Corollary: per-node local-disk durability is not required for
safety** — safety comes from majority replication, not from any single disk. So a node may lose its entire local
disk and rejoin: it refetches the recent log tail from its peers and the bulk from object storage, with **no data
loss**. (Commodity NVMe is sufficient; no per-node RAID/battery-backed durability is needed for correctness.)

*Evolution (not v1, see §14):* externalize the log to a shared/replicated log service (log-is-the-database) to make
compute fully stateless — a bigger build, deferred.

---

## 7. Flow control, backpressure & QoS — one token system

kv9 unifies three concerns usually built separately — **capacity** (how much a tenant may do), **fairness** (whose
work runs when), and **backpressure** (is the pipeline healthy) — into a **single token-based flow-control system**.
Every unit of work acquires tokens before proceeding; token availability is the *one* signal that governs admission,
fairness, and backpressure together. (Diagram: `docs/ARCHITECTURE.md` §10.)

### 7.1 The token model — two kinds of buckets
A request/batch must acquire tokens from **both** before proceeding; if either is short it waits (bounded) then is
rejected with a retryable throttle:

1. **Tenant admission buckets (capacity + fairness).** Per-tenant / per-keyspace buckets sized by the tenant's
   provisioned rate (WCU/RCU) — the GAC allotment (§7.4). Bounds *how much* a tenant may do; burst = bucket depth.
2. **Pipeline-health buckets (backpressure).** Node/region-local buckets, one per **finite shared resource in the
   read/write pipeline**; a write reserves from the relevant ones:
   - **memtable-memory** (bytes) — reserved on write, **released on flush**.
   - **WAL / in-flight-upload** — bound un-truncated WAL + concurrent uploads; **replenished when a flush lands and
     the WAL truncates**.
   - **compaction-debt** — depleted as L0 / pending-manifest backlog grows; **replenished as compaction catches up**.
   - **object-store request/$** — bound request rate and cost to the store.
   - **cache-fill (per-tenant)** — bound how fast a tenant pulls cold blocks into the shared block cache, so one
     tenant's scan can't evict another's hot set (this is the cache-isolation axis of §1.1).

Because both kinds are the **same currency**, backpressure is **naturally per-tenant fair**: when a shared pipeline
bucket runs low, the fair queue (§7.3) decides *whose* tokens are honored — the tenant *causing* the pressure is
throttled, not an innocent neighbor.

### 7.2 Credit feedback loop = backpressure
Pipeline-health tokens are **credits granted by downstream progress** (credit-based flow control, à la TCP/RDMA):
each stage returns tokens as it drains — flush completes → memtable + WAL tokens returned; upload completes →
in-flight tokens returned; compaction reduces L0 → compaction-debt tokens returned. **Token level *is* pipeline
health.** When object storage or compaction lags, credits stop flowing, buckets empty, and ingress *automatically*
slows — before the WAL backs up or memory blows. There is no separate "backpressure signal"; it's the absence of
credits. (This is the mechanism §6.2/§6.5 refer to.)

### 7.3 Fair scheduling under scarcity
When tokens are plentiful everyone proceeds. When a shared bucket is scarce, a **weighted fair queue** (per
tenant/keyspace, virtual-time) hands out the scarce tokens proportionally to configured weights — throttling is
proportional and starvation-free. This is the "*when*"; the tenant admission bucket is the "*how much*". Applies to
serving CPU (read/exec pools) as well as the write pipeline.

### 7.4 Global Admission Control (cross-node)
A tenant's cap is **cluster-wide**, not per-node. The metadata plane is the GAC authority: it hands each node a
**local sub-allotment** of a tenant's tokens; a node asks for more when low and returns unused tokens (DynamoDB GAC).
If the authority is briefly unreachable, nodes fall to a **degraded mode** (bounded fail-open at the last-known rate)
rather than hard-stopping — availability over precision for a short window.

### 7.5 Client contract — throttle, don't collapse
Short overloads **queue with a deadline**; sustained overload **rejects** with a retryable throttle + backoff hint
(never build unbounded queues — anti-collapse). Two distinct signals so clients react correctly and avoid thundering
herds: **`QuotaExceeded`** ("you hit *your* limit — slow down") vs **`Overloaded`** ("this node/region is saturated —
back off / retry elsewhere").

---

## 8. Time & consistency

`txn` keyspaces need Snapshot Isolation, which needs a timestamp order. kv9's key move: this order is **per txn group,
not global** — which is what makes the timestamp path scale.

### 8.1 Sharded TSO — a pool of providers serving many keyspaces / txn groups
Because a transaction never crosses a txn group (§3.6), each group needs timestamps ordered only within itself. So
kv9 runs a **pool of TSO providers** (diagram: `docs/ARCHITECTURE.md` §5). Mapping (all metadata in the system
keyspace):

```
   tenant ─1:N▶ keyspace ─1:N▶ txn group ─1:1▶ TSO timeline ─N:1▶ TSO provider (pool member)
                  (txn keyspace only; raw keyspace has no txn group / no timeline)
```

- A **cluster has many TSO providers, not one.** Each provider **hosts one or more TSO timelines** and serves the
  txn groups assigned to those timelines — a single provider serves timelines from *different* keyspaces at once.
- A **txn group owns exactly one timeline** (its own monotonic clock + persisted window). A **`txn` keyspace has ≥1
  txn groups** (default 1 → one timeline, a single classic TSO for that keyspace); a **`raw` keyspace has none**.
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

The key point: **the keyspace boundary is absolute — no tier ever permits a cross-keyspace transaction.** What the
tier affects is only the *cross-txn-group* restriction **within** a sharded `txn` keyspace: an unsharded keyspace is
one Tier-1 timeline with full SI; a keyspace that shards its TSO into groups yet still needs transactions across those
sub-groups can opt into Tier 2/3 (accepting the clock requirement + commit-wait). Spanner's within-keyspace mechanisms
— leader leases (§5.3) and 2PC participant-coordinator (§9.1) — kv9 already uses.

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
  point (Spanner's participant-coordinator). ResolveLock cleans up on failure.
- **Confinement is keyspace-absolute + fail-fast.** A transaction runs in **one `txn` keyspace**; **any access to
  another keyspace is rejected immediately** — the keyspace boundary is absolute and **never** relaxed by any
  timestamp tier. Within that keyspace the txn is pinned to one **txn group** (the group whose sub-range covers its
  keys; the keyspace's single group by default); touching a *different* txn group of the same keyspace returns
  `CrossTxnGroup` at the router. This is what makes per-group TSO timelines correct (§3.6). (`raw` keyspaces have no
  transactions at all.)
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
- **Rebalance is meta-only and near-free** (§6.5): moving a region ships its **manifest** (file references), and the
  target attaches by lazily pulling blocks from object storage — seconds, not a bulk copy. Split/merge can likewise
  be meta-only (children reference the parent's immutable files until compaction rewrites).
- **But cheap-to-move ≠ move-always.** Every move invalidates cached routing → epoch-reject → refresh load on L1. So
  the placement loop is **damped**: hysteresis (imbalance must persist beyond a window), per-region **cooldown**, a
  **cap on moves/interval**, and **batched routing updates**. Cheapness of the move must not become churn.
- **Placement / rebalance** is a MetaLeader responsibility and is **consumption-aware from day one** (WCU/RCU/CPU/
  region-count/cache-locality/**object-store request cost**), *not* local-disk-capacity-first — which is now not even
  meaningful, since bulk data is on object storage, not local disk. Placement optimizes compute/cache/load, and
  weighs the $ of object-store requests (compaction), not data volume.
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
│   ├── meta/     ← MetaStore = small SQL/catalog engine over the system keyspace (docs/METADATA-CATALOG.md):
│   │               schema/codec/tables/migrate; membership · catalog · routing · placement/scheduler ·
│   │               TSO provider pool — all as tables; multi-level metadata (L0/L1); Bootstrap FSM; MetaLeader
│   ├── engine/   ← Engine trait + MemEngine; MVCC layout (default/lock/write); WriteBatch;
│   │               ObjectStore trait (S3/GCS/Azure/local) + disaggregated LsmEngine design (§6.5);
│   │               Manifest (immutable-SST file refs, mutable via raft), local block cache
│   ├── raft/     ← RaftGroup trait + single-node stub
│   ├── region/   ← Region, RegionRouter, epoch, split/merge (throughput-aware), WalStream/WalPool (§6.4)
│   ├── txn/      ← Percolator 2PC (txn keyspaces) + txn-group confinement; raw executor
│   └── server/   ← Node assembly, API traits (Txn/Raw/Admin/Router), request routing
└── src/ (bin `kv9`) ← main.rs: single binary; node CLI (--node-id, --join, --data-dir, --addr)
```

`cargo check --workspace` passing is the milestone's definition of done. Method bodies may be `unimplemented!()`
/typed errors, but types, traits, and module boundaries are real and reflect this design.

---

## 13. Design principles distilled
The choices above, as standalone principles:
0. **Object storage is the single source of truth; compute is stateless.** All durable state lives in object storage;
   compute holds only cache + a transient, raft-replicated WAL ingress. This is the property that makes kv9
   cloud-native — seconds-scale elasticity, scale-to-zero, instant failover, independent storage/compute scaling.
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
11. **Quiesce idle regions** — heartbeat/election cost must not scale with region count (multi-tenant has millions of
    cold regions). (§6.1)
12. **Forward-compatible formats, never panic on the unknown.** Raft-log entries, manifests, and on-object formats
    carry versions and tolerate unknown fields/types; new entry types are gated by cluster version. Rolling upgrade
    is a first-class constraint, not an afterthought.
13. **No unquota'd in-memory path.** Every large read/load/scan/value/compaction-input either streams or counts
    against memory tokens (§7) — there is no "no-size-hint" bypass that can OOM a node.
14. **Watermark discipline.** Name the watermarks crisply (committed-index, applied-index, flushed/persisted-index
    that gates truncation, safe-time for reads); one writer per watermark; never compare composite state by *summing*
    components (use tuples/lexicographic order).
15. **Metadata mutations are atomic & idempotent** — a single raft (or `system`-group 2PC) commit, never a
    multi-step-by-convention sequence with partial-rollback leaks; one authoritative source per mapping.
16. **Invariants are enforced, not hoped for** — protect on-disk/raft state with types (illegal states
    unrepresentable) or hard asserts that hold in release; never "log-and-continue" past a corrupted invariant.
17. **Per-tenant observability + tests are first-class** — per-tenant metrics from day one (needed for billing/QoS
    anyway); no subsystem ships without tests; accounting accumulates fractional usage (never truncates).

---

## 14. Future directions (documented, not in v0)
- **Externalized log service** (log-is-the-database): move the WAL/log off the raft group into a shared replicated
  log so compute becomes fully stateless and page/data servers rebuild from the log on object storage (§6.5 open
  decision). v1 keeps the raft-group-local WAL.
- **Coprocessor / pushdown**, **CDC / change feeds**, **PITR / native backup** — layers above the region.
- **Elastic background compute** — offload compaction / index build to stateless workers reading/writing object
  storage directly, keeping the serving path light.
- **HLC / TrueTime** timestamp sources behind `TimeSource` (§8.2).

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
