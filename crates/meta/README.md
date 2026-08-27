# kv9-meta

The self-hosted metadata plane — no external placement driver (DESIGN §5, §8, §10).
Metadata is just data in a reserved system keyspace, replicated by the same Raft.

- `membership` — node id → address/state/heartbeat/capacity (DESIGN §5.1).
- `catalog` — keyspace catalog + tenants (DESIGN §5.1).
- `routing` — the region routing table snapshot the router caches (DESIGN §5.1).
- `layered` — **multi-level metadata**: `L0` root (`META_REGION_0`, tiny/fixed) and
  `L1` sharded meta-regions that split/merge like data (DESIGN §5.1.1).
- `bootstrap` — the **election-first** state machine
  (`Discovering → BootstrapElection → Initializing | WaitForBootstrap → Serving`),
  DESIGN §5.2.
- `leader` — `MetaLeader` (Raft leader of `META_REGION_0`) with lease + gray-failure
  double-confirm (DESIGN §5.3).
- `placement` — consumption-aware scheduler scoring + GAC token buckets (DESIGN §10).
- `tso` — a **pool of `TsoProvider`s**, each hosting one or more per-txn-group
  timelines; `TimestampOracle` trait + `EmbeddedTso` stub, `TxnGroupId` on the handle
  (DESIGN §8.1–§8.2).
