# Redis reference and Raw engine group-commit comparison

Baseline: `b3cbc35fc410ed2770ee45a5f5ea5a5c84c3f566`. Candidate: `fe650ed814757e8172fb03eced102894e35d4a4d`.

The runtime was an unaccepted candidate when this measurement began. This report establishes performance evidence only. Parameterized Raw composition proof, source mutation controls and exact MinIO/Chaos fault acceptance remain separate gates; no production acceptance or roadmap completion is claimed.

The complete baseline (60 trials) and candidate (60 trials) matrices passed independent report rechecking. Both matrices had zero non-acknowledged measured calls and zero retained backend jobs after verification. The candidate completed its first measurement attempt; no trial was discarded or repeated.

This is a standalone Redis memory reference (`save ""`, `appendonly no`, zero replicas) alongside three WAL-backed KV9 voters with unchanged quorum semantics. It is not an equal-durability comparison.

The paired protocol uses 64 hot keys, 23-byte keys, 128-byte values, 2 repetitions, 3000-ms measurement windows, two client runtime threads, and logical concurrency [1, 4, 16, 32, 64]. Client CPUs: [0, 1]; server CPUs: [2, 3, 4, 5]. Host and filesystem inventories are retained in each matrix. Logical CPU affinity is not exclusive physical-core or host isolation. Every trial retains its full latency histograms, CPU samples, outcomes, and server snapshots.

Rates below pool acknowledged operations over the sum of both measured cohort durations, including drain. The ratio is descriptive; overloaded rows must be read with their non-acknowledgment counts.

At concurrency 64, pooled confirmed write throughput rose from 93.9 to 734.4 operations/s (7.82x), and mixed throughput rose from 174.4 to 519.6 operations/s (2.98x). Candidate pure-write repetitions were 957.8 and 513.9 operations/s; that substantial spread is retained, rather than reporting the first peak alone. Mixed repetitions were 511.0 and 528.4 operations/s. Single-call write throughput remained about 33 operations/s.

Read throughput was 2–3% lower across most rows (83.4k operations/s at concurrency 64), while the paired Redis reference also varied between runs. Two short repetitions on a shared host do not establish the cause or significance of those small differences. The b3 matrix was collected earlier; this is a frozen-protocol comparison, not randomized interleaving of database revisions or an isolated sustained-capacity estimate. Redis remains a separate memory reference, around 493k reads/s in this candidate run.

| Mix | Concurrency | Baseline KV9 ops/s | Candidate KV9 ops/s | Ratio | Baseline non-acknowledged | Candidate non-acknowledged | Redis reference ops/s (baseline / candidate run) |
|---|---:|---:|---:|---:|---:|---:|---:|
| read | 1 | 15346.1 | 15111.4 | 0.98x | 0 | 0 | 176749 / 170787 |
| read | 4 | 38815.0 | 37819.4 | 0.97x | 0 | 0 | 459606 / 440925 |
| read | 16 | 65999.9 | 63774.9 | 0.97x | 0 | 0 | 503605 / 487596 |
| read | 32 | 76930.3 | 75952.8 | 0.99x | 0 | 0 | 507505 / 482138 |
| read | 64 | 85143.5 | 83384.3 | 0.98x | 0 | 0 | 507486 / 493146 |
| write | 1 | 33.2 | 32.7 | 0.98x | 0 | 0 | 177251 / 175120 |
| write | 4 | 49.3 | 65.6 | 1.33x | 0 | 0 | 443880 / 431242 |
| write | 16 | 80.1 | 168.3 | 2.10x | 0 | 0 | 485847 / 470175 |
| write | 32 | 88.6 | 301.4 | 3.40x | 0 | 0 | 488859 / 482167 |
| write | 64 | 93.9 | 734.4 | 7.82x | 0 | 0 | 490355 / 477393 |
| mixed | 1 | 66.4 | 65.3 | 0.98x | 0 | 0 | 175092 / 172145 |
| mixed | 4 | 81.6 | 89.8 | 1.10x | 0 | 0 | 455347 / 425607 |
| mixed | 16 | 128.5 | 195.9 | 1.52x | 0 | 0 | 494935 / 475114 |
| mixed | 32 | 154.4 | 282.7 | 1.83x | 0 | 0 | 499461 / 487978 |
| mixed | 64 | 174.4 | 519.6 | 2.98x | 0 | 0 | 489362 / 494300 |

