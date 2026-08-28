# kv9-engine

The storage engine abstraction (DESIGN §6.2 "Storage engine abstraction") and its v0
in-memory implementation.

- `Engine` trait — the per-region logical LSM keyed within a range.
- `MemEngine` — an in-memory engine for the skeleton and tests, backed by a persistent
  (structurally shared) ordered map so snapshots are O(1) and writes never copy.
- `ReadView` / `Engine::snapshot` — a consistent view. Reads taken through one view agree
  with each other, which two separate `get` calls do not: a whole `WriteBatch` can commit
  between them. `iter` / `iter_rev` stream in both directions so a caller merging its own
  write buffer can stop at a limit instead of materializing a range (DESIGN §13
  principle 13, "no unquota'd in-memory path").
- `ColumnFamily` (`default` / `lock` / `write`) — the Percolator MVCC layout for `txn`
  keyspaces (DESIGN §9.1 "Txn keyspaces — Percolator 2PC over MVCC").
- `WriteBatch` — atomic multi-CF write unit applied from a committed raft entry. Atomic
  means no reader observes a partial batch, across column families.

A future `LsmEngine` plugs in behind the same trait (DESIGN §6.2, §12 "Crate layout"). It
will be a **minimal native LSM, not RocksDB**: RocksDB assumes local-first storage and
fights the immutable-SST-on-object-storage / manifest-in-raft model this engine is shaped
around (`docs/ROADMAP.md`, Dependency decisions; DESIGN §6.5 "Storage-compute
disaggregation").

Note on layering: this crate owns the MVCC *layout and codec*, while the ordered-KV
container beneath it is opaque-bytes — it never reinterprets keys, and the multi-tenant
key prefix is encoded above it in `kv9_common::codec`. Flush is likewise not owned here:
a region's manifest lives in raft-replicated region state, the leader builds and uploads
SSTs, and followers adopt the resulting file ids (DESIGN §6.5).

See `DESIGN.md` §6 and §9.
