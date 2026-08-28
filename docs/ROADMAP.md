# kv9 — Development Roadmap

How to build kv9 from the current design (`DESIGN.md`). Ordering principle: **build the distributed backbone first,
keep storage simple (WAL + replay); then swap in the real disaggregated engine.**

Rationale: the *structural* spine of kv9 is the **self-hosted metadata plane** (raft + election-first bootstrap + the
SQL catalog) — everything else plugs into it, and **raft is needed regardless** (metadata *and* user regions use it).
So we front-load the highest **distributed-systems** risk (consensus, election, bootstrap, failover, self-hosted
metadata) behind the `Engine` trait — Phase 1 uses only a **simple WAL-and-replay engine sufficient for restart
testing** — then bring in the disaggregated object-storage engine (the thesis) as a clean swap. Build vertical slices so there is always a running `kv9`.

## Dependency decisions (make up front — they shape everything)
- **Consensus:** `raft-rs` (tikv/raft-rs 0.7.x, feature **`protobuf-codec`** — builds with **no protoc / no native
  toolchain**; the `prost-codec` feature *does* require protoc and **must not be used**). Chosen for: synchronous
  pull-model (`RawNode`/`Ready`) matching the region apply loop, built-in `pre_vote` + `check_quorum` (§5.3
  gray-failure discipline), and tick-driven cores compatible with idle-region quiescing (§6.1) and deterministic
  simulation. raft-rs provides the consensus core only — transport, log storage, state machine, and the drive loop
  are ours. *Not* `openraft`: async runtime (tokio) + push-model apply would force an async boundary and a
  state-machine trait redraw across the workspace; no check-quorum primitive. Needed from Phase 1.
  *(Decision record: build probes + 3-node spike, 2026-08-27; approved by EdHuang.)*
  CI note: `protobuf-codec` needs no protoc — but a **broken or partial `protoc` earlier on `PATH` fails the
  build** (protobuf-build probes it, then panics) in a way that looks unrelated to protobuf. CI images should
  either omit protoc entirely or ensure the one present is functional.
- **Storage engine (Phase 2):** a **minimal native LSM** (memtable + immutable SST writer/reader + manifest), *not*
  RocksDB — RocksDB assumes local-first storage and fights the immutable-SST-on-object-storage / manifest-in-raft
  model. Until then the raft state machine runs on the Phase 1 simple WAL engine (`MemEngine` remains for
  tests and the in-process harness).
- **Object storage (Phase 3+):** the `object_store` crate (pure-Rust S3/GCS/Azure/local) behind `ObjectStore`.
- **Async I/O:** `tokio`. **Wire (Phase 1-final onward):** pure-Rust `tonic` gRPC for both public APIs and
  node-internal Raft/discovery. The server owns one listener and registers all services; Raft uses long-lived
  client streams with byte/count batching, while the synchronous core is reached only through channels.

## Phases

### Phase 1 — Metadata plane: raft + election + SQL catalog, on real multi-process nodes. ← start here
Bring up `raft-rs` for the **system-keyspace raft group** (`META_REGION_0`), whose state machine applies committed
entries into the **Phase 1 simple WAL engine** (`MemEngine` stays for tests). On top of that KV, build the **metadata SQL catalog** (`meta` crate,
`docs/METADATA-CATALOG.md`): row/index codec, hardcoded schema, typed accessors, transactions. Add the
**election-first bootstrap** FSM (join-set → elect → winner initializes the catalog), **membership**, and
`CreateKeyspace` / regions catalog.
**Multi-node means real OS processes, not in-process peers.** Phase 1 acceptance is three `kv9` processes that
discover each other over the network, elect, bootstrap, survive `kill -9` of the leader, and let the killed node
restart and rejoin. (An earlier reading treated an in-process 3-peer harness as satisfying "multi-node"; it does
not, and in-process testing structurally cannot surface restart/identity bugs — the initialized-marker gap was
found exactly this way.)
**Storage in Phase 1 is a *simple* persistence engine for testing** — append-only WAL + replay on start,
tolerating a torn tail record — **not** the disaggregated engine. SST / compaction / manifest / object storage
remain Phase 2. Restart safety needs three things persisted, in two crates: raft **HardState (term + vote) + log**
(a raft *safety* requirement — a node that forgets its vote can vote twice in a term and elect two leaders), the
state-machine data, and the bootstrap initialized marker.
- *Demo:* **three real `kv9` processes** bootstrap themselves (election-first), self-host metadata as SQL tables on
  raft, handle membership + keyspace creation, **survive `kill -9` of the leader, and let the killed process
  restart and rejoin** — reproducible by a single command.
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
quiescing**; full **token flow-control** (cross-node GAC, fair queue, cache-fill); per-tenant metrics; production
TLS/mTLS and authorization hardening on the existing gRPC wire.
- *Demo:* elastic, multi-tenant, throughput-scaling cluster with predictable QoS.

## Cross-cutting from day one
Unit + property tests; **per-tenant metrics**; versioned on-disk/raft formats (never panic on unknown); no
unquota'd in-memory path. Early in Phase 4, invest in a **deterministic simulation harness** (FoundationDB /
TigerBeetle style) for raft/failure paths — it pays for itself.

What makes a test *count as evidence* is not decided here: **[docs/TESTING.md](TESTING.md)** is the single
authority on verification and acceptance criteria. This section says which kinds of testing the project
commits to; that file says when a red, a green, or a gate may be believed. Do not restate its criteria here.

## Maps to DESIGN milestones
Phase 1 ≈ M2/M4 backbone (raft + bootstrap + MetaLeader, simple WAL storage) · Phase 2 ≈ M1/M2 storage ·
Phase 3 ≈ M1 txn + M2 persistence · Phase 4 ≈ M3 multi-region/split/rebalance · Phase 5 ≈ M4/M5 (sharded TSO, GAC,
scheduler, scrubber, wire).
