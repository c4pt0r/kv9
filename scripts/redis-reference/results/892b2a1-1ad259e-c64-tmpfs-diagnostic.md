# Ready versus asynchronous reads: volatile c64 diagnostic

The exact asynchronous-read candidate improved acknowledged GET throughput by
**30.3%** and mixed throughput by **10.3%** against the pooled, contemporaneous
Ready-before/Ready-after bracket. PUT differed by 0.5%, within the observed
repetition spread. GET's pooled p99 moved into the next lower histogram bucket,
while aggregate server CPU fell. This establishes a useful local diagnostic
result, not a general capacity or production acceptance claim.

**All runtime data was on volatile tmpfs. These runs provide no disk durability
or power-loss guarantee.** Normal quorum and sync calls remained enabled. Redis
is a standalone memory reference with persistence disabled, not an equivalent
durability deployment. The candidate's GET rate is still about 23% of its paired
Redis reference; this does not establish Redis-like performance.

## Exact runtime and fixed clients

| Artifact | Revision or SHA-256 |
|---|---|
| Ready runtime | `892b2a178450309859113c942f1738c070130eb5` |
| Async-read runtime | `1ad259e78c148b0b6b8d837b142a2e9521b60b3f` |
| Ready release executable | `e8232f8c612d92dad68b9209aa55d2d043d5a6fe3b22baac93c6735ee4c25b5d` |
| Async-read release executable | `ebf3b387bc362d2b1443cc30f2ab3b006c58114cb4b5a1ff24ab920f0ef73823` |
| Fixed Ready persistent client | `b47d5f408cd9a36ce7a8117917eb0fba530b0616125b4fff38980a255ddd12dc` |
| Fixed Redis reference client | `57947d5a94c739dd58ca98e2c9a85c40814040826f13f99e85cd0e2c7fb2f636` |

The candidate is based on Ready; it excludes the proposal-queue candidate.
Root built its clean, exact release source with empty features on CPUs 6–31,
then copied the server executable and unchanged Ready client into
`/tmp/kv9-redis-async-read-build`. The target used for that root-owned build was
`/home/dongxu/kv9/target`; this diagnostic used the copied executable and did
not rebuild it. Runtime and client source identities are recorded separately.
A same-revision full benchmark build gate is not claimed.

The independently computed source-inventory digests are
`a49402a2fd5e12b97e778b5d4d5186ee58a88a54fef8d24abc42c6072079c31f`
for Ready and
`582af1d0d078b65faea17881084a338ec30c5dcc114df9a78d65eef466ee08f0`
for the candidate. All inventoried inputs and actual executing process hashes
were checked against their manifests. The candidate build manifest has the
individual source hashes; its aggregate digest above was computed by the audit.

## Unchanged workload and placement

Each of Ready-before, candidate and Ready-after has twelve cohorts: GET100,
PUT100 and GET50/PUT40/DELETE10, with two three-second repetitions and separate
KV9/Redis executions. Repetition one runs KV9 before Redis; repetition two
reverses those targets. Each cohort uses concurrency 64, one outstanding request
per worker, 64 hot keys, 23-byte keys, 128-byte values and 32 warmup operations.
KV9 retains 1,500-ms deadlines and at most six attempts for explicit NotLeader
routing refusals; uncertain writes are never retried. Redis has no retries or
pipelining. No operation cap was reached.

Clients use logical CPUs 0–1; all three voters share CPUs 2–5 on one host over
IPv4 loopback. The corrected launcher explicitly exposes CPUs 0–31 so the
unchanged fixture chooses those intended subsets. An additional observer checks
the inherited mask before each server/client execution, captures the actual
child affinity immediately after execution, and verifies those observations
precede the measured interval. The workload and existing validators are unchanged.
Root runtime checks and the release build completed before timing; root archival
ran after timing and process cleanup. Heavy proof workers were paused. This remains one shared host with logical CPU/topology limitations,
short closed-loop repetitions and no independent machine failure domains.

Redis 7.0.15's live configuration confirms `save ""`, `appendonly no`, zero
replicas and one I/O thread. Missing GET values are successful nil/None results
in mixed traffic; the fixed KV9 performance client does not export hit/miss counts.

## Throughput, latency and CPU

Rates are acknowledged operations divided by cohort elapsed time, including
drain. Parentheses retain repetition one / two.

| Workload | Ready-before ops/s | Async-read ops/s | Ready-after ops/s | Candidate / pooled Ready |
|---|---:|---:|---:|---:|
| GET100 | 86,172.5 (86,031.3 / 86,313.7) | 112,459.5 (112,573.7 / 112,345.2) | 86,424.3 (86,659.2 / 86,189.5) | 1.303 |
| PUT100 | 67,151.7 (67,552.9 / 66,750.4) | 67,347.6 (67,131.9 / 67,563.3) | 66,927.3 (67,213.0 / 66,641.6) | 1.005 |
| Mixed | 68,886.6 (69,600.1 / 68,173.1) | 75,992.7 (76,197.3 / 75,788.2) | 68,905.5 (69,179.0 / 68,631.9) | 1.103 |

