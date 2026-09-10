# Native batch measurement client

`kv9-batch-benchmark` adds bounded measurements of native `batch_get` and
`batch_put` through the ordinary persistent SDK. Streaming gRPC is the default;
unary remains selectable and tarpc requires the existing experimental feature.
This tool does not change the server, consensus, storage or client retry path.

This checkpoint supplies a development-tested measurement tool and independent
aggregate validator. It supplies **no accepted batch performance result**. Fresh
clean release builds, enclosing runtime/environment checks, offered-load sweeps
and paired Redis MGET/MSET measurements remain required. The separate
`kv9-batch-workload` and atomic history checker continue to own correctness
histories; aggregate measurements cannot replace them.

## Configuration and execution

Build outside the source tree using a clean committed checkout:

```sh
env CARGO_TARGET_DIR=/path/to/cargo-target PYTHONDONTWRITEBYTECODE=1 \
  taskset -c 6-31 python3 scripts/build-workload.py \
  --binary kv9-batch-benchmark --release --output /path/to/new-client-build
```

The configuration uses an existing Raw keyspace and three ordinary endpoints.
Replace the example addresses and keyspace ID with the actual test cluster.
The dataset prefix must be fresh, and no other writer may modify its keys.
Credentials come from `KV9_CLIENT_TOKEN` and are not included in the report.

```json
{
  "version": 1,
  "client": {
    "version": 1,
    "peers": [
      {"node_id": 1, "address": "127.0.0.1:20160"},
      {"node_id": 2, "address": "127.0.0.1:20161"},
      {"node_id": 3, "address": "127.0.0.1:20162"}
    ],
    "keyspace_id": 1,
    "epoch_conf_ver": 1,
    "epoch_version": 1,
    "max_in_flight": 16,
    "max_attempts": 6,
    "deadline_ms": 1500,
    "retry_backoff_ms": 5
  },
  "rpc_transport": "tonic_stream",
  "run_id": "batch_run_001",
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

For fixed offered load, use
`"load": {"kind": "fixed_rate", "batches_per_second": 10000}`.
The total number of slots due strictly before the cutoff must fit `max_calls`.
Each worker owns a strided set of slots, keeps at most one call outstanding,
and explicitly counts overdue slots it cannot send. It never builds an
unbounded queue or catches up by releasing all missed work as a burst.

```sh
taskset -c 0-1 /path/to/new-client-build/kv9-batch-benchmark \
  --config /path/to/config.json \
  --build-manifest /path/to/new-client-build/build.json \
  --output /path/to/new-run
```

The enclosing experiment must arrange server CPUs, WAL/durability mode, process
identity and exclusive timing. Do not overlap measurements with builds, tests,
profiling or Chaos. This command alone does not establish isolation.

## Bounds and measurement meaning

- Workers: 1 through the SDK's configured maximum, at most 256. Every worker
  issues one native batch at a time; the report records the configured upper
  bound on outstanding items and input payload bytes, not a measured peak RSS.
- Batch size: 1 through 256, no larger than the dataset's key count. Keys per
  dataset: 1 through 65,536. Values: 16 through 8,192 bytes. Exact encoded
  requests and present-value responses must fit the SDK's 1 MiB limit.
- Pending input payload: at most 64 MiB. Warmup: at most 100,000 calls.
  Measurement: at most 60 seconds and 10 million allocated calls. Fixed offered
  rate: at most two million batches per second, subject to the total call cap.
- Initialization checks that all dataset keys and a sentinel are absent, then
  writes deterministic values. Warmup is separate. Every successful measured
  read is checked for ordered key identity and complete value integrity.
  Final verification checks every key and the unchanged sentinel.
- One batch yields one logical latency sample. Whole-call time includes input
  construction, SDK execution and response-content validation. SDK call time
  and transport-attempt time are separate. Fixed-rate runs also record dispatch
  lateness and scheduled-to-completion latency, including delayed starts.
- Success, refusal, unknown write, read failure and client rejection retain
  separate logical populations. Unknown writes keep their whole input-item
  count and are not automatically replayed. Attempt success/refusal/failure
  populations and typed reasons remain separate from logical outcomes.
- The throughput denominator extends from measurement start through the later
  of its cutoff and final measured completion. Setup, warmup and verification
  are excluded. Work finishing after the cutoff remains counted and its latency
  stays visible. No latency is divided by batch size.
- Histograms retain integer nanosecond counts, sum and extrema with 64
  subdivisions per power of two. p50/p95/p99 are bucket intervals, not invented
  exact values. Overflow invalidates summaries instead of silently wrapping.

Closed-loop operation-cap stops are explicitly labeled and are not eligible as
duration-based timing runs. Dirty debug builds are allowed for development and
always report `timing_eligible=false`; dirty release builds are rejected.
The client flag indicates only its local prerequisites, never acceptance of
environmental isolation or a performance comparison.

## Independent validation

Run validation after the timed process has exited:

```sh
PYTHONDONTWRITEBYTECODE=1 taskset -c 6-31 python3 \
  scripts/batch_benchmark_report.py \
  --directory /path/to/new-run --build-directory /path/to/new-client-build \
  --expected-config /path/to/config.json --expected-revision CLEAN_COMMIT_SHA \
  --require-timing --output /path/to/new-validation.json
