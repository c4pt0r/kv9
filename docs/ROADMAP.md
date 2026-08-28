# kv9 — Development Roadmap

How to build kv9 from the current design (`DESIGN.md`). Ordering principle: **build the distributed backbone first,
mock the storage engine; then swap in the real disaggregated engine.**

Rationale: the *structural* spine of kv9 is the **self-hosted metadata plane** (raft + election-first bootstrap + the
SQL catalog) — everything else plugs into it, and **raft is needed regardless** (metadata *and* user regions use it).
So we front-load the highest **distributed-systems** risk (consensus, election, bootstrap, failover, self-hosted
metadata) with the storage engine **mocked** behind the `Engine` trait, then bring in the disaggregated
object-storage engine (the thesis) as a clean swap. Build vertical slices so there is always a running `kv9`.

## Dependency decisions (make up front — they shape everything)
- **Consensus:** `openraft` (pure-Rust, async — leader election, log replication, membership, snapshots). *Not*
  `raft-rs` (pulls protobuf/`protoc`). Needed from Phase 1.
- **Storage engine (Phase 2):** a **minimal native LSM** (memtable + immutable SST writer/reader + manifest), *not*
  RocksDB — RocksDB assumes local-first storage and fights the immutable-SST-on-object-storage / manifest-in-raft
  model. Until then, the raft state machine is the skeleton's `MemEngine` (mock).
- **Object storage (Phase 3+):** the `object_store` crate (pure-Rust S3/GCS/Azure/local) behind `ObjectStore`.
- **Async I/O:** `tokio`. **Wire (later):** `tonic` gRPC.

## Phases

### Phase 1 — Metadata plane: raft + election + SQL catalog (storage MOCKED). ← start here
Bring up `raft-rs` for the **system-keyspace raft group** (`META_REGION_0`), whose state machine applies committed
entries into a **mock `MemEngine` KV**. On top of that KV, build the **metadata SQL catalog** (`meta` crate,
`docs/METADATA-CATALOG.md`): row/index codec, hardcoded schema, typed accessors, transactions. Add the
**election-first bootstrap** FSM (join-set → elect → winner initializes the catalog), **membership**, and
`CreateKeyspace` / regions catalog. Multi-node.
- *Demo:* a **multi-node cluster bootstraps itself (election-first), self-hosts its metadata as SQL tables on raft,
  handles membership + keyspace creation, and survives leader failover** — storage mocked.
- *Retires:* the hardest, most structural risk — consensus integration, election, bootstrap, self-hosted catalog,
  failover — the spine everything hangs on.
- *First concrete task:* `raft-rs` single-node — `RawNode` behind the synchronous `RaftGroup`, a `Ready` loop that
  persists entries + hardstate **before** sending messages, and a `MemEngine` state machine — with a
  `propose(put)→apply→get` round-trip test; then the catalog schema/codec on top; then multi-node election +
  bootstrap. Correlate a proposal by `(term, index)` matched against the applied entry, never by log position
  alone: a position can be overwritten by a new leader's entry, so "applied ≥ N" would report success on another
  command.

### Phase 2 — Disaggregated storage engine (the thesis), swapped in behind `Engine`
`engine`: memtable → **local WAL** → flush to **immutable SST** on an `ObjectStore` (local-dir impl first) →
**manifest** (file refs, the mutable pointer) → block cache → read path → **recovery** (replay WAL + load manifest).
Swap the raft state machine from `MemEngine` to this real engine; wrap user data in the **raw KV API**.
- *Demo:* real disaggregated engine under the raft groups; data flushes to a local "bucket"; **restart recovers**;
  a region re-opens purely from its manifest.
- *Retires:* source-of-truth / immutability / flush→manifest→truncate / SST format / recovery — the storage thesis.

### Phase 3 — Real object storage + transactions
Point `ObjectStore` at **S3 / MinIO** (prefix layout, multipart, checksums, **GC** = refcount + orphan scan, first
memtable-memory/backpressure tokens). Add **Percolator SI**: embedded TSO (one timeline), `default/lock/write` CFs,
prewrite/commit/get, MVCC reads — **keyspace-confined**.
- *Demo:* a `txn` keyspace does SI transactions on real object storage; GC reclaims; slow store throttles, not OOMs.
- *Retires:* object-storage engineering + the transaction model on the disaggregated engine.

### Phase 4 — Multi-region + meta-only elasticity (the payoff proof)
User regions each their own raft group (raft-log = the WAL); **leader flushes → manifest change via raft → followers
adopt**; region routing from the catalog (L0/L1) + epoch checks; **meta-only snapshot** (ship manifest, attach from
object store); split/merge (throughput-aware) + pre-shard; rebalance (damped).
- *Demo:* add a node → a region **attaches meta-only in seconds** (scale-out = metadata, not data); split a hot
  region; leader failover.
- *Retires:* meta-only movement, multi-region routing, split/merge, elasticity.

### Phase 5 — Scale & multi-tenant hardening
Distributed + L1-sharded scheduler; **sharded TSO** (per-txn-group timelines, provider pool); **idle-region
quiescing**; full **token flow-control** (cross-node GAC, fair queue, cache-fill); per-tenant metrics; **wire layer
(`tonic`)** + auth.
- *Demo:* elastic, multi-tenant, throughput-scaling cluster with predictable QoS.

## Cross-cutting from day one
Unit + property tests; **per-tenant metrics**; versioned on-disk/raft formats (never panic on unknown); no
unquota'd in-memory path. Early in Phase 4, invest in a **deterministic simulation harness** (FoundationDB /
TigerBeetle style) for raft/failure paths — it pays for itself.

## Maps to DESIGN milestones
Phase 1 ≈ M2/M4 backbone (raft + bootstrap + MetaLeader, storage mocked) · Phase 2 ≈ M1/M2 storage ·
Phase 3 ≈ M1 txn + M2 persistence · Phase 4 ≈ M3 multi-region/split/rebalance · Phase 5 ≈ M4/M5 (sharded TSO, GAC,
scheduler, scrubber, wire).
