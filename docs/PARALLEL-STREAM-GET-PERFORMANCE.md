# Bounded parallel stream GET performance

Candidate `f2c4e856d03f75ac6e4718f0aac7f4e7a1cf3d02` improves point GET from
207,734–208,320 to **287,795–288,611 successful calls/s** in the matched
comparison. Paired gains are **38.15% and 38.93%**. Mean whole-call latency
falls from 307.0–307.9 to **221.6–222.2 us**. Candidate p99 buckets span
389.120–401.407 us, below both control p99 buckets (446.464–458.751 us).

BatchGet(1) improves from 199,516–200,361 to **281,983–287,208 calls/s**,
with paired gains of **41.33% and 43.35%**. Mean latency falls from 319.2–320.6
to 222.7–226.8 us; p99 also improves in both repeats. These are calls/s and
items/s only because each request contains one key. Larger batches and writes
were not timed in this comparison.

Actual Redis GET reaches **496,264–505,568 calls/s**, with 126.5–128.8 us
mean latency and p99 buckets spanning 231.424–235.519 us. Its point-read
throughput advantage is now **1.757x and 1.719x** within the two repeats, compared
with approximately 2.43x in the [previous accepted comparison](BORROWED-BATCH-GET-PERFORMANCE.md).
Redis parity remains open.

## Decision and interpretation

Retain bounded parallel handler scheduling for continued performance development.
Both candidate point repetitions exceed both same-run control repetitions;
mean and p99 latency improve as well. The result supports the hypothesis that
polling synchronous handler work in one stream owner constrained useful
parallelism. It does not isolate a complete latency breakdown or establish a
hardware ceiling. The stream ownership correction is separately covered by a
compiled premature-release mutation control.

The [local and actual Chaos acceptance](PARALLEL-STREAM-REQUESTS-ACCEPTANCE.md)
passed before timing. The backend still performs the same Raft read barrier and
committed/applied write protocol. This is a candidate-branch result, with no
master/default promotion or completion of the broader proof/fault roadmap.
Next investigate the common request path's allocation/copying and
synchronization costs using bounded profiles and matched experiments. First
screen narrow changes with targeted correctness tests and provisional timing,
then perform full acceptance for useful candidates. Do not relax consistency
to close the remaining gap.

## Source and measurement contract

| Role | Exact revision | Executable SHA-256 |
|---|---|---|
| Borrowed-context control server | `850f0deeed8d03426d28d65bbcdd94a503f16cf7` | `916738e118bf3d9f2c373f4fe81d7523db0db0f9a401acda9bfbc0450c9dd684` |
| Parallel stream candidate server | `f2c4e856d03f75ac6e4718f0aac7f4e7a1cf3d02` | `3ed7974e3eebd9dc6a0e91988d3913f2999fe44fa38d989737022481f993d305` |
| Shared native client | `03c1c776a5dd7d1cc67491ab253e02ce51665bf8` | `22ca0883ca2900fab0b457d87ebc6bc50662d7145af0846304f02b5841a6d30b` |
| Shared Redis GET/MGET client | `b8ec38f660786412705f350d96ab086f0e7f6c60` | `61349111171ded5cb7fff9085c7886381788e5f4c7a407e91a5dda618d124892` |

All source trees are clean and frozen. Executables use default-feature release
builds and the same compiler. Only server revision/build pins and paths changed
from the accepted six-arm driver; its wrapper, native client, Redis client and
explicit client argument file are unchanged. The independent auditor has only
six corresponding server-pin/path substitutions, preserving all predicates.
Point GET pairs with actual Redis GET; BatchGet(1) pairs with Redis MGET(1).

Each fresh cohort uses 64 closed-loop workers, 4,096 keys plus a sentinel,
128-byte values, batch size one, read-only traffic, seed 71, 128 warmup calls,
a 1,500 ms measurement interval and a ten-million-call safety cap. The second
repeat reverses the six-arm order. Setup, drain and complete deterministic
nonce-zero data readback are outside timing. Whole-call latency includes
preparation, transport and response validation. These short closed-loop runs
are not sustained-capacity or offered-load saturation curves.

Client CPUs are 0–1; all measured servers share CPUs 2–5. Three owned background
containers are restricted to 6–15,22–31 during timing and restored to their exact
configured/effective `0-31` afterward. All builds, tests, fault injection,
profiling and independent audits were terminal before timing. Unrelated host
services and BuildKit remain unconstrained: this is a shared-host diagnostic.

