# Local Redis reference comparison

This bounded fixture compares a standalone Redis memory reference with three
WAL-backed KV9 voters. It measures the current Raw KV hot path without changing
KV9 acknowledgment, WAL, quorum, or read-barrier behavior. It is not a comparison
with equal durability or equal failure domains.

Redis starts on an owned ephemeral IPv4 loopback port with `save ""`,
`appendonly no`, no replicas, and one I/O thread. The fixture records both the
configuration file and live `CONFIG GET` / `INFO replication` observations.
Redis persistence settings are described in the [official persistence guide](https://redis.io/docs/latest/operate/oss_and_stack/management/persistence/).
Redis `appendfsync always` would add local AOF durability; it would still not
equal KV9's quorum semantics. Redis replication is asynchronous by default,
and `WAIT` does not establish a strongly consistent CP system, as explained in
the [official replication guide](https://redis.io/docs/latest/operate/oss_and_stack/management/replication/).
No mode in this fixture is labeled a durability-matched Redis comparison.

The normal protocol has 64 mutable keys, one reserved sentinel, 128-byte values,
and fixed-width 23-byte keys. Both clients use the existing KV9 deterministic
key, operation, and value generator. Each trial initializes the entire hot set,
then performs 32 warmup operations before timing. Measured mixes are GET 100%,
PUT/SET 100%, and GET 50% / PUT/SET 40% / DELETE/DEL 10%. Missing reads are
successful operations. The mixed workload can contain missing keys; Redis
records its observed missing-read count, while the current KV9 performance
report does not export that count. A Redis read-only trial rejects an unexpected
missing key. All generated Redis GET payloads are checked for length and value
integrity.

Concurrency is the number of outstanding logical calls, with one request per
worker. Redis uses one persistent TCP connection per worker and no pipelining.
KV9 uses its existing persistent HTTP/2 client. Both have two Tokio runtime
threads. Neither retries an uncertain write; KV9's existing client may follow
an explicit NotLeader refusal. The default concurrency sweep is 1, 4, 16, 32,
and 64, which stays within the fixture's 64-request public admission limit.
Each mix/concurrency pair has two three-second repetitions. The second
repetition reverses concurrency and target ordering to expose simple order
drift. Finite operation budgets bound every run; hitting a budget invalidates
the timed trial instead of silently shortening its measurement.

Client/server affinity is separated (the first two allowed CPUs for clients,
the next four for servers). This is still a shared-host loopback experiment.
It does not model inter-host RTT, independent storage failures, or open-loop
arrival rates. Process and per-thread CPU counters are sampled every 50 ms;
reported CPU utilization uses samples inside the measured cohort. This makes
client saturation visible. Latency reports retain 65 power-of-two nanosecond
buckets and report percentile *intervals*, not invented exact percentiles.
Initialization, warmup, and final verification are outside measured throughput.
The numerator is successful operations; the denominator includes drain time
for calls issued before the timed deadline. Failures and attempted RPC counts
remain visible separately.

An unsuccessful write is not necessarily a failed write. The derived analysis
separates UnknownWrite from pre-execution admission refusal, NotLeader refusal,
read failure, and client rejection. A timed-out caller can leave its server job
and admission reservation alive. The retained after-drain / after-verification
status snapshots include queued, running, and in-flight backend counts. The
fixture is reused across trials, so residual work can affect later trials;
client completion alone must not be interpreted as server quiescence.
Under overload, new logical operations can receive fast admission refusals
without retrying prior uncertain writes. Report acknowledgment throughput and
successful-operation latency separately: aggregate terminal latency can be
dominated by these fast refusals.

## Build and run

Build the database from a clean worktree at the intended exact revision. Keep
the benchmark driver's additions in a separate worktree so their untracked
files cannot change the database's recorded source identity.

```sh
CARGO_TARGET_DIR=/tmp/kv9-reference-db-target \
  python3 scripts/build-benchmark.py --output /tmp/kv9-reference-db-build

# Run this in the worktree containing the new reference fixture.
CARGO_TARGET_DIR=/tmp/kv9-reference-client-target \
  cargo build --release --locked --manifest-path scripts/redis-reference/Cargo.toml

python3 scripts/redis-comparison.py \
  --build /tmp/kv9-reference-db-build \
  --redis-client /tmp/kv9-reference-client-target/release/kv9-redis-reference \
  --expected-revision FULL_COMMIT_SHA \
  --output /tmp/kv9-reference-attempt-1

python3 scripts/check-redis-comparison.py \
  --build /tmp/kv9-reference-db-build \
  --matrix /tmp/kv9-reference-attempt-1 \
  --expected-revision FULL_COMMIT_SHA \
  --output /tmp/kv9-reference-attempt-1-check.json

python3 scripts/analyze-redis-comparison.py \
  --matrix /tmp/kv9-reference-attempt-1 \
  --output /tmp/kv9-reference-attempt-1-outcomes.json
```

Every output directory must be fresh. Failed attempts remain on disk; process
cleanup only affects this invocation's owned child processes. The normal run
uses locally installed `redis-server` and `redis-cli`, and records their server
version and executable hash. Docker is queried only for host inventory.
Nothing invokes hosted CI or publishes GitHub state.

For a smoke run, explicitly override `--concurrency 4 --repetitions 1
--measure-ms 1000 --keys 8`. Such output is a functional fixture check, not a
saturation baseline. For A/B comparisons, keep the exact driver binary and
protocol unchanged, build both database revisions separately, and retain
both complete matrices. Report the Redis reference separately from the
KV9 scheduling comparison.

After independently checking both matrices, render their paired report:

```sh
python3 scripts/report-redis-comparison.py \
  --baseline /tmp/kv9-reference-baseline \
  --candidate /tmp/kv9-reference-candidate \
  --baseline-check /tmp/kv9-reference-baseline-check.json \
  --candidate-check /tmp/kv9-reference-candidate-check.json \
  --output /tmp/kv9-reference-paired
```

The renderer requires identical protocol and driver-source hashes, preserves
acknowledgment and refusal populations separately, and includes a scoped
before/after durable-write stage sample. When concurrency 16 is present, it
also compares that concurrent snapshot interval between the two revisions.
Use `--candidate-status` to retain explicit pending correctness gates for
an unaccepted optimization. The first retained result is in
[results/f2c1-4201d04.md](results/f2c1-4201d04.md).
The subsequent combined outbound/completion-notification result is in
[results/4201d04-b4a74b2.md](results/4201d04-b4a74b2.md).
The isolated accepted-socket TCP_NODELAY result is in
[results/b4a74b2-cc8bc87.md](results/b4a74b2-cc8bc87.md).
The append-slice synchronization candidate's performance evidence is in
[results/cc8bc87-b3cbc35.md](results/cc8bc87-b3cbc35.md); its correctness
acceptance gates are tracked separately from these measurements.

The bounded Raw engine group-commit candidate's performance evidence is in
[results/b3cbc35-fe650ed.md](results/b3cbc35-fe650ed.md), including both
high-concurrency repetitions and scoped engine synchronization counts.
Correctness acceptance remains separate from this report.

The Raft Ready group-sync candidate's disk performance evidence is in
[results/fe650ed-892b2a1.md](results/fe650ed-892b2a1.md). It preserves both
repetitions and whole-trial Raft/engine sync counts. Volatile storage diagnostics
are separate from this durable matrix and its acceptance gates.

## Volatile tmpfs diagnostic

A separate bounded wrapper can distinguish storage waiting from protocol/CPU
cost without changing the production binary or disabling its sync calls. This
is **diagnostic-only volatile storage with no disk durability or power-loss
guarantee**. Never combine its numbers with the durable paired report or
production acceptance. It requires at least 32 GiB free in `/dev/shm`, retains
all original artifacts, and checks an 8 GiB free-space floor between trials.

```sh
python3 scripts/tmpfs-redis-diagnostic.py \
  --build /tmp/kv9-reference-db-build \
  --redis-client /tmp/kv9-reference-client-target/release/kv9-redis-reference \
  --expected-revision FULL_COMMIT_SHA \
  --output /tmp/kv9-tmpfs-diagnostic-attempt-1

python3 scripts/check-tmpfs-redis-diagnostic.py \
  --build /tmp/kv9-reference-db-build \
  --matrix /tmp/kv9-tmpfs-diagnostic-attempt-1 \
  --expected-revision FULL_COMMIT_SHA \
  --output /tmp/kv9-tmpfs-diagnostic-check-1.json
```

The fixed diagnostic is 24 trials: concurrency 1/64, all three mixes, two
3-second repetitions, KV9 and standalone Redis. It observes every running
voter's actual tmpfs mount and executable identity. After stopping children,
it copies and verifies original data hashes before removing only its owned
tmpfs directory. Retention failures preserve scratch for diagnosis. A focused
auditor rechecks complete reports and storage/copy evidence, including four
invalid-evidence controls, without modifying the shared durable validators.
The retained [Ready diagnostic](results/892b2a1-tmpfs-diagnostic.md) reports
repetition spread, CPU observations and the limits of the storage comparison.

The indexed read-receipt candidate's unchanged disk matrix is in
[results/892b2a1-73ddb0d.md](results/892b2a1-73ddb0d.md). It did not establish
an end-to-end read-throughput gain; all decreases, reference drift, memory
tradeoffs and repetition data remain visible.
