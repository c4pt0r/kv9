# kv9-server

Node assembly and the request-serving surface (DESIGN §4, §9, §11, §12).

- `api` — the public surface as Rust traits: `RawApi` (point reads/writes, batch get/put, the atomic range
  batch `RawBatch`, scan, delete-range, TTL) and `AdminApi` (keyspaces, data groups, membership, retention,
  migration). Every raw request carries `(keyspace_id, region_epoch)` and is checked against the committed
  range directory at admission and again at apply.
- `grpc`, `point_stream`, `point_wire` — the `kv9.v1.Kv9` gRPC service (`proto/kv9.proto`), its bearer-token
  authenticator and admission/backpressure (`admission`), typed refusal markers in trailing metadata, and the
  persistent point-stream transport that projects onto the same handlers.
- `client` — the persistent raw client, the range-aware routed client and the TTL client used by the
  binary, the workload tools and the tests (the language SDKs under `sdk/` implement the same contract).
- `runtime` — `NodeRuntime`: the election-first bootstrap, the metadata group, data-group drivers, the
  maintenance loops (checkpoint upload, raft-log compaction, TTL), remote storage and retention plumbing.
- `region_manager`, `data_groups`, `node`, `routing`, `fence`, `observability` — local data-group
  lifecycle (recovery, retirement, reclamation), the data-group admin client, catalog assembly, the
  apply-time epoch fence and the status/metrics files.
- `bin/` — `kv9-workload`, `kv9-batch-workload`, `kv9-batch-benchmark` (the single `kv9` binary lives in
  the repository root `src/`).

Auth on the admin/meta surface is in scope from day one (DESIGN §11, §13 principle 9). Range split and
merge are not part of this crate on `master`; the split prototype lives on `experimental/range-split`.
