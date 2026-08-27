# kv9-engine

The storage engine abstraction (DESIGN §6.2) and its v0 in-memory implementation.

- `Engine` trait — the per-region logical LSM keyed within a range.
- `MemEngine` — an in-memory `BTreeMap`-backed engine for the skeleton and tests.
- `ColumnFamily` (`default` / `lock` / `write`) — the Percolator MVCC layout for
  `txn` keyspaces (DESIGN §8.1).
- `WriteBatch` — atomic multi-CF write unit applied from a committed raft entry.

A future `LsmEngine` (RocksDB FFI or native Rust LSM) plugs in behind the same trait
(DESIGN §6.2, §12). See `DESIGN.md` §6 and §8.