Contemporaneous standalone Redis references were:

| Paired matrix | GET/s | PUT/s | Mixed/s |
|---|---:|---:|---:|
| Ready-before | 501,086.3 (492,986.6 / 509,186.0) | 492,071.4 (491,704.0 / 492,438.7) | 497,246.4 (496,257.4 / 498,235.5) |
| Async-read | 494,564.2 (478,960.5 / 510,168.3) | 493,830.8 (494,391.3 / 493,270.4) | 504,313.0 (505,330.1 / 503,296.0) |
| Ready-after | 505,395.9 (503,943.1 / 506,847.3) | 489,130.0 (490,487.8 / 487,772.2) | 497,324.9 (498,929.2 / 495,720.7) |

Pooled acknowledged GET p99 is in **0.524288–1.048575 ms** for the candidate,
versus **1.048576–2.097151 ms** for both Ready matrices. PUT and mixed remain
in the latter interval for all three matrices. These are histogram bucket
bounds, not exact percentile points.

| Workload | Ready-before server cores (repetitions) | Async-read server cores | Ready-after server cores |
|---|---:|---:|---:|
| GET100 | 3.593 / 3.619 | 3.149 / 3.231 | 3.595 / 3.620 |
| PUT100 | 3.625 / 3.612 | 3.601 / 3.602 | 3.609 / 3.618 |
| Mixed | 3.657 / 3.646 | 3.569 / 3.563 | 3.668 / 3.669 |

Candidate GET clients used 1.202 / 1.192 of two allowed cores, with the busiest
thread at 0.603 / 0.596 cores. Across all reference cohorts, Redis clients used
at most 1.411 cores and their busiest thread 0.708. These observations show
client CPU headroom at this concurrency, not an unlimited-load guarantee or an
attribution of every saved CPU cycle.

## Outcomes, registry use and retained evidence

All **36 valid cohorts** completed: **4,202,354 KV9** and **26,853,510 Redis**
measured logical operations succeeded. There were zero final refusals, unknown outcomes,
transport/read failures or client rejections. This statement does not erase
successful operations' permitted NotLeader routing attempts.

Before the candidate's first GET cohort, all async registry peaks were zero.
After it, voter 2's peak was 65, voter 1's was 1 and voter 3's was zero, showing
that serving reads exercised the new path. No per-voter cause is inferred from
these cumulative gauges. The registry bound is 128; client/public concurrency
64 is not an internal-registry peak bound, since request release follows result
publication. Every after-verification snapshot has zero async queued, active
and in-flight entries on all voters. All public count/byte/running ledgers also
drain to zero. Public read running/backend timing now includes asynchronous
preparation, blocking-pool queue and engine execution, not OS thread occupancy.

Valid raw evidence is `/tmp/kv9-ready-async-c64-tmpfs-corrected2`:
`plan.json`, three original matrices, complete outcome histograms and CPU samples,
per-process identities, live Redis observations, premeasurement affinity guards,
all six primary checks, `final-audit.json`, and
`latency-cpu-stage-details.json`. The final independent audit passed on its first
invocation, checking all 36 cohorts and exact source/client/runtime identities,
premeasurement placement, admission settings and residual registries.
All 63 observed process lifetimes exited. Before removing owned tmpfs paths,
original/copy hashes were compared for 48 files / 671,574,846 bytes in Ready-before,
51 files / 694,272,402 bytes in the candidate, and 48 files / 670,178,434 bytes in
Ready-after. Those data copies remain evidence on disk, not runtime durability.

Two earlier environment attempts remain separate and excluded:

- `/tmp/kv9-ready-async-c64-tmpfs-diagnostic`: the outer launcher mistakenly
  inherited CPUs 6–31, making the dynamic fixture choose clients 6–7 and voters
  8–11. All three unchanged placement observers rejected it. Its 36 completed
  cohorts are invalid for this comparison and retained without rewriting them.
  The final invalid matrix had already completed before a targeted stop could
  execute; no signal was sent. All 63 owned lifetimes exited and all data copies
  were retained before scratch cleanup.
- `/tmp/kv9-ready-async-c64-tmpfs-corrected`: the added placement guard initially
  mistook `redis-server --version` for a measured server. It rejected that
  preflight before any voter/client launch. Only that observer classification
  was corrected; no workload or validator assertion was weakened.

This report does not replace disk, MinIO, Chaos Mesh, proof or whole-system
acceptance gates. No hosted CI was triggered by this diagnostic.