KV9 has three voters, ordinary streaming gRPC and normal quorum-confirmed reads,
with WAL sync calls retained on volatile tmpfs. Redis is one memory instance
with save/AOF disabled, no replicas and one I/O thread. Their fault tolerance
and power-loss durability are not equivalent. This run makes no disk-durability,
cross-host or new write-throughput claim.

## All twelve cohorts

All **5,941,631 measured calls succeeded**, with no refused,
unknown, failed, client-rejected or dropped calls and no extra transport
attempts. No cohort reached the safety cap. Quantiles below are retained
histogram bucket intervals, not exact percentile samples or averages of
percentiles.

| Cohort | Calls | Calls/s | Mean us | p50 us | p95 us | p99 us |
|---|---:|---:|---:|---|---|---|
| 000-old-point-p00064 | 312,541 | 208,320.2 | 307.040 | 315.392–319.487 | 393.216–397.311 | 454.656–458.751 |
| 001-old-batch1-p00064 | 299,295 | 199,515.7 | 320.569 | 335.872–339.967 | 401.408–405.503 | 454.656–458.751 |
| 002-new-point-p00064 | 431,738 | 287,794.9 | 222.232 | 210.944–212.991 | 344.064–348.159 | 397.312–401.407 |
| 003-new-batch1-p00064 | 423,025 | 281,982.8 | 226.778 | 215.040–217.087 | 352.256–356.351 | 401.408–405.503 |
| 004-redis-mget1-p00064 | 741,814 | 494,457.9 | 129.285 | 118.784–119.807 | 169.984–172.031 | 229.376–231.423 |
| 005-redis-get1-p00064 | 758,456 | 505,568.1 | 126.462 | 118.784–119.807 | 165.888–167.935 | 233.472–235.519 |
| 006-redis-get1-p10064 | 744,507 | 496,263.9 | 128.820 | 118.784–119.807 | 167.936–169.983 | 231.424–233.471 |
| 007-redis-mget1-p10064 | 754,240 | 502,749.5 | 127.159 | 118.784–119.807 | 167.936–169.983 | 229.376–231.423 |
| 008-new-batch1-p10064 | 430,853 | 287,207.7 | 222.655 | 212.992–215.039 | 335.872–339.967 | 393.216–397.311 |
| 009-new-point-p10064 | 432,956 | 288,611.0 | 221.602 | 210.944–212.991 | 339.968–344.063 | 389.120–393.215 |
| 010-old-batch1-p10064 | 300,570 | 200,361.0 | 319.214 | 331.776–335.871 | 405.504–409.599 | 458.752–462.847 |
| 011-old-point-p10064 | 311,636 | 207,733.9 | 307.906 | 319.488–323.583 | 393.216–397.311 | 446.464–450.559 |

## Acceptance and retained evidence

The timing wrapper and child exited 0: root session **56838**. The independent
read-only audit passed on its first execution, with all 12 cohorts, 40 exited
owned lifetimes, 2,320 source-file checks, 349 in-window resource samples,
24 qualifying fresh drains, and 24 voter/listener/mount bindings. All
264 retained files / 44,377,937 bytes match. Container CPU sets are exactly
restored, historical namespace UIDs are unchanged and cleanup errors are empty.
No measurement or audit rerun occurred. Read-only performance accounting is
separate from the full atomic-history correctness checks.

- Raw matrix: `/tmp/kv9-parallel-stream-matched-diagnostic-attempt1/cohorts`.
- Outer completion: `/tmp/kv9-parallel-stream-matched-diagnostic-attempt1/summary.json`.
- Independent result: `/tmp/kv9-parallel-stream-matched-independent-first/results-first`.
- Frozen driver/preflight: `/tmp/kv9-parallel-stream-comparison-preparation`.

| Artifact | SHA-256 |
|---|---|
| Driver | `80af32abe0f7b757a11fe8e77057480d26ce4a19383ea98f5cede4a7027104da` |
| Wrapper | `e922939e675d722fe36c3b0890d31d6329be810dd7567b2e369643d10e2137fb` |
| Raw matrix | `d62463e3a23a2fbd14178e73425625b04ec58f3ee37a771d44b96ab92454a363` |
| Outer completion/restoration | `c07a17e59dddeefcdf3315a39f50a3915d5fdc1ebff1583a93026461e54975a8` |
| Independent audit | `596f4bbe66dd58cce0e8e7df100d7cdb10d6f2cec7dc3dedd3df27ab685c7aa5` |
| Independent input inventory | `f6baffc5be2af62829733f469542839843bdb4fa3b81c2e0ad8140cd7bb5f980` |

No hosted CI was dispatched.
