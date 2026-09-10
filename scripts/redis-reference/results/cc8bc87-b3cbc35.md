# Redis reference and KV9 scheduling comparison

Baseline: `cc8bc87b6b34d07c76eb2019651aff7574f5fea1`. Candidate: `b3cbc35fc410ed2770ee45a5f5ea5a5c84c3f566`.

The runtime was an unaccepted candidate when this measurement began. Proof refinement, omitted-sync/premature-publication source controls, and exact process/Chaos gates are separate acceptance requirements; this report establishes performance evidence only.

The complete baseline (60 trials) and candidate (60 trials) matrices passed independent report rechecking. Measurement completion does not mean that every logical call was acknowledged.

This is a standalone Redis memory reference (`save ""`, `appendonly no`, zero replicas) alongside three WAL-backed KV9 voters with unchanged quorum semantics. It is not an equal-durability comparison.

The paired protocol uses 64 hot keys, 23-byte keys, 128-byte values, 2 repetitions, 3000-ms measurement windows, two client runtime threads, and logical concurrency [1, 4, 16, 32, 64]. Client CPUs: [0, 1]; server CPUs: [2, 3, 4, 5]. Host and filesystem inventories are retained in each matrix. Logical CPU affinity is not exclusive physical-core or host isolation. Every trial retains its full latency histograms, CPU samples, outcomes, and server snapshots.

Rates below pool acknowledged operations over the sum of both measured cohort durations, including drain. The ratio is descriptive; overloaded rows must be read with their non-acknowledgment counts.

| Mix | Concurrency | Baseline KV9 ops/s | Candidate KV9 ops/s | Ratio | Baseline non-acknowledged | Candidate non-acknowledged | Redis reference ops/s (baseline / candidate run) |
|---|---:|---:|---:|---:|---:|---:|---:|
| read | 1 | 15145.6 | 15346.1 | 1.01x | 0 | 0 | 176967 / 176749 |
| read | 4 | 39217.5 | 38815.0 | 0.99x | 0 | 0 | 459625 / 459606 |
| read | 16 | 66794.4 | 65999.9 | 0.99x | 0 | 0 | 495448 / 503605 |
| read | 32 | 77434.9 | 76930.3 | 0.99x | 0 | 0 | 509164 / 507505 |
| read | 64 | 85437.6 | 85143.5 | 1.00x | 0 | 0 | 508990 / 507486 |
| write | 1 | 33.1 | 33.2 | 1.00x | 0 | 0 | 176939 / 177251 |
| write | 4 | 40.0 | 49.3 | 1.23x | 0 | 0 | 452072 / 443880 |
| write | 16 | 48.0 | 80.1 | 1.67x | 0 | 0 | 463233 / 485847 |
| write | 32 | 49.0 | 88.6 | 1.81x | 0 | 0 | 490367 / 488859 |
| write | 64 | 42.0 | 93.9 | 2.24x | 213823 | 0 | 492856 / 490355 |
| mixed | 1 | 65.5 | 66.4 | 1.01x | 0 | 0 | 176570 / 175092 |
| mixed | 4 | 75.0 | 81.6 | 1.09x | 0 | 0 | 456029 / 455347 |
| mixed | 16 | 89.6 | 128.5 | 1.43x | 0 | 0 | 496763 / 494935 |
| mixed | 32 | 93.4 | 154.4 | 1.65x | 0 | 0 | 500227 / 499461 |
| mixed | 64 | 101.2 | 174.4 | 1.72x | 0 | 0 | 500700 / 489362 |

## Outcome certainty and residual work

| Revision | Trial | Acknowledged | UnknownWrite | Admission refused | NotLeader refused | Read failure | Client rejected | Backend jobs after verification |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| baseline | b00641 | 167 | 40 | 122725 | 0 | 0 | 0 | 0 |
| baseline | b01641 | 173 | 48 | 91010 | 0 | 0 | 0 | 0 |

UnknownWrite means the client did not establish the outcome; it is not proof that the write failed. Configured client concurrency does not bound outstanding backend jobs after caller cancellation. After snapshots are captured after client drain and final verification; they are not atomic across nodes. Trials use the same persistent KV9 fixture, so retained background work can affect subsequent trials. A fast-refusal population can dominate aggregate terminal latency; acknowledged latency must be reported separately.

Workers can issue new logical operations after fast explicit refusals. They never retry a prior uncertain write. Where refusals dominate, aggregate terminal percentiles largely describe refusals; the following percentiles include acknowledged calls only. Values are histogram intervals, not exact percentile estimates.

## Acknowledged latency at concurrency 64

