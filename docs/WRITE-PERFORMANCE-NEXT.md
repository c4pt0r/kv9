# Write performance against three-copy Redis

Updated: 2026-09-12. The current priority is write throughput and latency,
targeting Redis with one primary and two replicas. Read optimization is held at
the selected ThinLTO/Safe ReadIndex baseline. The experimental lease work and
its remaining clock/Chaos gates are retained; read parity is not claimed and is
no longer a prerequisite for this write phase.

## Comparison contract

| Panel | Successful call requires | Interpretation |
| --- | --- | --- |
| KV9, three voters | Existing Raft commit, durable apply and response fences | Preserve all current consistency and synchronization rules. |
| Redis, primary + two replicas, WAIT 1 | SET/MSET OK and at least one replica acknowledgment on that connection | Primary replication-confirmed reference; two acknowledged copies, not Raft semantics. |
| Redis, primary + two replicas, WAIT 2 | SET/MSET OK and both replica acknowledgments on that connection | Additional all-replica latency/throughput reference. |
| Redis asynchronous replication | SET/MSET OK only | Optional separately labeled reference; never substitute it for WAIT results. |

[WAIT](https://redis.io/docs/latest/commands/wait/) applies to preceding writes
on the same connection and returns the actual acknowledgment count. It does not
make Redis strongly consistent or provide replica fsync receipts. The installed
Redis 7.0.15 lacks [WAITAOF](https://redis.io/docs/latest/commands/waitaof/), which
requires Redis 7.2 or newer. A future fsync-confirmed panel needs a separately
qualified version/configuration; even WAITAOF does not establish Raft semantics.

The [first accepted performance panel](WRITE-REDIS3-BASELINE.md) uses KV9's
normal synchronization calls on explicitly volatile tmpfs WAL and Redis with
save disabled and appendonly=no.
It isolates replication/protocol/CPU cost on one shared host. It cannot establish
power-loss durability, independent host failure, cross-host capacity or equal
durability. Keep actual disk costs in a separate panel. Preserve all historical
standalone Redis measurements under their original configuration and hashes.

## Implemented reference client

`kv9-redis-batch-reference` version 4 adds mandatory, explicit
`write_confirmation` configuration. For example, a point-write run includes:

```json
{
  "version": 4,
  "read_api": "get",
  "write_api": "set",
  "batch_size": 1,
  "write_confirmation": {"kind": "wait", "replicas": 1, "timeout_ms": 100}
}
```

This fragment supplements the existing address, deadline, dataset, concurrency
and bounded-load fields. Batch writes use mget/mset and batch_size=64.
`{"kind":"async"}` selects explicit asynchronous acknowledgment. WAIT counts
must be 1 or 2; its nonzero timeout cannot exceed the original call deadline.
Null, missing v4 confirmation and unknown fields are rejected.

Each worker permits one logical call in flight on one persistent connection.
It sends the data command and WAIT together, consumes both responses and uses
one absolute deadline. There is no additional artificial client round trip,
no second connection for confirmation and no retry of an uncertain write.
A short acknowledgment count is `replication_shortfall` in the `unknown_write`
population. A timeout, lost response or malformed confirmation cannot turn the
preceding OK into a successful logical write. Reads issue no WAIT.

Reports retain whole-call/dispatch latency histograms, separate command and
confirmation attempts, actual replica acknowledgment counts and their combined
RESP command count. One SET+WAIT is one logical call and two RESP commands;
one MSET(64)+WAIT is one logical call and 64 input items. Attempt counters record
client send attempts, not guaranteed server execution. Version 1–3 input and
metric shapes stay compatible. The current tree also restores the exact v3
point GET/SET implementation from `0be806d9671e2c50701a64aa7889c8859b7648ba`,
which was already used by the retained matched point measurements.

[Local validation and original evidence](redis-replication-reference-v1/README.md)
cover real three-process SET/MSET confirmation, replica pause/shortfall controls,
unknown-write/deadline/framing tests and backward report compatibility. This
checkpoint provides no new QPS result and changes no KV9 runtime algorithm.
The [independent v4 reader and clean release](write-reference-qualification-v1/README.md)
now pass 44 Python tests, all four original client reports and 32 rejection
controls. The subsequent [12 smoke and 24 timed cohorts](WRITE-REDIS3-BASELINE.md)
now pass independent readback. All 18,993,624 measured calls succeed once;
original failed audit and schema repair remain retained without workload reruns.

## Executable development order

1. Completed: independent v4 accounting, clean releases and exact source/binary,
   Redis three-node configuration, CPU and finite-protocol binding. Preserve
   the current build lock, immutable inputs and original retention/disk guards.
2. Completed: SET/Put and MSET/BatchPut(64), c1/c64, 128-byte values, ten-second
   windows and two opposite orders, with 12 fresh smokes and 24 timed cohorts.
   The [report](WRITE-REDIS3-BASELINE.md) includes successful calls/items per second,
   mean/p50/p95/p99, all outcomes, drops and independently recomputed client/all
   three-server CPU. No build, profiler or codec overlaps timing. Use these as
   the selected write baseline; do not repeat runs to conceal failed attempts.
3. The existing [slicing-by-eight CRC candidate and proof](https://github.com/c4pt0r/kv9/blob/65511010e2fda8adba04efd831a39bcdca1979a4/docs/CRC32-SLICING-QUALIFICATION.md)
   is now reapplied to selected ThinLTO as experimental `e748620`.
   It preserves the checksum polynomial and WAL bytes; it already has a
   source-bound Lean equivalence proof and ordinary recovery evidence on its
   historical base. Fresh exact-source checks now pass 47 distinct Lean theorem
   statements, 710 workspace tests/doctests (23 existing ignored), Clippy, a clean
   release and ordinary recovery with 363 complete operations and 26 unknowns.
   [Original qualification evidence](write-reference-qualification-v1/README.md)
   preserves the separate populations. Historical kernel timings are not a database speedup.
   Existing engine and Raft Ready group commit must not be reimplemented.
4. Next: run an initial selected-versus-CRC write screen with the same fixed
   native v3 client: point Put/BatchPut(64), c1/c64, two opposite orders, eight
   two-second smokes and sixteen ten-second timed cohorts. Reserve capacity
   before launch; retain original 96-GiB preflight and every storage cap/floor.
   Its exact-source Chaos image is built, probed and loaded; run and independently
   audit the actual candidate fault histories. Qualify useful
   improvements with full point/batch and mixed-read regression coverage, applicable exact
   source proofs, ordinary recovery and actual Chaos Mesh fault histories before
   default promotion. Preserve loaded batch-write p99 and fixed-rate client-drop
   limitations; an aggregate throughput gain alone is insufficient.
5. Continue with measured checksum, allocation, batching and replication costs.
   Use the retained profiles before collecting a necessary current-source profile;
   do not repeat rejected worker/transport sweeps. DPDK requires cross-host/NIC
   evidence. A real-disk panel must retain every sync and acknowledgment rule.
6. After the write phase, return to bounded dynamic multi-Raft (#22), epoch routing
   (#23), recoverable membership (#24), automatic splits (#25) and placement
   (#27), with their existing storage/recovery/proof prerequisites. Metadata and
   scheduling must be replicated or safely replaceable. Only object storage may
   be a service-critical singleton.

CI remains local. GitHub CI is reserved for releases or explicitly selected key
milestones. This client checkpoint closes no original industrial work package.
