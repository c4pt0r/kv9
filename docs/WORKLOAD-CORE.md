# Bounded workload components

The workload module supplies versioned configuration, deterministic data,
bounded statistics and a complete point-history recorder for #40. The executable,
build provenance, resource report, independent artifact checks and real MinIO
acceptance are described in [WORKLOAD-RUNNER.md](WORKLOAD-RUNNER.md). The persistent
client's 19-window Chaos Mesh gate is described in [PERSISTENT-CHAOS.md](PERSISTENT-CHAOS.md).
The reproducible measurement protocol and client calibration are described in
[BENCHMARKS.md](BENCHMARKS.md). These component tests are not benchmark acceptance.

## Configuration version 1

WorkloadConfig::read caps the configuration file at 64 KiB before deserialization,
rejects unknown fields and validates limits before channel/workload allocation.
Names are 1–64 ASCII letters, digits, underscores or hyphens. Tokens and artifact
paths are supplied separately. A fresh raw keyspace with a positive 24-bit ID
and epoch 1/1 is required; the workload cannot silently repair split epochs.

| Resource | Bound |
| --- | --- |
| Workers | 1–configured client concurrency, at most 256 |
| Mutable keys | 1–4,096, plus one immutable sentinel |
| Value bytes | 16–8,192 |
| Dataset | At most 8 MiB including conservative per-row overhead |
| Operations across all phases | At most 100,000 in correctness mode, 1,000,000 in performance mode |
| Warmup operations | At most 100,000; reserved before measured work |
| Measurement duration | 1 ms–1 hour |
| Per-worker interval | 0–1,000 ms; this is closed-loop pacing |
| Full correctness history | At most 64 MiB, with a conservative reservation for every configured operation |
| Performance-only history bytes | Exactly zero; complete operation accounting remains required |
| Fault/lifecycle phase labels | A fixed 19-entry vocabulary including every existing Chaos window |

The byte reservation further restricts the effective correctness-operation count.
Configuration must reserve initialization (absence check plus acknowledged write
for each key/sentinel, plus at most peers-minus-one additional sentinel reads),
warmup and up to one final read per configured peer for each key, leaving at least
one measured operation. The recorder also checks actual invocation/terminal byte sizes and
reserves completion space before an invocation is written. It stops before a
new RPC if history space cannot accommodate it; it never samples or truncates
while claiming a complete history.

The deterministic generator maps a seed and per-invocation nonce to a point
operation. It embeds the nonce in every generated value, keeps all mutable keys
inside the configured dataset and excludes the sentinel from random mutations.
Its specified integer mixer is not a cryptographic random source. The composing
runner must assign disjoint initialization and traffic nonce ranges and must
establish the fresh-keyspace and acknowledged-dataset assumptions before measuring.

## History and metrics

Recorder::begin writes a version-1 independent-checker invocation and returns a
non-clone ticket. Recorder::complete consumes the ticket, verifies the operation
and terminal outcome, and writes the completion. A stable recorder identity
survives moving the Rust owner. A mutex orders both event types and prevents
interleaved JSONL writes. Outstanding reservations are released individually;
one completion cannot consume another operation's reserved space.

Unknown writes and failed reads remain unknown in the history. Positive write
receipts, validated refusal reasons, attempt records and logical duration remain
in the observation fields. A local rejection with no attempts can prove refusal.
Protocol errors remain unknown outcomes but also carry a malformed marker, so
they cannot be passed off as accepted correctness evidence. An unexpected value
larger than the declared workload bound invalidates the run rather than being
silently truncated.

Finalization requires nonzero work, no outstanding tickets, equal invocation and
terminal counts and exactly two events per issued operation, followed by a
successful file flush/sync. The summary distinguishes accounting_complete from
full_history_complete. Performance mode can satisfy the former and never the
latter. Both modes leave independently_checked=false: only a separate checker
and witness replay can establish that result. Recorder timing reports time spent
in its calls, including lock waits and logging; it is not database latency.

Metrics use client-reported monotonic durations. Logical and attempt populations
are separate, and initialization/warmup/measurement/fault/verification rows each
have distinct histograms. Twelve fixed outcome-count categories retain success,
NotLeader, three admission reasons, two read-barrier phases, unmarked RPC status,
protocol error, deadline and local client refusals. Operation kind distinguishes
unknown writes from read failures. Fixed 17-code arrays additionally retain
unmarked gRPC status codes separately for logical operations and attempts;
these codes alone do not establish whether their origin was server or transport.
Errors never disappear from latency/count
populations. The histograms use the existing seven outcome groups and 65
logarithmic nanosecond buckets, returning percentile intervals and saturation
flags; a bucket boundary is not an exact percentile. The fixed 114 latency
objects are allocated once on the heap; snapshots allocate only bounded output.

## Independent response-loss control

    python3 scripts/check-client-history.py --output /tmp/kv9-client-history-new
    python3 scripts/check-workload-controls.py --output /tmp/kv9-workload-controls-new
    cargo test --locked -p kv9-server workload::

The first script runs an explicitly selected ignored Rust fixture against a real
gRPC listener. It preserves all four logical operations and all eight events:
v0 is made visible while its reply is held, a second client reads v0 and writes
v1, the original reply is lost, then the second client reads again. The recorder
and real client produce the history; the Python checker imports no kv9 code and
does not use the fixture's server effect counter to decide correctness.

The baseline and restored clients retain one unknown write and three successes;
both full histories have independently replayed legal witnesses. An isolated
source mutation that retries the unknown write must still compile and finish the
fixture, but its public history has no legal execution. Only an unrestricted,
exhausted search can accept this negative result; an inconclusive or restricted
failure is rejected. Raw histories, source hashes, compile/test logs, diagnostics
and checker witnesses remain in the output directory.

Two additional source controls omit terminal accounting or serialized terminal
records. Each must compile, select exactly one unit test, fail its named
behavioral assertion and pass after exact source restoration. Boundary tests
also cover byte exhaustion, maximum configured values, out-of-order completions,
dropped tickets, empty histories, seed/nonce reproducibility and phase-separated
latencies. None of these fixtures establishes a MinIO/Chaos throughput baseline.