## Outcome certainty and residual work

Both revisions recorded zero UnknownWrite, admission refusal, NotLeader refusal, read failure, or client rejection in measured calls. Every retained after-verification snapshot had zero queued, running, and in-flight backend jobs. These snapshots are asynchronous observations, not a stronger atomic quiescence guarantee.

UnknownWrite means the client did not establish the outcome; it is not proof that the write failed. Configured client concurrency does not bound outstanding backend jobs after caller cancellation. After snapshots are captured after client drain and final verification; they are not atomic across nodes. Trials use the same persistent KV9 fixture, so retained background work can affect subsequent trials. A fast-refusal population can dominate aggregate terminal latency; acknowledged latency must be reported separately.

Workers can issue new logical operations after fast explicit refusals. They never retry a prior uncertain write. Where refusals dominate, aggregate terminal percentiles largely describe refusals; the following percentiles include acknowledged calls only. Values are histogram intervals, not exact percentile estimates.

## Acknowledged latency at concurrency 64

| Mix | Baseline mean ms | Candidate mean ms | Baseline p99 interval ms | Candidate p99 interval ms |
|---|---:|---:|---|---|
| read | 0.751 | 0.767 | 1.049–2.097 | 1.049–2.097 |
| write | 669.133 | 86.232 | 536.871–1073.742 | 134.218–268.435 |
| mixed | 358.615 | 121.616 | 268.435–536.871 | 134.218–268.435 |

## CPU headroom and measurement limits

At concurrency 64, the candidate read client used 1.037–1.038 CPU cores out of its two allowed CPUs, with the busiest client thread at about 0.520 cores. The three database processes together used 3.607–3.613 cores out of four allowed CPUs. The Redis reference client used 1.36–1.47 cores across the three mixes, with its busiest thread at most 0.736 cores; its server used about one core. These observations leave CPU headroom in both clients under this protocol.

Candidate pure-write trials used only 0.016–0.020 client cores and 0.104–0.113 aggregate server cores at concurrency 64. Low CPU usage does not establish which durability or scheduling wait sets the remaining ceiling. Root's separate source/test work stayed on logical CPUs 6–31, which can still share physical-core and storage resources. No other owned runtime stress fixture was deliberately run during these timed trials.

## Durable-write stage sample

Candidate pure-write trial `b00011` has 195 acknowledged writes including initialization and warmup, and 0 unsuccessful measured calls. Before/after exporter snapshots encompass initialization, warmup, measurement, drain, and final reads; stage means are not isolated measured-only averages and counters are independent between metrics.

| Voter | Raft WAL sync calls | Mean Raft sync ms | Engine WAL sync calls | Mean engine sync ms |
|---|---:|---:|---:|---:|
| 1 | 390 | 7.202 | 195 | 4.999 |
| 2 | 390 | 7.123 | 195 | 8.204 |
| 3 | 390 | 7.517 | 195 | 8.156 |

In this sample, the recorded sync counts expose the durable I/O cost per acknowledged write. They are observations of this snapshot interval, not a universal fixed-count claim across elections, migrations, or future group-commit implementations.

## Concurrent durable-write sample

The following first-repetition concurrency-16 snapshot deltas include initialization, warmup, measurement, drain, and final reads. Sync/acknowledgment ratios summarize the entire interval; they are not isolated measured-phase stage averages. Engine sync counts remain visible independently of Raft sync counts.

| Revision | Voter | Acknowledged writes including setup/warmup | Non-acknowledged measured | Raft sync calls | Engine sync calls | Raft syncs / acknowledgment |
|---|---|---:|---:|---:|---:|---:|
| baseline | 1 | 353 | 0 | 264 | 353 | 0.748 |
| baseline | 2 | 353 | 0 | 267 | 353 | 0.756 |
| baseline | 3 | 353 | 0 | 264 | 353 | 0.748 |
| candidate | 1 | 616 | 0 | 405 | 201 | 0.657 |
| candidate | 2 | 616 | 0 | 406 | 205 | 0.659 |
| candidate | 3 | 616 | 0 | 406 | 204 | 0.659 |


