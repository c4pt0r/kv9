# Native Redis batch reference

`kv9-redis-batch-reference` issues RESP2 MGET/MSET through a persistent TCP
connection per worker, with one whole batch outstanding on each connection.
It is a single-instance Redis reference for native KV9 batches. Redis and KV9
retain their distinct replication and durability semantics. Existing point
reference source, configuration and reports remain separate.

Redis documents [MGET](https://redis.io/docs/latest/commands/mget/) as an atomic
multi-key read and [MSET](https://redis.io/docs/latest/commands/mset/) as an atomic
multi-key write. Framing follows the official
[RESP2 specification](https://redis.io/docs/latest/develop/reference/protocol-spec/).
This client neither pipelines individual GET/SET commands nor adds MULTI/EXEC.
It does not claim a Redis acknowledgement is a Raft commit receipt.

## Comparable workload and timing

Both batch clients compile the same deterministic key/value generator, operation
selection, fixed-slot arithmetic and 64-subdivision histogram implementation in
`crates/server/src/bin/kv9-batch-benchmark/common.rs`. Key strings, value key-index
and nonce headers, complete value bodies and ordered batch key positions match
for the same configuration and logical slot. Shared source reduces drift;
independent Python arithmetic and actual process checks remain required.

Choose identical run ID, seed, key/value counts and sizes, batch size, workers,
read percentage, warmup, duration, call cap, offered-load mode and call deadline.
`redis_batch_report.paired_configuration` checks these fields and both clients'
input limits. The configured worker count bounds concurrent batches; multiply by
batch size for the input-item bound. Redis uses up to one preconnected socket per
worker plus its serial setup/verification connection. KV9 retains its separately
reported streaming connection and admission limits.

Whole-call latency starts before constructing batch keys/values and ends after
validating the complete reply. `client_call` covers connection establishment when
needed, RESP encoding, request write and response parsing. Fixed-rate runs also
retain dispatch lateness and scheduled-to-completion latency. Each worker drops
and counts overdue slots instead of building a queue or issuing a catch-up burst.
Every measured terminal call is counted, including completions after cutoff;
throughput extends through the final measured completion. Latency is never divided
by the number of keys in a batch.

The measurement outcomes are success, unknown write and read failure. Reasons
separately identify deadline, I/O, Redis error reply, malformed protocol and data
integrity failures. No command is retried. A failed connection is discarded, and
only a subsequent new logical call may reconnect within its own deadline.
A lost MSET reply remains a whole-batch unknown. Redis error replies are treated
conservatively as failures; no unproved zero-effect refusal semantics is inferred.
Malformed framing or invalid data cancels new measurement work and drains the
other workers, producing an incomplete report. Network and Redis error outcomes
remain visible while subsequent new logical calls and final verification proceed.

Initialization requires an empty dataset prefix, checks and writes every key plus
a sentinel, and warmup must succeed. Final verification reads every key and the
unchanged sentinel. These checks establish bounded value integrity, not a complete
linearizability history. Use a fresh Redis instance and no other writer.

## Build and run

The executable uses the standalone `scripts/redis-reference/Cargo.toml` package.
The retained build helper inventories repository sources, records standalone
Cargo output and hashes the actual executable. Dirty debug builds are allowed
for development; dirty release builds are refused by the client.

```sh
env CARGO_TARGET_DIR=/path/to/target PYTHONDONTWRITEBYTECODE=1 \
  taskset -c 6-31 python3 scripts/build-workload.py \
  --binary kv9-redis-batch-reference --release --output /path/to/new-build
```

Example configuration for an already running, fresh loopback Redis instance:

```json
{
  "version": 1,
  "address": "127.0.0.1:6379",
  "deadline_ms": 1500,
  "run_id": "paired_batch_001",
  "seed": 71,
  "workers": 16,
  "keys": 4096,
  "batch_size": 16,
  "value_bytes": 128,
  "read_percent": 50,
  "warmup_calls": 1000,
  "measure_ms": 10000,
  "max_calls": 1000000,
  "load": {"kind": "closed_loop"}
}
```

For fixed offered load use `{"kind":"fixed_rate","batches_per_second":10000}`.
Workers are bounded at 256, keys at 65,536, batches at 256 items, values at 8,192
bytes, pending input at 64 MiB, encoded request/response at 1 MiB, measurement at
60 seconds/10 million calls and offered rate at two million batches/second.
Both Redis and KV9 encoded limits must admit every paired configuration.

```sh
taskset -c 0-1 /path/to/new-build/kv9-redis-batch-reference \
  --config /path/to/config.json --build-manifest /path/to/new-build/build.json \
  --output /path/to/new-run

PYTHONDONTWRITEBYTECODE=1 taskset -c 6-31 python3 scripts/redis_batch_report.py \
  --directory /path/to/new-run --build-directory /path/to/new-build \
  --expected-config /path/to/config.json --expected-revision CLEAN_COMMIT_SHA \
  --require-timing --output /path/to/new-validation.json
```

Run the validator after measurement. Omit timing/revision requirements for honest
debug development checks. It checks exact field types and input/build identities,
encoded sizes, all outcome/item/reason/command/connection populations, histogram
arithmetic and containment, per-worker offered-slot and cutoff conservation,
phase/worker time spans and final drain denominators. Native and Redis validators
share transport-neutral cohort arithmetic; neither fabricates the other's RPC,
retry or acknowledgement semantics.

The enclosing fixture must independently record Redis version/configuration,
server/client process identity, CPU placement, connection counts and resource
usage; KV9's corresponding voter/WAL/durability settings must also be explicit.
Use repeated order-balanced pairs and offered-load/latency curves. No timing may
overlap builds, tests, profiling or Chaos. `timing_eligible` only indicates client
prerequisites and never certifies environment isolation or a valid comparison.
No accepted Redis/KV9 batch performance result is supplied by this implementation.

## Development acceptance

Six Redis Rust tests, seven native-client regression tests and targeted
warnings-denied Clippy passed. Thirty Python batch-report tests passed, including
the seven Redis-specific configuration/accounting examples. Shared validator
extraction revalidated all thirteen original native reports and rejected all
783 existing corruptions. Debug/release control summary SHA-256 values are
`a6a266c805739254bbbff77934b80249c9de36482b1f5dbf0bf676d2d6d93c39` and
`a9286783e56669e126d1552068eb115540f3cd1b0ebf6e4c9795ad3b249fcf0d`.

The retained dirty debug build in `/tmp/kv9-redis-batch-reference-build-dev1`
has executable SHA-256
`4dc511149d70c89b27a2ba62ce6107b9059686c143fd0ec46ef1c383a4f1faa7`.
Its manifest inventories the exact uncommitted Rust sources based on `03f1ae5`;
it is not a clean release artifact. Later Python/docs additions do not relabel
that earlier build inventory.

The nineteen process cases in
`/tmp/kv9-redis-batch-reference-correctness-attempt1` passed their intended
predicates: twelve fresh Redis cases and seven controlled RESP peer cases.
They cover batch sizes 1/4/16/64/128/256, read/write/mixed loads, fixed-rate
shedding, cap/idle-worker boundaries, fragmented replies, malformed count/length,
server errors and consumed MSET commands followed by EOF or timeout. Seventeen
client runs exited 0 with complete reports. Wrong-count and oversized-length
cases intentionally exited 1 with incomplete reports after one protocol failure
and no subsequent measured dispatch. The strict complete-report validator rejects
both. All thirty-eight owned client/backend lifetimes exited.

These reports retain 230,614 calls and 2,685,081 input items, including three
unknown writes and three read failures. An independent readback decoded all 118
original controlled-peer request frames and replayed ordered MSET mutations into
a separate model. Consumed EOF/timeout cases prove the mutation preceded reply
loss, followed by a new nonce on a new connection, with no replay of the uncertain
batch. Final dataset values and unchanged sentinels were checked independently.
Process summary SHA-256:
`b23fb40c8d256734b9503cf8f28213d782fc54524307acf5660a3dd3ad671404`.
Independent audit SHA-256:
`b3e8a70db737c92bc74181afccf4b30f635eafa887357b3fbbfa9a3ff4422a63`.

All seventeen complete reports were revalidated unchanged, and 696 copied-report
corruptions were rejected with restored originals checked between cases.
Control summary `/tmp/kv9-redis-batch-report-controls-first/summary.json` SHA-256:
`b7f4d7f3128967fe9530ed6f4fb84713cd28804aa94827b170eae6a914262889`.
These are shared-host development correctness/accounting checks, not timing
samples or Raft/durability equivalence. Their original logs and helper preparation
correction remain retained. No hosted CI was dispatched.

Local validation commands:

```sh
PYTHONDONTWRITEBYTECODE=1 taskset -c 6-31 python3 -m unittest discover \
  -s scripts -p 'test_*batch*report.py'
PYTHONDONTWRITEBYTECODE=1 taskset -c 6-31 python3 scripts/check-redis-batch-controls.py \
  --directory /path/to/retained-run --build-directory /path/to/retained-build \
  --output /path/to/new-controls
```
