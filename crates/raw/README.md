# kv9-raw

The raw-KV executor (DESIGN §9).

- `raw` — the direct-KV planner for `raw` keyspaces: `RawPut/RawGet/RawDelete/RawScan/
  RawBatchGet`, the atomic **range batch** (`RawBatch`: mixed put/delete, one Raft entry,
  all keys inside one range — DESIGN §9.1), chunked `RawDeleteRange`, and TTL rows.
  No locks, no 2PC, no MVCC: every plan is applied as one fenced command by the driver.

The executor holds no engine or Raft handle by construction (see the module docs); it
only produces plans that a driver owner can commit.
