# kv9-server

Node assembly and the request-serving surface (DESIGN §4, §11).

- `api` — the v0 API surface as Rust traits (so it compiles without a protoc
  toolchain): `TxnApi`, `RawApi`, `AdminApi`, `RouterApi` (DESIGN §11). Every data
  request carries `(keyspace_id, region_epoch)`.
- `routing` — request routing: resolve keyspace → region, epoch-check, and validate the
  request's API type against the keyspace declaration (DESIGN §11).
- `node` — `Node`: assembles the store (engine + raft), the metadata plane (catalog,
  routing table, bootstrap, TSO pool), the region router, and the txn/raw executors
  into one process. One binary, one node type; roles are behaviors (DESIGN §3.5).

Auth on the admin/meta surface is in scope from day one (DESIGN §11, §13 principle 9).
