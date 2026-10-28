# kv9-region

The region layer (DESIGN §3.3, §6, §9).

- `Region` + `RegionEpoch` (`conf_ver`, `version`) — the range shard = Raft group, with
  epoch-checking on every request (DESIGN §6.1).
- `RegionRouter` — resolves `key → region` and epoch-checks; the client-side cache
  follows the MemDS freshness discipline (DESIGN §5.4).
- `manifest` — the manifest seam between a region's Raft log and its object-storage checkpoints.
- **Sharded WAL:** `WalStream` + `WalPool` + a stable `region → stream` assignment —
  shared-but-sharded log ingress that keeps group-commit amortization while lifting the
  single-WAL write ceiling (DESIGN §6.4).

Split/merge decisions are a design goal (DESIGN §10); the split prototype, including the throughput-aware
split hooks that used to live here, is on the `experimental/range-split` branch, not on `master`.