```

For honest dirty/debug development checks, omit `--expected-revision` and
`--require-timing`. Every output path must be new. The validator reads bounded
JSON, rejects duplicate fields and boolean/integer substitutions, independently
calculates protobuf sizes, histogram sums/quantiles, phase and worker counts,
offered/issued/dropped populations, terminal drain spans and throughput rates.
It also checks aggregate retry conservation: only nonterminal NotLeader
refusals may add attempts. These aggregate identities are necessary checks,
not a reconstruction of each call's retry history.

Run/build manifests, retained executable hashes, source inventories, standalone
Cargo artifacts and dependency features are bound together. The validator does
not prove source-to-binary compilation, actual server identity, linearizability,
durability or host isolation; those remain enclosing acceptance requirements.

Local controls:

```sh
PYTHONDONTWRITEBYTECODE=1 taskset -c 6-31 python3 -m unittest discover \
  -s scripts -p test_batch_benchmark_report.py
PYTHONDONTWRITEBYTECODE=1 taskset -c 6-31 python3 \
  scripts/check-batch-benchmark-controls.py \
  --directory /path/to/retained-run --build-directory /path/to/retained-build \
  --output /path/to/new-control-results
```

## Development evidence

The retained dirty debug client SHA-256 is
`2a3552a7bd1102f7e37b490955e1a28791f8c0979902447dd14b4eeb035ef1f9`.
It ran against the unchanged standalone e246 server, SHA-256
`f7e88b6c2f2514748c13fbaf85c3ee0540284138b3f3cc6bd774bdfd17e7eb4e`,
using three local WAL voters. This deliberately does not claim a common clean
release revision. Exact inventories and original debug reports are retained in
`/tmp/kv9-native-batch-benchmark-build-dev1` and
`/tmp/kv9-native-batch-benchmark-smoke-dev1`.

Seven Rust unit tests and targeted warnings-denied Clippy passed. Four real-RPC
development cases passed: closed-loop GET with 4 items, PUT with 256 items,
fixed-rate mixed traffic with 16 items, and deliberate client scheduler shedding.
The final case records missed offered slots; it is not database-overload evidence.
All seven owned server/client processes exited. Summary SHA-256:
`510fb8647540f45e9f50394898d361d1dc0c33de5d17d20ba22a851db8f878d3`.

The independent validator passed nine arithmetic/failure tests and revalidated
all four original reports. It rejected 232 copied-report corruptions, restoring
the original report after each case and rechecking unchanged input hashes.
Control summary SHA-256:
`bb99309bfc92c13520d9c8a758e00d7d46594a4fe665e3f3556672cb633f2efb`.
The original early test-build type-inference and Clippy clone-on-copy failures
remain in their development logs; both were corrected before the passing gates.

None of these debug runs is an accepted performance sample. The next measurement
stage needs clean release builds, batch sizes 1/4/16/64/128/256 where byte limits
permit, concurrent-batch and offered-load sweeps, failure-inclusive whole-batch
latency curves and paired order-balanced Redis MGET/MSET controls.
