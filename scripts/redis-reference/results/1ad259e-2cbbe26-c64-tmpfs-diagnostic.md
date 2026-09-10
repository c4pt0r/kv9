# Asynchronous reads versus sealed read groups: volatile c64 diagnostic

Exact sealed-read-groups candidate `2cbbe26` improved acknowledged GET throughput
by **7.0%** and mixed throughput by **5.0%** against the pooled contemporaneous
async1ad-before/async1ad-after bracket. PUT differed by 0.2%, within the observed
repetition spread. The p99 histogram buckets did not change. This is a modest
local diagnostic improvement, not a production acceptance or general capacity
claim.

**All runtime data was on volatile tmpfs: these runs provide no disk durability
or power-loss guarantee.** Normal quorum and sync calls remained enabled.
Standalone Redis has persistence disabled and no replicas, so it is a reference
with different guarantees. Candidate GET throughput was approximately 24% of
its paired Redis reference; this does not establish Redis-like performance.

## Exact artifacts and unchanged protocol

| Artifact | Revision or SHA-256 |
|---|---|
| Async baseline runtime | `1ad259e78c148b0b6b8d837b142a2e9521b60b3f` |
| Sealed read-groups runtime | `2cbbe26a3d4273c6d265fa40c8b56c548b1a59ec` |
| Async release executable | `ebf3b387bc362d2b1443cc30f2ab3b006c58114cb4b5a1ff24ab920f0ef73823` |
| Groups release executable | `b460443c768847e6e931e9915d6959fe359ba045ea807e1309e0238c44ecc05f` |
| Fixed Ready persistent client | `b47d5f408cd9a36ce7a8117917eb0fba530b0616125b4fff38980a255ddd12dc` |
| Fixed Redis reference client | `57947d5a94c739dd58ca98e2c9a85c40814040826f13f99e85cd0e2c7fb2f636` |

Root built the clean exact candidate in release mode with features `[]` and
copied it to `/tmp/kv9-redis-read-groups-build`; this diagnostic did not rebuild
it or change the client. The root build used CPUs 6–31 and its shared target,
`/home/dongxu/kv9/target`. Runtime and client source identities remain separate;
a same-revision client build is not claimed. All build-manifest source inputs
and executing executable hashes were independently checked. Aggregate source
inventory digests are `582af1d0d078b65faea17881084a338ec30c5dcc114df9a78d65eef466ee08f0`
for async1ad and `9dc1040001b71f3aa92bdc86871b50ea2e9b71422df1e7974505e125751b4586`
for the candidate.

The existing protocol ran three matrices in order: async1ad-before, candidate,
async1ad-after. Each contains GET100, PUT100 and GET50/PUT40/DELETE10, each with
two three-second repetitions and separate KV9/Redis executions: **36 cohorts**.
Repetition one runs KV9 before Redis; repetition two reverses those targets.
Every cohort uses concurrency 64, 64 hot keys, 23-byte keys, 128-byte values,
one outstanding request per worker and 32 warmup operations. KV9 retains
1,500-ms deadlines, six maximum attempts for explicit NotLeader routing, and
no retries of uncertain writes. Redis has no retries or pipelining. No operation
cap was reached.

The known corrected affinity launcher was unchanged: outer CPUs 0–31, clients
0–1 and all three voters 2–5. The inherited mask was checked before every
client/server execution; actual masks and executable hashes were observed
before the measured intervals. Runtime/client communication uses IPv4 loopback
on one shared host. Root runtime checks and release compilation finished before
timing; the proof scheduler paused at a safe boundary with no active heavy
worker. Root performed only small documentation/GitHub operations until timing
and owned process cleanup finished. These are short closed-loop observations
with logical CPU placement, not isolated hosts or separate failure domains.

Redis 7.0.15's retained live configuration shows `save ""`, `appendonly no`,
zero replicas and one I/O thread. Missing GETs are successful nil/None results
in mixed traffic; the unchanged KV9 performance client does not export hit/miss
counts. Existing workload and validators were unchanged; the separate observer
adds only group-counter and drain checks.

## Throughput and latency

Rates divide acknowledged operations by cohort elapsed time, including drain.
Parentheses retain repetition one / two; the baseline ratio pools all four
contemporaneous async1ad repetitions.

| Workload | Async-before ops/s | Read-groups ops/s | Async-after ops/s | Candidate / pooled baseline |
|---|---:|---:|---:|---:|
| GET100 | 112,020.0 (112,162.6 / 111,877.4) | 120,358.6 (120,906.5 / 119,810.8) | 112,983.8 (113,456.9 / 112,510.7) | 1.070 |
| PUT100 | 66,776.4 (66,702.7 / 66,850.1) | 67,202.1 (67,572.7 / 66,831.4) | 67,366.0 (67,325.7 / 67,406.4) | 1.002 |
| Mixed | 75,655.2 (75,779.8 / 75,530.7) | 79,766.8 (79,825.6 / 79,707.9) | 76,276.3 (76,490.2 / 76,062.4) | 1.050 |

Contemporaneous standalone Redis references:

| Paired matrix | GET/s | PUT/s | Mixed/s |
|---|---:|---:|---:|
| Async-before | 508,143.9 (508,287.9 / 507,999.9) | 490,832.0 (491,282.4 / 490,381.6) | 503,844.7 (504,227.5 / 503,461.9) |
| Read-groups | 504,558.7 (505,048.4 / 504,068.9) | 491,466.9 (490,410.2 / 492,523.5) | 498,841.5 (496,693.6 / 500,989.4) |
| Async-after | 508,655.8 (507,001.2 / 510,310.3) | 494,155.3 (494,375.8 / 493,934.7) | 496,504.9 (493,731.7 / 499,278.1) |

