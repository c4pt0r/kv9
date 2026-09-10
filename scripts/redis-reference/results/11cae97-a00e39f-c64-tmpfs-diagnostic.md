# Asynchronous write completion: volatile c64 diagnostic

Exact `a00e39f` improved acknowledged PUT throughput by **18.40%** and mixed
throughput by **10.61%** against the surrounding resident `11cae97` baseline.
Both candidate PUT and mixed repetitions exceed all four corresponding baseline
repetitions. GET was **0.26% lower** overall, with overlapping repetitions; no
GET improvement or general absence of regressions is claimed.

Candidate PUT reached **78,501.3/s**, versus its standalone Redis reference
**492,632.4/s**: about 15.9% of the reference throughput. The remaining gap is
large. This result supports retaining `a00e39f` for further performance work;
it does not promote the runtime past its remaining fault/proof gates.

**All database data was volatile tmpfs: no disk or power-loss durability.**
Normal Raft quorum and sync calls remained enabled. Redis was standalone with
persistence disabled and zero replicas. These differ in guarantees and work per
logical operation. This short c64 diagnostic is not disk acceptance, a general
capacity estimate, or evidence of independent host failure domains.

## Frozen inputs and protocol

| Artifact | Revision or SHA-256 |
| --- | --- |
| Resident baseline | `11cae977f15df0912c2a35561480d45447bae660` |
| Async-write candidate | `a00e39f9f8da7bb381df19afd2eed63e8a1f1b75` |
| Baseline release executable | `413cfdb20bd947d56a51c792c1d8ab69a676af98a50721649c7b0eceb54f9f33` |
| Candidate release executable | `b50da6f48e4ed88416892a3490653afb979a410a8c8c935abfc5d1e6008ccfbc` |
| Unchanged Ready client | `b47d5f408cd9a36ce7a8117917eb0fba530b0616125b4fff38980a255ddd12dc` |
| Unchanged Redis client | `57947d5a94c739dd58ca98e2c9a85c40814040826f13f99e85cd0e2c7fb2f636` |

Root supplied clean release builds with features `[]` at
`/tmp/kv9-redis-resident-read-build` and
`/tmp/kv9-redis-async-write-wait-build`. All 412 resident and 415 candidate
build inputs were independently rehashed and copied into evidence. Aggregate
source digests are
`34b6fcebd05d90f799d912425dd64e247ffe758e8ae88bbb7274670ace648ef3` and
`64d464d00e1ab3d433825b70da64475eac407371c72f283541f868cb88bb1884`.
The client retains its original Ready `892b2a1` build/source provenance;
it is not relabeled as a client built from either server revision.

The unchanged protocol ran resident-before, candidate, resident-after. Each
matrix contains GET100, PUT100 and GET50/PUT40/DELETE10, two three-second
repetitions, and separate KV9/Redis executions: **36 cohorts**. Repetition two
reverses target order. There are 64 workers, 64 hot keys, 23-byte keys,
128-byte values, one outstanding request per worker and 32 warmup operations.
KV9 keeps 1,500-ms deadlines, six maximum attempts for explicit NotLeader
routing, and no retries of uncertain writes. Redis has no retries or pipelining.
Every trial stopped by duration below the operation cap.

The same affinity guard checked outer CPUs 0–31, clients 0–1, and all three
voters 2–5 before measurement. Root heavy work completed and the complete proof
scheduler subtree was quiescent before launch; proof resumed after the 36
cohorts and all owned processes had exited. IPv4 loopback and logical CPU masks
on one shared host do not establish physical isolation. No new polling or
instrumentation was added for the async-apply observer.

Redis 7.0.15 retained `save ""`, `appendonly no`, zero replicas and one I/O
thread. Missing GETs are successful nil/None in mixed traffic. The frozen
KV9 performance client does not expose a hit/miss split.

## Throughput, p99 and CPU

Rates divide acknowledged operations by cohort elapsed time, including drain.
Parentheses show repetition one / two. Changes pool all four baseline
repetitions rather than selecting one favorable baseline.

| Workload | Resident-before ops/s | Async write ops/s | Resident-after ops/s | Change vs pooled baseline |
| --- | ---: | ---: | ---: | ---: |
| GET100 | 136,110.3 (136,250.9 / 135,969.8) | 136,171.3 (136,772.7 / 135,570.0) | 136,948.0 (137,431.5 / 136,464.5) | -0.262% |
| PUT100 | 66,315.0 (66,674.8 / 65,955.1) | 78,501.3 (78,363.6 / 78,638.9) | 66,292.8 (66,183.7 / 66,401.9) | +18.396% |
| Mixed | 84,218.4 (84,711.3 / 83,725.6) | 92,711.6 (93,630.4 / 91,792.8) | 83,419.0 (83,786.9 / 83,051.1) | +10.610% |

Contemporaneous Redis reference, including both repetitions:

