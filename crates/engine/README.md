# kv9-engine

`Engine` provides atomic multi-column-family batches and stable `ReadView` snapshots.
`MemEngine` uses structurally shared ordered maps; `WalEngine` adds local durability.
`ReplicatedEngine` requires data and its exact `(term,index)` in one durable record.
WAL v2 supports positioned and unpositioned records, reads legacy v1, and rejects
complete non-monotonic positioned histories.

`checkpoint` seals a full-state `FrozenFlush`, writes immutable SSTs through the real
MinIO backend, verifies remote visibility, and yields non-cloneable `PreparedSst`
capabilities. The server/region layers consume these into a Raft manifest attempt.
After ordered apply, `WalEngine` saves the checkpoint reference and reclaims the
covered WAL by atomic tail replacement. Recovery verifies remote SSTs before replay.

All data currently stays in memory; the 48 MiB full checkpoint cap, synchronous WAL
copy/rename, and lack of incremental LSM/block cache are explicit initial limits.
`MemoryObjectStore` is only a test fixture. See [object storage](../../docs/OBJECT-STORAGE.md)
and [roadmap](../../docs/ROADMAP.md).
