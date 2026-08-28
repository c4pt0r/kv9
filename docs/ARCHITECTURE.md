# kv9 — Architecture Diagrams (ASCII)

Companion to `DESIGN.md`. Each diagram cites the DESIGN section it visualizes.

---

## 1. Cluster topology — N identical single binaries, no external control plane (§4)

```
                    kv9 cluster : N identical single-binary nodes  (no external PD / etcd / Chubby)

  ┌──────────── node A ────────────┐  ┌──────────── node B ────────────┐  ┌──────────── node C ────────────┐
  │  Router | Txn/Raw | Admin  API │  │  Router | Txn/Raw | Admin  API │  │  Router | Txn/Raw | Admin  API │
  │ ─────────────────────────────  │  │ ─────────────────────────────  │  │ ─────────────────────────────  │
  │  DATA PLANE                    │  │  DATA PLANE                    │  │  DATA PLANE                    │
  │   region replicas (raft grps)  │  │   region replicas (raft grps)  │  │   region replicas (raft grps)  │
  │   Engine(LSM) + WAL pool       │  │   Engine(LSM) + WAL pool       │  │   Engine(LSM) + WAL pool       │
  │  META PLANE (member)           │  │  META PLANE (member)           │  │  META PLANE (member)           │
  │   system-keyspace raft groups  │  │   system-keyspace raft groups  │  │   system-keyspace raft groups  │
  └───────────────┬────────────────┘  └───────────────┬────────────────┘  └───────────────┬────────────────┘
                  └──────── raft replication (per region) + membership heartbeat across all nodes ────────┘

                    exactly one node currently holds  [ MetaLeader ]  — elected via raft, lease-held
```

---

## 2. One node = two planes in one binary (§4, §5)

```
 ┌────────────────────────────── one kv9 node (single binary) ──────────────────────────────┐
 │  gRPC :   Router API   │   Txn API   │   Raw API   │   Admin API                         │
 │              │              │             │             │                                │
 │              ▼              ▼             ▼             ▼                                │
 │        RegionRouter ──▶ TxnExecutor / RawExecutor ─────────────▶ Store                   │
 │        (caches L0→L1→L2)        │                                  │                     │
 │              │                  ▼                                  ▼                     │
 │              │           TimestampOracle                    Region (Raft group)          │
 │              │           (provider for txn-group)             ├─ RaftGroup (consensus)   │
 │              │                                                ├─ apply ─▶ Engine (LSM)   │
 │              │                                                └─ append ─▶ WAL stream    │
 │              │                                                            (from WalPool) │
 │   ┌──────────┴─────────── META PLANE  (this node is a member) ─────────────────────┐     │
 │   │  system-keyspace regions (raft) hold:                                          │     │
 │   │   membership · keyspace catalog · region routing (L1) · placement ·            │     │
 │   │   per-txn-group TSO windows · MetaLeader lease                                 │     │
 │   └────────────────────────────────────────────────────────────────────────────────┘     │
 └──────────────────────────────────────────────────────────────────────────────────────────┘
```

---

## 3. Multi-level metadata hierarchy — metadata scales like data (§5.1.1)

```
   L0  ROOT  ── META_REGION_0 : fixed id, tiny, NEVER grows (always in memory)
       │        holds only:  locations of L1 meta-regions ("meta of meta")
       │                     membership root · MetaLeader lease · TSO windows
       │  points to
       ▼
   L1  META-REGIONS  ── ordinary SHARDED regions in the system keyspace (split/merge like data)
       │        hold:  region routing table (region_id → range,epoch,peers,leader)
       │               keyspace catalog · placement state
       │        spread across many nodes  ⇒  metadata throughput & memory scale HORIZONTALLY
       │  points to
       ▼
   L2  USER REGIONS  ── tenant data ; range-sharded raft groups

   lookup :  key ─▶ L0: which L1 meta-region routes this key
                 ─▶ L1: which L2 user-region owns the key
                 ─▶ L2 leader
             (client caches every level; root near-static ⇒ almost all hits are L1/L2 cache)

   ↔ contrast: a centralized placement driver (e.g. TiKV's PD) keeps ALL region meta in one process's memory
      — a single-node metadata scaling ceiling. kv9's L1 sharding removes it.
```

---

## 4. Election-first bootstrap state machine (§5.2)

