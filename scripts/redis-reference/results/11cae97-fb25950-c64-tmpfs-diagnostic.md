# Write serialization allocation: volatile c64 diagnostic

Exact serialization candidate `fb25950` showed pooled acknowledged-throughput
differences of **GET +0.65%, PUT +1.80% and mixed +1.91%** against the
contemporaneous resident `11cae97` before/after bracket. GET was essentially
equal to the final baseline, and the PUT repetitions overlap the baseline
range. This small local difference does not establish a substantial performance
gain or a meaningful step toward Redis. No performance promotion is claimed.

**This is volatile tmpfs, with no disk or power-loss durability.** Normal
quorum and sync calls remained enabled. Redis is a standalone reference with
persistence disabled and zero replicas, so its guarantees differ. These short,
closed-loop observations are not production acceptance or a general capacity
estimate. The earlier CPU profile's unattributed allocation frames are not
assigned to these serialization changes by this measurement.

## Exact artifacts and unchanged protocol

| Artifact | Revision or SHA-256 |
|---|---|
| Resident baseline | `11cae977f15df0912c2a35561480d45447bae660` |
| Serialization candidate | `fb2595030f7cc6d7e12a81afa13a027545bc9afe` |
| Baseline release executable | `413cfdb20bd947d56a51c792c1d8ab69a676af98a50721649c7b0eceb54f9f33` |
| Candidate release executable | `e187af09941fab74098aa138ddeb0cfd87899cd0926088618ace2687825b30be` |
| Fixed Ready client | `b47d5f408cd9a36ce7a8117917eb0fba530b0616125b4fff38980a255ddd12dc` |
| Fixed Redis client | `57947d5a94c739dd58ca98e2c9a85c40814040826f13f99e85cd0e2c7fb2f636` |

Root supplied clean exact release builds, features `[]`, at
`/tmp/kv9-redis-resident-read-build` and
`/tmp/kv9-redis-write-serialization-build`. This task did not build or modify
runtime/client binaries. All 412 baseline and 413 candidate build-manifest
source inputs were independently rechecked and copied into evidence. Their
aggregate source digests are
`34b6fcebd05d90f799d912425dd64e247ffe758e8ae88bbb7274670ace648ef3` and
`d5d8255a26efc009ff1a390da898ece6c77b6a3320ed252fd3e26a57563a90ff`.
The client remains the separately identified Ready `892b2a1` release artifact
with its original build/source provenance, not a same-revision client build.

The frozen protocol ran resident-before, candidate, then resident-after.
Each matrix has GET100, PUT100 and GET50/PUT40/DELETE10, two three-second
repetitions and separate KV9/Redis executions: **36 cohorts**. Repetition two
reverses target order. All cohorts use 64 workers, 64 hot keys, 23-byte keys,
128-byte values, one outstanding request per worker and 32 warmup operations.
KV9 retains 1,500-ms deadlines, six maximum attempts for explicit NotLeader
routing and no retries of uncertain writes. Redis uses no retries or
pipelining. All trials stopped by duration below the operation cap.

The unchanged guard verified outer CPUs 0–31, clients 0–1 and all three voters
2–5; actual executable identities and masks were observed before measurement.
Root builds and archival completed, and the proof scheduler's complete subtree
was quiescent before timing. Root/proof heavy work resumed only after all
measured cohorts and owned process cleanup ended. Communication uses IPv4
loopback on one shared host; logical masks do not create isolated hosts or
independent failure domains.

Recorded Redis 7.0.15 configuration remains `save ""`, `appendonly no`, zero
replicas and one I/O thread. Missing GETs are successful nil/None in mixed
traffic; the unchanged KV9 performance client does not expose hit/miss counts.

## Throughput, latency and CPU

Rates are acknowledged operations divided by cohort elapsed time, including
drain. Parentheses show repetition one / two. The ratio pools all four baseline
repetitions; every result remains visible.

| Workload | Resident-before ops/s | Serialization ops/s | Resident-after ops/s | Candidate / pooled baseline |
|---|---:|---:|---:|---:|
| GET100 | 135,445.4 (135,266.7 / 135,624.2) | 137,157.4 (137,200.5 / 137,114.2) | 137,102.0 (136,812.1 / 137,391.8) | 1.006 |
| PUT100 | 67,009.5 (66,243.9 / 67,775.6) | 68,010.4 (67,392.5 / 68,628.2) | 66,605.2 (66,717.2 / 66,493.3) | 1.018 |
| Mixed | 83,973.2 (84,097.6 / 83,848.7) | 85,526.8 (85,903.9 / 85,149.8) | 83,883.2 (84,452.2 / 83,314.2) | 1.019 |

Contemporaneous standalone Redis references:

| Paired matrix | GET/s | PUT/s | Mixed/s |
|---|---:|---:|---:|
| Resident-before | 508,518.8 (503,976.9 / 513,060.4) | 489,196.0 (491,860.8 / 486,531.3) | 500,604.6 (503,405.3 / 497,804.0) |
| Serialization | 509,076.9 (505,289.5 / 512,864.0) | 493,819.5 (493,184.6 / 494,454.5) | 501,555.3 (497,972.4 / 505,138.1) |
| Resident-after | 501,826.0 (502,168.2 / 501,483.7) | 492,017.5 (490,326.5 / 493,708.4) | 501,680.7 (501,502.8 / 501,858.5) |

