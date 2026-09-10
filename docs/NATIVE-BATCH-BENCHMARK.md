# Native batch measurement client

`kv9-batch-benchmark` adds bounded measurements of native `batch_get` and
`batch_put` through the ordinary persistent SDK. Streaming gRPC is the default;
unary remains selectable and tarpc requires the existing experimental feature.
This tool does not change the server, consensus, storage or client retry path.

This checkpoint supplies a measurement tool checked against actual debug and
clean release processes, plus an independent aggregate validator. It supplies
**no accepted batch performance result**. Enclosing runtime/environment checks,
offered-load sweeps and paired Redis MGET/MSET measurements remain required. The separate
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
Cutoff completion totals must agree with the sequential workers' final calls.
Contained latency populations must obey cumulative-bucket order, and call
durations must fit their enclosing phase and worker-time envelopes. These are
necessary aggregate checks; they cannot recover individual sample pairings.
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

None of these debug runs is an accepted performance sample.

## Release process and validator review checkpoint

Standalone default-feature release client and server builds completed at clean
`53dcc9309ddad4e854c76f54dc5ce1307701e9e0`. Both use release optimization level 3;
server, client, engine and Raft feature arrays are empty. The retained build is
`/tmp/kv9-native-batch-release-build-first`, with manifest SHA-256
`1f79c6082fc84b99f5441b64010d2e86398f56475b19d0738e511e3af785c476`.
The client SHA-256 is
`d8abdb64695b8c7b003ca207dffb82a1cbbf83dbd0b9367b1e227f5ba236fa6d`;
the server SHA-256 is
`82cf715e6d8d1ea8898db1ad3624431958a21213c50ebc00ae5320943cc03991`.

Nine actual release-client cases completed against three ordinary WAL-backed
processes: batch sizes 1/4/16/64/128/256, a one-call operation cap, fewer offered
slots than workers, and loss/recovery of two voters. The final case retained
1,377 calls: 322 successful, 503 unknown writes, 511 read failures and 41
refusals. Every case completed final value/sentinel verification. All fourteen
owned client/server lifetimes exited. Reports and process identities remain in
`/tmp/kv9-native-batch-release-smoke-first`; summary SHA-256:
`75ba4a9e210431c17a3a9aa7b97e17dd4c1f6956d49708bb57dc9ea1083a896e`.

These runs used shared-host client CPUs 6-7 and server CPUs 8-31, with fixture
admission/flush settings. They check release execution and outcome accounting;
they are not isolated memory performance measurements, a Redis comparison,
new linearizability acceptance or Chaos Mesh evidence. The two-voter case used
process termination and original-directory restart.

Independent review of the original `53dcc93` validator found three acceptance
gaps: relabeled post-cutoff completions, impossible contained-latency
distributions, and durations/worker stops outside their enclosing stages.
The genuine reports are retained unchanged. The validator now checks worker-tail
conservation, distribution containment, attempt maxima, phase spans and total
available worker time. The original review and reproductions remain in
`/tmp/kv9-native-batch-benchmark-independent-review`.

Follow-up review supplied two additional controls with identical bucket counts
but impossible exact scheduled-latency extrema. Containment now checks both
bucket order and exact minima/maxima. Eleven Python tests passed. The corrected
validator revalidated all thirteen unchanged reports and rejected 783 copied
report corruptions, with each original restored and checked between controls:

- Four debug reports, 242 controls:
  `/tmp/kv9-native-batch-validator-final-debug-controls/summary.json`, SHA-256
  `992aa53494fba3c37821b836f98dbe51019eb3f0cbe4557e4ccd37a116244df1`.
- Nine release reports, 541 controls:
  `/tmp/kv9-native-batch-validator-final-release-controls/summary.json`, SHA-256
  `9d9b1fad7197434e58ceb648c12d7248f49abd0cc7f14538a8edaf9c357d29ff`.

An earlier invocation incorrectly paired a release run with the debug build
directory; it failed at the retained-manifest identity check. That failed
invocation remains recorded, and the two final groups use their matching builds.
This checkpoint changes validation and documentation only; it does not rebuild
or change the retained release client, SDK or database runtime.

The next measurement stage needs batch sizes 1/4/16/64/128/256 where byte limits
permit, concurrent-batch and offered-load sweeps, failure-inclusive whole-batch
latency curves and paired order-balanced Redis MGET/MSET controls, all under
independently checked runtime and isolation settings.