| Mix | Baseline mean ms | Candidate mean ms | Baseline p99 interval ms | Candidate p99 interval ms |
|---|---:|---:|---|---|
| read | 0.748 | 0.751 | 1.049–2.097 | 1.049–2.097 |
| write | 999.525 | 669.133 | 1073.742–2147.484 | 536.871–1073.742 |
| mixed | 607.958 | 358.615 | 536.871–1073.742 | 268.435–536.871 |

## Durable-write stage sample

Candidate pure-write trial `b00011` has 197 acknowledged writes including initialization and warmup, and 0 unsuccessful measured calls. Before/after exporter snapshots encompass initialization, warmup, measurement, drain, and final reads; stage means are not isolated measured-only averages and counters are independent between metrics.

| Voter | Raft WAL sync calls | Mean Raft sync ms | Engine WAL sync calls | Mean engine sync ms |
|---|---:|---:|---:|---:|
| 1 | 394 | 7.225 | 197 | 8.318 |
| 2 | 394 | 7.031 | 197 | 5.025 |
| 3 | 394 | 7.436 | 197 | 8.213 |

In this sample, the recorded sync counts expose the durable I/O cost per acknowledged write. They are observations of this snapshot interval, not a universal fixed-count claim across elections, migrations, or future group-commit implementations.

## Concurrent durable-write sample

The following first-repetition concurrency-16 snapshot deltas include initialization, warmup, measurement, drain, and final reads. Sync/acknowledgment ratios summarize the entire interval; they are not isolated measured-phase stage averages. Engine sync counts remain visible independently of Raft sync counts.

| Revision | Voter | Acknowledged writes including setup/warmup | Non-acknowledged measured | Raft sync calls | Engine sync calls | Raft syncs / acknowledgment |
|---|---|---:|---:|---:|---:|---:|
| baseline | 1 | 252 | 0 | 377 | 252 | 1.496 |
| baseline | 2 | 252 | 0 | 378 | 252 | 1.500 |
| baseline | 3 | 252 | 0 | 376 | 252 | 1.492 |
| candidate | 1 | 353 | 0 | 264 | 353 | 0.748 |
| candidate | 2 | 353 | 0 | 267 | 353 | 0.756 |
| candidate | 3 | 353 | 0 | 264 | 353 | 0.748 |


## Retained evidence

- baseline: `/tmp/kv9-redis-comparison-cc8-attempt1`; matrix SHA-256 `6a3a8b7a1f94fd444f138bec4f098f424b07ad50ce730d2247499dd0a3df7192`.
- baseline independent recheck: `/tmp/kv9-redis-comparison-cc8-check1.json`; SHA-256 `a09e11e2a25037e1ee4453400159a8b30edb90f920023c7b8ce9db488846ba7f`.
- baseline release database SHA-256: `362d5401fa03878cc6e7a52820cba2a8d968193338765ea122d14ccf4d46b9fc`; source tree SHA-256 `b2bb5aa3d5f610bc9b34a08a37f5afebb07af4daff30c197747193b3320c4939`.
- candidate: `/tmp/kv9-redis-comparison-b3-attempt1`; matrix SHA-256 `fa2e43e1fa4b9cbbb22c91a053af9e371543f0fa909a185916a3e5e21fe15090`.
- candidate independent recheck: `/tmp/kv9-redis-comparison-b3-check1.json`; SHA-256 `a8c35ad929e65611c14313ac3ed8fe033661d6e102ec95300efd215ac23327c1`.
- candidate release database SHA-256: `5cb8a4469262015e461bb62e342159d4f7d6a30407e4aa9506a7a74501fc94e9`; source tree SHA-256 `c019316049bb35cafd36b21c8ebcc5fb3b10bdc7cf8063b67e6e2714a38a8169`.
- Redis: Redis server v=7.0.15 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=e53ff17674aa6190; server SHA-256 `1b2950213684d355e0e49fe6702c7bfb9d40b78f0da9ce289fc0e8f2a295e540`.
- Redis reference client SHA-256: `57947d5a94c739dd58ca98e2c9a85c40814040826f13f99e85cd0e2c7fb2f636`.

The full paired artifact is retained at `/tmp/kv9-redis-paired-cc8-b3-final/paired.json` (SHA-256 `7969c1bbded7d85ddf2394455bd08cf1f3f629afd9eae10ed2a4c843364a8b2c`). It includes per-repetition CPU and throughput, pooled successful latency histograms, exact certainty families, and per-node backend occupancy. The local fixture, native Redis client, independent rechecker, outcome analyzer, and paired renderer are versioned together. Hosted CI was not invoked.