Every individual KV9 GET repetition has acknowledged p99 in
**0.524288–1.048575 ms**; every PUT/mixed repetition remains in
**1.048576–2.097151 ms**. All Redis repetitions have p99 in
**0.131072–0.262143 ms**. Pooled pairs have the same bounds. These are histogram
buckets, not exact percentile points; no p99 bucket improvement is established.

| Workload | Resident-before server cores (repetitions) | Serialization server cores | Resident-after server cores |
|---|---:|---:|---:|
| GET100 | 3.060 / 3.126 | 3.095 / 3.134 | 3.074 / 3.174 |
| PUT100 | 3.604 / 3.630 | 3.579 / 3.603 | 3.603 / 3.630 |
| Mixed | 3.513 / 3.530 | 3.517 / 3.484 | 3.527 / 3.509 |

Candidate GET clients used 1.423 / 1.416 cores (busiest threads 0.708 / 0.708);
PUT clients 0.793 / 0.807 (0.397 / 0.402), and mixed clients 0.970 / 0.963
(0.490 / 0.480). Across Redis cohorts, client CPU stayed below 1.412 cores and
busiest-thread CPU below 0.708. These observations show client CPU headroom
within two allowed cores, without identifying every bottleneck or proving
saturation capacity.

## Complete outcomes and retained read behavior

All **5,189,120 KV9** and **26,992,217 Redis** measured logical operations
succeeded. There were zero final refusals, unknown outcomes, transport/read
failures or client rejections. Permitted explicit NotLeader routing attempts
remain in the complete client reports; successful logical completion does not
imply a single RPC attempt.

Candidate counter observations:

| Workload / repetition | Completed inline | Blocking submitted | Inline fraction | Admitted groups | Admitted members |
|---|---:|---:|---:|---:|---:|
| GET / 1 | 411,160 | 650 | 99.8422% | 149,383 | 411,810 |
| GET / 2 | 410,993 | 557 | 99.8647% | 147,669 | 411,550 |
| PUT / 1 | 130 | 0 | 100.0000% | 130 | 130 |
| PUT / 2 | 130 | 0 | 100.0000% | 130 | 130 |
| Mixed / 1 | 127,991 | 1,385 | 98.9295% | 41,657 | 129,376 |
| Mixed / 2 | 126,765 | 1,455 | 98.8652% | 40,228 | 128,220 |

Counter deltas span setup, warmup, measurement, drain and verification. PUT's
130 reads per repetition belong to setup/verification, not timed GETs. Inline
fractions divide inline by inline plus blocking; occasional blocking fallback
is valid under the opportunistic resident-read contract. No measurement-only
allocation or dispatch savings are inferred from these deltas.

All voters' counters were monotonic. At drained boundaries, inline plus
blocking was bounded by admitted/completed public RawRead and, in this point-GET
fixture, equaled completed RawRead minus retained backend errors. Maximum group
size reached 64; registry peak 65 remains within its limit of 128. Both are
process-lifetime observations, not isolated measurement maxima. Every matrix's
before/after public count, queued/running work, encoded bytes and async/group
occupancy drained to zero. All 63 observed owned process lifetimes exited.

## Evidence and limits

Original evidence: `/tmp/kv9-resident-serialization-c64-tmpfs-bracket`.
All three matrices, six unchanged checks and the additional independent
audits/observers passed their first invocation. No timed cohort failed, was
discarded or rerun; no failed observer attempt occurred in this increment.
Earlier profile and benchmark observer failures remain with those experiments.

- Complete source, identity, outcome, CPU and drain audit: `final-audit.json`, SHA-256 `5e5be9f85793793b6752b0c50d87c4fd500c5876480aef24bcb5b6ad29da336c`.
- Per-voter grouping and execution counters: `group-counter-summary.json`, SHA-256 `f9738feed404d1ff20d5f09713fa569cf1aa6dacca9d91ea2be9d2b62eeef916`.
- Latency/CPU and whole-trial sync-stage observations: `latency-cpu-stage-details.json`, SHA-256 `f12ab9ae0dcbbf7bfa709b9f54661dde9b9ab306103fa8ba368bf90081c0552d`.
- Every repetition's acknowledged p99 bucket: `per-repetition-latency-check.json`, SHA-256 `493ee77a6933fa921937acd3f8c4d56752bf2e37afd50929a4c1bc18a04d642b`.

Complete inventory: `/tmp/kv9-resident-serialization-c64-tmpfs-inventory.json`, **1,703 files / 2,283,087,967 bytes**, SHA-256 `ecbdc2bf87b29320329d6194cc423c4d5bbff5f406bb679304ccb9683d9fb479`.
Every original was reopened and checked against its length/hash; the second
pass is recorded in
`/tmp/kv9-resident-serialization-c64-tmpfs-inventory-verification.json`.
Raw matrices/outcomes/histograms/CPU, exact source/build/client identities,
premeasurement masks, tmpfs mounts/data copies and every helper are retained.
The report is excluded to avoid a circular hash.

Each matrix retains 51 copied/hash-verified tmpfs files: 715,925,468 bytes for
the first baseline, 727,155,096 for the candidate and 712,854,215 for the final
baseline. Owned scratch was removed after verification; these later disk copies
do not change runtime volatility. All original evidence remains. No runtime,
client, shared-validator, GitHub or hosted-CI mutation was performed by this task.

This representation experiment remains isolated without a performance-promotion
claim. The diagnostic does not replace durable-disk, MinIO, Chaos Mesh or proof
composition acceptance, and earlier runtime fault evidence is not relabeled as
acceptance of `fb25950`.