## Grouped engine synchronization and repetition spread

In the concurrency-16 snapshot interval above, engine syncs per acknowledged write fell from exactly 1.000 in b3 to 0.326–0.333 in fe. The candidate applied 616 acknowledged writes including setup/warmup using 201–205 engine sync calls per voter. This directly observes coalescing under the stated whole-trial scope; it does not imply every measured command belongs to a fixed-sized group.

Both concurrency-64 pure-write snapshot intervals are retained in `/tmp/kv9-redis-paired-b3-fe-final/candidate-c64-stages.json`:

| Repetition | Measured confirmed writes/s | Acknowledged writes including setup/warmup | Engine sync calls per voter | Engine syncs / acknowledgment | Mean engine sync ms per voter |
|---|---:|---:|---|---|---|
| 0 | 957.8 | 3,041 | 196–208 | 0.064–0.068 | 7.23–8.64 |
| 1 | 513.9 | 1,697 | 193 | 0.114 | 7.79–9.10 |

The intervals contain similar engine sync-call counts but different amounts of acknowledged work. They include initialization, warmup, measurement, drain, and final reads, so these observations do not isolate a causal explanation for the repetition spread. Every original counter, latency histogram and CPU sample remains available for further analysis.

## Retained evidence

- baseline: `/tmp/kv9-redis-comparison-b3-attempt1`; matrix SHA-256 `fa2e43e1fa4b9cbbb22c91a053af9e371543f0fa909a185916a3e5e21fe15090`.
- baseline independent recheck: `/tmp/kv9-redis-comparison-b3-check-for-fe.json`; SHA-256 `a8c35ad929e65611c14313ac3ed8fe033661d6e102ec95300efd215ac23327c1`.
- baseline release database SHA-256: `5cb8a4469262015e461bb62e342159d4f7d6a30407e4aa9506a7a74501fc94e9`; source tree SHA-256 `c019316049bb35cafd36b21c8ebcc5fb3b10bdc7cf8063b67e6e2714a38a8169`.
- candidate: `/tmp/kv9-redis-comparison-fe-attempt1`; matrix SHA-256 `3840fb8d6cfe62dbaa0a9c472d71e386a04c62b96297645f48808464464609f9`.
- candidate independent recheck: `/tmp/kv9-redis-comparison-fe-check1.json`; SHA-256 `f67765b0f1ed17f085298dc9ed3fa07bd33fa2c196fded9944b4ad88fc8745fc`.
- candidate release database SHA-256: `f308336972eea6321defc331de2be3f88bc7f570a592af3529a1a78788b589dd`; source tree SHA-256 `1e77891287e905e805fa04e4455b7f9896e16c30d6bf7067e2cb8961bb6b2de5`.
- Redis: Redis server v=7.0.15 sha=00000000:0 malloc=jemalloc-5.3.0 bits=64 build=e53ff17674aa6190; server SHA-256 `1b2950213684d355e0e49fe6702c7bfb9d40b78f0da9ce289fc0e8f2a295e540`.
- Redis reference client SHA-256: `57947d5a94c739dd58ca98e2c9a85c40814040826f13f99e85cd0e2c7fb2f636`.

The full paired artifact is retained at `/tmp/kv9-redis-paired-b3-fe-final/paired.json` (SHA-256 `ec0aea2d7422aa363d6b80da333a7fab0aeb67ec096f31be1fe0f9361ae6a0ba`). It includes per-repetition CPU and throughput, pooled successful latency histograms, exact certainty families, and per-node backend occupancy. The local fixture, native Redis client, independent rechecker, outcome analyzer, and paired renderer are versioned together. Hosted CI was not invoked.

The final local audit is `/tmp/kv9-redis-fe-final-audit.json`; it confirms all
93 observed owned process identities exited. The candidate file manifest is
`/tmp/kv9-redis-fe-file-manifest.json` (1,010 files, 130,607,310 bytes).
The first build and first complete matrix attempt both passed; no failed or
partial candidate measurement was omitted.