```
        start(--join seeds, --data-dir)
                │
                ▼
          ┌───────────┐   "is the cluster initialized?"  (ask join-set)
          │Discovering│───────────────┬───────────────────────────┐
          └───────────┘        initialized                    uninitialized
                                       │                             │
                                       ▼                             ▼
                                 ┌──────────┐              ┌────────────────────┐
                                 │ Joining  │              │ BootstrapElection  │  raft leader election
                                 └────┬─────┘              └─────────┬──────────┘  over empty META_REGION_0
                                      │                    elected   │   not elected
                                      │                    ┌─────────┴─────────┐
                                      │                    ▼                   ▼
                                      │            ┌──────────────┐    ┌──────────────────┐
                                      │            │ Initializing  │   │ WaitForBootstrap │
                                      │            │ write catalog:│   │ wait for catalog,│
                                      │            │ sys keyspace, │   │ then register    │
                                      │            │ default tenant│   │ self             │
                                      │            │ L0/L1 records,│   └────────┬─────────┘
                                      │            │ TSO window    │            │
                                      │            └───────┬───────┘            │
                                      │                    └─────────┬──────────┘
                                      └──────────────────────────────┤
                                                                     ▼
                                                                ┌─────────┐
                                                                │ Serving │  data-driven from here
                                                                └─────────┘
   (metadata init = ordinary committed raft entries ⇒ crash-safe & idempotent; a crashed initializer re-elects)
```

---

## 5. Sharded TSO — a pool of providers serving many keyspaces / txn groups (§3.6, §8.1)

```
   keyspaces              txn groups              timelines               TSO provider pool (meta plane)
  ┌────────┐             ┌──────────┐            ┌────────────┐          ┌───────────────────────────┐
  │  ks A  ├──┐          │ default  ├──────────▶ │ TL:default │───────▶  │ Provider P1  (on node A)  │
  │  ks B  ├──┼───────▶  └──────────┘            └────────────┘     ┌──▶ │  hosts TL:default , TL:G2 │
  │  ks C  ├──┘          ┌──────────┐            ┌────────────┐     │    └───────────────────────────┘
  │  ks D  ├──────────▶  │ group G2 ├──────────▶ │ TL:G2      │─────┘
  └────────┘             └──────────┘            └────────────┘          ┌───────────────────────────┐
  ┌────────┐             ┌──────────┐            ┌────────────┐          │ Provider P2  (on node C)  │
  │  ks E  ├──────────▶  │ group G3 ├──────────▶ │ TL:G3      │───────▶  │  hosts TL:G3              │
  └────────┘             └──────────┘            └────────────┘          └───────────────────────────┘

     keyspace ─N:1─▶ txn group ─1:1─▶ timeline ─N:1─▶ provider     (assignment is DATA, rebalanceable:
                                                                    hot group → dedicated provider;
                                                                    many cold groups → share one)
     INVARIANT: a transaction never crosses a txn group
                ⇒ timelines are independent, NO global order / NO cross-group timestamp comparison.
```

---

## 6. Sharded WAL — K independent streams, each shared by many regions (§6.4)

```
   regions on a node                          WAL pool  (K streams; default one per high-IOPS volume)

   R1 ─┐
   R4 ─┼──────────────────────────────▶  [ WalStream 0 ] ── fsync ─▶ vol0 ─┐
   R7 ─┘   group-commit: ONE fsync                                          │
           amortized across R1,R4,R7                                        │  background compaction
                                                                            │  ─────────────────────▶
   R2 ─┐                                                                    │  per-region rlog files
   R5 ─┼──────────────────────────────▶  [ WalStream 1 ] ── fsync ─▶ vol1 ─┤  (durable log stays
   R8 ─┘                                                                    │   per-region; the stream
                                                                            │   is only the ingress)
   R3 ─┐                                                                    │
   R6 ─┼──────────────────────────────▶  [ WalStream 2 ] ── fsync ─▶ vol2 ─┘
   R9 ─┘

   region ──(stable hash / placement)──▶ stream        streams fsync in PARALLEL ⇒ no single-writer ceiling
   ↔ contrast: TiKV serializes a store's raft log into ONE shared WAL/engine — great group-commit, but a
      single-writer ceiling on per-node write throughput. kv9's WAL pool keeps the amortization, lifts the ceiling.
   tune: more streams = more parallelism, less amortization per stream — match to # of IO devices.
```

---

## 7. Write path (txn keyspace) — commit does NOT wait for object storage (§6, §8)

