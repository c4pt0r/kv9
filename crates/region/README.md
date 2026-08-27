# kv9-region

The region layer (DESIGN §3.3, §6, §9).

- `Region` + `RegionEpoch` (`conf_ver`, `version`) — the range shard = Raft group, with
  epoch-checking on every request (DESIGN §6.1).
- `RegionRouter` — resolves `key → region` and epoch-checks; the client-side cache
  follows the MemDS freshness discipline (DESIGN §5.4).
- Split/merge hooks with **throughput-aware** signatures — split triggers on consumed
  throughput/CPU and picks the split key from the observed access distribution, never
  crossing a keyspace boundary (DESIGN §10, §13 principles 3 & 7).
- **Sharded WAL:** `WalStream` + `WalPool` + a stable `region → stream` assignment —
  shared-but-sharded log ingress that keeps group-commit amortization while lifting the
  single-WAL write ceiling (DESIGN §6.4).