Every KV9 GET repetition, and each pooled GET pair, has acknowledged p99 in
**0.524288–1.048575 ms**. Every KV9 PUT/mixed repetition and pair remains in
**1.048576–2.097151 ms**. Every Redis repetition has p99 in
**0.131072–0.262143 ms**. These are observed bucket bounds, not exact percentile
points; this experiment establishes no p99 bucket improvement.

| Workload | Async-before server cores (repetitions) | Read-groups server cores | Async-after server cores |
|---|---:|---:|---:|
| GET100 | 3.168 / 3.236 | 3.095 / 3.190 | 3.141 / 3.217 |
| PUT100 | 3.595 / 3.609 | 3.610 / 3.617 | 3.605 / 3.612 |
| Mixed | 3.558 / 3.576 | 3.542 / 3.555 | 3.559 / 3.560 |

Candidate GET clients used 1.280 / 1.259 of two allowed cores; busiest threads
used 0.640 / 0.628 cores. Across the Redis cohorts, client CPU stayed below
1.398 cores and its busiest thread below 0.701. These observations show client
CPU headroom at c64; they do not identify every remaining bottleneck or support
an unbounded-load claim.

## Actual grouping and complete outcomes

All **4,671,143 KV9** and **26,984,755 Redis** measured logical operations
succeeded. There were zero final refusals, unknown outcomes, transport/read
failures or client rejections. Successful operations may still contain the
permitted explicit NotLeader routing attempts. Complete outcome populations,
histograms, reports and resource samples are retained.

Candidate counters demonstrate actual sealed grouping:

| Workload / repetition | Admitted groups | Admitted members | Members per group |
|---|---:|---:|---:|
| GET / 1 | 155,827 | 362,905 | 2.329 |
| GET / 2 | 141,329 | 359,639 | 2.545 |
| Mixed / 1 | 38,323 | 120,216 | 3.137 |
| Mixed / 2 | 38,163 | 120,059 | 3.146 |
| PUT / 1 | 130 | 130 | 1.000 |
| PUT / 2 | 130 | 130 | 1.000 |

These deltas span complete before/after trial snapshots, including prefill,
warmup, measurement, drain and final verification. In particular, the PUT
trial's reads belong to setup/verification; this is not a measurement-only
ReadIndex-call reduction ratio. Maximum admitted group size reached 64, within
the configured turn bound, and is a process-lifetime maximum rather than a
per-measurement maximum. Group and member counters were monotonic on every
voter; admitted groups never exceeded admitted members, and members never
exceeded 64 times groups.

Every after-verification snapshot has zero active groups, queued requests,
active members and in-flight reservations on all candidate voters; both
baseline matrices' async registries also drained. Every matrix's public
request count, queued/running work and encoded bytes drained to zero.
Candidate registry peak 65 is within its 128 bound: client concurrency 64 is
not an internal peak bound because reservation release follows response
publication. No cause for another voter's cumulative peak is inferred without
routing evidence.

## Evidence and limits

Original evidence: `/tmp/kv9-async-groups-c64-tmpfs-bracket`. The three original
matrices, full outcome reports/CPU samples, tmpfs mount observations and data
copies, exact source/build/client identities, premeasurement affinity records,
and all six unchanged checks remain available. `final-audit.json` independently
checks all 36 cohorts and 63 executing process lifetimes; SHA-256
`b5ceef02a4b9afcddf3a547a60988d7e0bf458502dab6e21b25e1a23f3bc5c6b`.
`group-counter-summary.json` retains every voter's before/after counter delta;
SHA-256 `574d3f78a909f47152b44cf180364c7b023d35fb8501a30edfe888cecff5eaf0`.
`latency-cpu-stage-details.json` retains further stage and CPU observations;
SHA-256 `4140ffb5d366928c51952a31aba74bcd8f042b5cd69ca0c3be392d985e8b11c9`.

All matrices and the main independent audits passed their first invocation.
An additional per-repetition percentile cross-check initially used expected
bucket indices one position too high; its exact input and failure remain
retained. Only that extra observer was corrected to compare the already
reported bucket bounds directly. No failed candidate cohort was discarded or
rerun. The earlier Ready/async experiment's affinity
failures remain with that experiment; they are not relabeled as attempts here.
All 63 observed owned process lifetimes exited. Each owned tmpfs data directory
was copied and verified against the original before scratch removal; the
retained disk copies do not change runtime volatility. All retained evidence remains in place. No additional timing, hosted CI, runtime mutation or shared-validator
change was performed for this report.

This diagnostic does not replace disk, MinIO, actual Chaos Mesh, correctness
proof or composition acceptance. The earlier exact-1ad NetworkChaos evidence
is not acceptance of this new read-groups candidate.

The complete inventory is `/tmp/kv9-async-groups-c64-tmpfs-inventory.json`:
**1,696 files / 2,216,617,283 bytes**, SHA-256
`cc5de1578263ff17bc7d67ac687ff571ca92afab7ff5581dd5f891435fa90145`.
Every inventoried original was reopened and its length/hash independently
verified; `/tmp/kv9-async-groups-c64-tmpfs-inventory-verification.json` records
that second pass. The inventory covers original matrices, the retained extra
observer failure, exact release/client builds, source inputs and helpers.
Verified tmpfs data copies comprise 48 files / 689,376,766 bytes before,
51 / 704,608,808 bytes for groups, and 51 / 695,160,674 bytes after.