| Matrix | GET/s | PUT/s | Mixed/s |
| --- | ---: | ---: | ---: |
| Resident-before | 507,319.4 (507,160.9 / 507,477.8) | 492,699.6 (493,560.2 / 491,838.9) | 498,983.7 (498,165.1 / 499,802.2) |
| Async write | 507,002.8 (507,189.9 / 506,815.7) | 492,632.4 (492,333.8 / 492,931.1) | 501,063.6 (498,102.9 / 504,024.2) |
| Resident-after | 504,103.8 (501,533.9 / 506,673.8) | 491,776.9 (490,742.0 / 492,811.8) | 498,683.7 (497,954.4 / 499,413.0) |

Every individual KV9 GET repetition has acknowledged p99 in
**0.524288–1.048575 ms**; every PUT/mixed repetition remains in
**1.048576–2.097151 ms**. Every Redis repetition has p99 in
**0.131072–0.262143 ms**. These are histogram buckets, not exact percentile
points. End-to-end p99 did not move to a lower bucket.

CPU observations, repetition one / two; server cores sum all three voters:

| Matrix / workload | Server cores | Client cores | Busiest client thread cores |
| --- | ---: | ---: | ---: |
| Resident-before / GET100 | 3.089 / 3.132 | 1.410 / 1.401 | 0.704 / 0.700 |
| Resident-before / PUT100 | 3.596 / 3.601 | 0.774 / 0.773 | 0.387 / 0.387 |
| Resident-before / Mixed | 3.515 / 3.519 | 0.954 / 0.949 | 0.477 / 0.471 |
| Async write / GET100 | 3.078 / 3.116 | 1.425 / 1.405 | 0.713 / 0.701 |
| Async write / PUT100 | 3.520 / 3.522 | 0.889 / 0.887 | 0.446 / 0.445 |
| Async write / Mixed | 3.461 / 3.454 | 1.040 / 1.022 | 0.520 / 0.513 |
| Resident-after / GET100 | 3.096 / 3.153 | 1.421 / 1.400 | 0.709 / 0.703 |
| Resident-after / PUT100 | 3.599 / 3.631 | 0.774 / 0.777 | 0.387 / 0.390 |
| Resident-after / Mixed | 3.523 / 3.520 | 0.941 / 0.937 | 0.472 / 0.470 |

Across Redis cohorts, client CPU was at most 1.398 cores and the
busiest client thread at most 0.701. Both clients had observed CPU
headroom within their two allowed cores; this does not identify every
bottleneck or establish saturation capacity. Complete per-repetition client,
server and thread samples remain in the raw matrices.

## Existing write-stage observations

The following successful stage means come only from differences of existing
before/after counters and histograms, in microseconds (repetition one / two).
The intervals include prefill, warmup, measurement, drain, verification and
metrics export boundaries. Nested stages overlap; wall waiting is not CPU work,
and these means must not be added as independent latency components.

| Stage | Resident-before µs | Async write µs | Resident-after µs |
| --- | ---: | ---: | ---: |
| Public preparation queue | 41.11 / 40.57 | 20.84 / 16.45 | 42.44 / 39.33 |
| Proposal submission | 9.54 / 10.27 | 9.59 / 10.96 | 9.37 / 10.77 |
| Application wait | 515.48 / 520.40 | 429.29 / 439.44 | 518.32 / 520.04 |
| Logical proposal wait | 525.24 / 530.92 | 440.90 / 452.44 | 527.92 / 531.07 |
| Public backend | 529.37 / 535.86 | 445.30 / 457.15 | 532.18 / 535.86 |
| Command apply | 43.66 / 44.32 | 37.31 / 39.97 | 43.21 / 44.34 |

Whole-trial successful write counts are 200,185 / 198,001 before,
235,230 / 236,057 for the candidate, and 198,689 / 199,346 after. Each of the
first five stage counts matches its trial's count; command apply is three
observations per acknowledged write, across three voters. Every outcome and
per-voter histogram delta is retained and checked for nonnegative counts/sums,
matching buckets and absent saturation/clamping.

The preparation queue measures admission-to-start, including dispatch. Public
backend timing includes the logical asynchronous wait in the candidate.
Candidate application timing starts after submission and before registration,
ending when completion is observed; the baseline surrounds synchronous
`wait_applied`. This boundary difference is retained in the interpretation.

Candidate backend/application/logical-wait delta-histogram p99 falls in
**0.524288–1.048575 ms**, versus **1.048576–2.097151 ms** for every baseline
repetition. Preparation queue p99 remains **0.131072–0.262143 ms** for the
candidate; one baseline repetition reaches **0.262144–0.524287 ms**. Proposal
submission p99 remains **0.131072–0.262143 ms** everywhere. Remaining application
and logical waiting are much larger than mean submission time; these samples
do not distinguish quorum, application and wakeup contributions within that wait.

## Complete outcomes, async apply and retained reads