```
   client
     │  begin: start_ts ← TSO(provider of this txn group)
     ▼
   TxnExecutor (Percolator 2PC)     all keys in ONE txn group  (cross-group ⇒ rejected at begin)
     │  prewrite → locks   ── commit_ts ← TSO ──▶  commit primary (atomic) → secondaries
     ▼  encode MVCC modify as custom raft log
   Region RaftGroup ── append ─▶  WAL stream fsync  +  raft majority   ═══════▶  ★ COMMIT ★
     │                              (LOCAL disk + quorum; independent of S3)
     ▼  apply committed entry
   Engine memtable          (raft log IS the memtable's WAL; recovery = replay raft log)
     │  memtable full → flush
     ▼
   L0 SSTable ── upload ─▶  OBJECT STORAGE        ← first & only time bytes leave the node (async, off hot path)
     │  flush committed → persisted-index↑ → WAL / raft-log truncation advances
     ▼
   compaction (storage→storage reorg; row + columnar + fts; may OFFLOAD to worker)
```

---

## 8. Read path — snapshot read, follower-capable (§8.3, §9)

```
   read (snapshot at ts)
     │  RegionRouter: keyspace → region (epoch-checked), api_type match
     ▼
   any replica with safe_time ≥ ts    (leader OR a caught-up follower — read-replicas are cheap)
     │  snapshot = memtable + local block cache
     │                    │ miss
     │                    ▼
     │              OBJECT STORAGE (immutable SST blocks)   ── see §9
     ▼
   lock-free consistent read at ts     (read-only txns take no locks)
```

---

## 9. Storage-compute disaggregation over object storage (§6.5)

```
  COMPUTE — region replicas are  (log + cache),  NOT full data copies
  ┌──────────────────────── region R : raft group across nodes ────────────────────────┐
  │   leader                        follower                     follower               │
  │   memtable                      memtable                     memtable               │  raft replicates
  │   local block cache             cache                        cache                  │  the RECENT LOG
  │   manifest (file-ids) ─────  mutable, raft-committed  ───────────────────────────── │  TAIL only
  └───────┬─────────────────────────────────────────────────────────────────────────────┘
          │  flush   : build SSTable → UPLOAD (immutable, write-once)
          │  compact : object-store → object-store  (offloadable to stateless workers)
          ▼
  ┌───────────────────────── OBJECT STORAGE = THE SOURCE OF TRUTH  (S3 / GCS / Azure) ──────────────────────────┐
  │  durable bulk (its own 9's)  ·  per-keyspace/tenant prefixes  ·  per-tenant CMEK        │
  │  immutable SSTs (file-id → hash-prefixed key)  ·  logical-delete + lifecycle GC         │
  └─────────────────────────────────────────────────────────────────────────────────────────┘

  durability :  recent tail = raft-majority WAL (local)   +   flushed bulk = object storage
  move region:  ship MANIFEST (file refs) → attach → lazily pull blocks   ⇒   rebalance in seconds
  invariants :  SSTs immutable (object storage sees only creates/deletes) ; manifest mutable ONLY via raft
  backpressure: upload latency gates WAL truncation ; object-store blip → writes continue on local WAL
```

---

## 10. Token-based flow control — capacity + fairness + backpressure in one currency (§7)

```
  a write/batch must acquire tokens from BOTH before it proceeds  (else: bounded wait → retryable throttle)

  ┌── tenant admission bucket (per-tenant/keyspace) ──┐      ┌── pipeline-health buckets (per node/region) ──┐
  │  sized by provisioned rate (WCU/RCU) = GAC        │      │  memtable-memory   WAL/in-flight-upload        │
  │  "how MUCH a tenant may do"  · burst = depth      │      │  compaction-debt   object-store req/$          │
  └───────────────────────────────────────────────────┘      │  cache-fill (per-tenant)                        │
                    ▲  refill from GAC authority               └────────────────────────────────────────────────┘
                    │  (local sub-allotment; degraded                         ▲   credits RETURNED by
                    │   fail-open if authority unreachable)                    │   downstream progress:
              ┌─────┴──────┐                                                   │   flush done → memtable+WAL tokens
              │ MetaLeader │  = GAC authority (§7.4)                           │   upload done → in-flight tokens
              └────────────┘                                                   │   compaction↓ → compaction-debt tokens
                                                                              (absence of credits = BACKPRESSURE,
   scarce shared bucket ─▶ weighted fair queue (per-tenant, virtual-time)      no separate signal)
                          decides WHOSE tokens are honored ⇒ the tenant CAUSING
                          the pressure is throttled, not a neighbor

   client signals:  QuotaExceeded ("slow down — your limit")   vs   Overloaded ("retry elsewhere — node saturated")
```