All **5,284,937 KV9** and **26,968,000 Redis** measured logical operations
succeeded. There were zero final refusals, unknown outcomes, transport/read
failures or client rejections. Permitted explicit NotLeader routing attempts
remain visible; successful logical completion need not mean a single RPC attempt.

The separate async-apply audit uses only the existing per-PID status/resource
snapshots. The source limit is 128. Queued/in-flight/peak bounds, stopped=false,
monotonic peak within each attested lifetime, and zero queued/in-flight at
both snapshot boundaries all passed. Voter 2's lifetime peak rose 0→1 in the
first GET trial and 1→64 in the first PUT trial, then remained 64; voters 1/3
remained at zero. These are occupancy/high-water observations, not cumulative
admissions or attribution to individual measured responses.

Candidate read counters, with legal fallback retained:

| Workload / repetition | Inline completed | Blocking submitted | Inline fraction | Groups | Members |
| --- | ---: | ---: | ---: | ---: | ---: |
| GET100 / 1 | 409,956 | 568 | 99.8616% | 154,253 | 410,524 |
| GET100 / 2 | 406,333 | 573 | 99.8592% | 151,407 | 406,906 |
| PUT100 / 1 | 130 | 0 | 100.0000% | 130 | 130 |
| PUT100 / 2 | 130 | 0 | 100.0000% | 130 | 130 |
| Mixed / 1 | 139,402 | 1,477 | 98.9516% | 46,484 | 140,879 |
| Mixed / 2 | 136,384 | 1,765 | 98.7224% | 46,318 | 138,149 |

These read deltas span the whole trial; PUT's reads belong to setup and
verification. All prior inline/fallback, public RawRead and grouping predicates
remain unchanged. At drained boundaries, inline plus blocking equals completed
RawRead minus retained backend errors and is bounded by admitted/completed
RawRead. All relevant counters are monotonic; group maximum reaches 64 and read
registry lifetime peak 65 is within 128. Public queued/running/in-flight/bytes,
read queues/groups/in-flight, and candidate async-apply occupancy all drain to
zero. All 63 observed owned process lifetimes exited.

## Evidence and limitations

Raw evidence: `/tmp/kv9-resident-async-write-c64-tmpfs-bracket`.
Session 79693 completed all 36 cohorts and six unchanged checks. Every
additional full, read/group, async-apply, p99/CPU and stage audit passed its
first invocation. No cohort was discarded or rerun; no failed observer attempt
occurred in this increment. Earlier failed observers remain with their own
experiments. No runtime/client/shared-validator or hosted-CI change occurred.

- Complete identities, outcomes, CPU and residual work: `final-audit.json`, SHA-256 `4ab2120912d9f7d27d90fbc6f97dc21b7bdcea0f62519784a200d2417d2aec1e`.
- Per-lifetime async-apply count/peak/drain: `async-apply-audit.json`, SHA-256 `7052dc62d92e5d006a407a3eeb49dd2750ce25edb5c6f99b3c7a0a7fd4bc7b12`.
- Read grouping and inline/fallback deltas: `group-counter-summary.json`, SHA-256 `e6a12e3b9d41ba806214e09e9e5fdc12863f0886351a8a9c77bc8f7c4461105d`.
- Each repetition acknowledged p99: `per-repetition-latency-check.json`, SHA-256 `cc224ae5ab282d6b80d514a657d27f635b47dc7b1db643a0a1f3380be31e0256`.
- CPU and whole-trial sync observations: `latency-cpu-stage-details.json`, SHA-256 `fd8498bcd29881fa2cc754ece67997825be4924fc9b201ce40414b6c46fd89bf`.
- All per-voter write-stage histogram/count deltas: `write-stage-audit.json`, SHA-256 `550373b597302c9052624cf0a9c28bc5e3372e51481977f491454d2dd0689ff0`.

Inventory `/tmp/kv9-resident-async-write-c64-tmpfs-inventory.json` contains **1,714 files /
2,369,292,591 bytes**, SHA-256
`eddd8086c5eb7be1f006c6a5a435978d8ae435547c7d68664b97bdf59f2bf916`. Every original file was independently reopened and
checked against its recorded length/hash; verification is
`/tmp/kv9-resident-async-write-c64-tmpfs-inventory-verification.json`.

All original matrices, complete outcomes/histograms/CPU, source/build/client
identities, affinity guards, tmpfs mounts, data and owned helpers are retained.
The report is outside the manifest to avoid recursive hashing. First baseline
retains 51 tmpfs files / 711,790,778 bytes, candidate 54 / 821,531,430, and final
baseline 51 / 709,330,694. Copies were hash-verified before removing owned
scratch; post-run disk copies do not change the runtime's volatility.

The gain is specific to this fixed c64 tmpfs workload and its short repetitions.
Exact resident Chaos evidence is separate and is not relabeled as `a00e39f`
fault acceptance. Durable-disk, MinIO/Chaos and proof/composition gates remain
necessary before any corresponding production-promotion claim.
