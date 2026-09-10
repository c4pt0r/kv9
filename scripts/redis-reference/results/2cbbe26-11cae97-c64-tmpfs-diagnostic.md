# Sealed groups versus resident reads: volatile c64 diagnostic

Exact resident-read candidate `11cae97` increased acknowledged GET throughput by
**14.6%** and mixed throughput by **4.4%** against the pooled contemporaneous
`2cbbe26` before/after baseline. **PUT was 1.3% lower**; this experiment does
not establish absence of regressions. Acknowledged p99 histogram buckets did
not change. This is a short local diagnostic, not production acceptance or a
general capacity claim.

**All runtime data was on volatile tmpfs, without disk or power-loss durability.**
Normal quorum and sync calls remained enabled. Standalone Redis had persistence
disabled and no replicas; its guarantees differ. Candidate GET reached about
27.1% of its contemporaneous Redis reference, and PUT about 13.5%; the gap to
Redis remains substantial.

## Exact artifacts and protocol

| Artifact | Revision or SHA-256 |
|---|---|
| Sealed-group baseline | `2cbbe26a3d4273c6d265fa40c8b56c548b1a59ec` |
| Resident-read candidate | `11cae977f15df0912c2a35561480d45447bae660` |
| Groups release executable | `b460443c768847e6e931e9915d6959fe359ba045ea807e1309e0238c44ecc05f` |
| Resident release executable | `413cfdb20bd947d56a51c792c1d8ab69a676af98a50721649c7b0eceb54f9f33` |
| Unchanged Ready client | `b47d5f408cd9a36ce7a8117917eb0fba530b0616125b4fff38980a255ddd12dc` |
| Unchanged Redis client | `57947d5a94c739dd58ca98e2c9a85c40814040826f13f99e85cd0e2c7fb2f636` |

Root supplied clean exact release builds with features `[]` from
`/tmp/kv9-redis-read-groups-build` and `/tmp/kv9-redis-resident-read-build`.
Root builds used CPUs 6–31 and its shared target; this diagnostic did not build
or modify runtime binaries. The persistent client remains the separately
identified Ready `892b2a1` artifact, not a same-revision client build. All 410
baseline and 412 candidate build-manifest source inputs were checked before
and after execution and copied into the evidence. Their aggregate inventory
hashes are `9dc1040001b71f3aa92bdc86871b50ea2e9b71422df1e7974505e125751b4586`
and `34b6fcebd05d90f799d912425dd64e247ffe758e8ae88bbb7274670ace648ef3`.

The unchanged protocol ran groups-before, resident candidate, then groups-after.
Each matrix has GET100, PUT100 and GET50/PUT40/DELETE10; each uses two three-second
repetitions and separate KV9/Redis executions, totaling **36 cohorts**. Target
order reverses in repetition two. Every cohort uses 64 workers, 64 hot keys,
23-byte keys, 128-byte values, one outstanding request per worker and 32 warmup
operations. KV9 retains 1,500-ms deadlines, six maximum attempts for explicit
NotLeader routing, and no retries of uncertain writes. Redis has no retries or
pipelining. Every trial ended by duration, below its operation cap.

The unchanged affinity guard checked outer CPUs 0–31, clients 0–1 and all three
voters 2–5 before execution and independently observed actual placement and
executable identities before measurement. Communication uses IPv4 loopback on
one shared host. Root release, process checks and correctness archival completed
before timing; the proof scheduler was paused with no active heavy descendants.
Root/proof heavy work resumed after all timing and owned process cleanup ended.
These are logical CPU masks, not isolated hosts or independent failure domains.

Redis 7.0.15's recorded configuration is `save ""`, `appendonly no`, zero replicas
and one I/O thread. Missing GETs produce successful nil/None results in mixed
traffic; the unchanged KV9 performance client does not export hit/miss counts.

## Throughput, latency and CPU

Rates divide acknowledged operations by cohort elapsed time, including drain.
Parentheses retain repetition one / two. Ratios use all four baseline
repetitions, without discarding the slower final GET repetition.

| Workload | Groups-before ops/s | Resident ops/s | Groups-after ops/s | Resident / pooled baseline |
|---|---:|---:|---:|---:|
| GET100 | 119,586.6 (119,138.0 / 120,035.2) | 136,227.1 (136,887.0 / 135,567.1) | 118,208.8 (120,509.0 / 115,908.5) | 1.146 |
| PUT100 | 66,810.7 (66,682.9 / 66,938.5) | 66,203.1 (66,630.3 / 65,775.8) | 67,368.8 (67,313.6 / 67,424.0) | 0.987 |
| Mixed | 79,782.8 (79,851.1 / 79,714.4) | 83,435.7 (83,622.4 / 83,248.9) | 80,012.8 (80,464.8 / 79,560.7) | 1.044 |

Contemporaneous standalone Redis references:

| Paired matrix | GET/s | PUT/s | Mixed/s |
|---|---:|---:|---:|
| Groups-before | 503,211.3 (503,008.2 / 503,414.5) | 486,928.0 (486,366.2 / 487,489.7) | 497,995.3 (500,627.6 / 495,363.1) |
| Resident | 503,462.3 (501,061.6 / 505,862.8) | 491,936.4 (491,666.2 / 492,206.5) | 499,728.3 (504,062.8 / 495,393.7) |
| Groups-after | 506,473.8 (509,298.9 / 503,648.8) | 488,601.9 (486,508.4 / 490,695.4) | 497,628.3 (496,365.4 / 498,891.2) |

Every individual KV9 GET repetition has acknowledged p99 in
**0.524288–1.048575 ms**; every PUT/mixed repetition is in
**1.048576–2.097151 ms**. Every Redis repetition has p99 in
**0.131072–0.262143 ms**. Pooled pairs have the same buckets. These are histogram
bounds, not exact percentile points, and show no p99 bucket improvement.

| Workload | Groups-before server cores (repetitions) | Resident server cores | Groups-after server cores |
|---|---:|---:|---:|
| GET100 | 3.102 / 3.211 | 3.091 / 3.150 | 3.115 / 3.187 |
| PUT100 | 3.606 / 3.603 | 3.603 / 3.613 | 3.607 / 3.616 |
| Mixed | 3.547 / 3.566 | 3.528 / 3.532 | 3.562 / 3.567 |

Resident GET clients consumed 1.413 / 1.397 of two allowed cores; busiest
threads used 0.703 / 0.697 cores. PUT clients used 0.785 / 0.766 cores and mixed
clients 0.942 / 0.943. Across all Redis cohorts, client CPU stayed below 1.409
cores and busiest-thread CPU below 0.710. The observed clients have CPU headroom
at c64; this does not identify every bottleneck or establish saturation capacity.

## Actual execution, grouping and complete outcomes

All **4,906,570 KV9** and **26,857,975 Redis** measured logical operations
succeeded. There were zero final refusals, unknown outcomes, transport/read
failures or client rejections. Successful logical operations can include
permitted explicit NotLeader routing attempts; those attempts and full outcome
populations remain in the original reports.

The candidate's actual before/after counters show resident execution and the
intended occasional blocking fallback:

| Workload / repetition | Completed inline | Blocking submitted | Inline fraction | Admitted groups | Admitted members |
|---|---:|---:|---:|---:|---:|
| GET / 1 | 410,362 | 505 | 99.8771% | 149,861 | 410,867 |
| GET / 2 | 406,461 | 444 | 99.8909% | 143,719 | 406,905 |
| PUT / 1 | 130 | 0 | 100.0000% | 130 | 130 |
| PUT / 2 | 130 | 0 | 100.0000% | 130 | 130 |
| Mixed / 1 | 124,615 | 1,304 | 98.9644% | 40,553 | 125,919 |
| Mixed / 2 | 123,906 | 1,423 | 98.8646% | 40,131 | 125,329 |

These deltas cover the whole trial: setup, warmup, measured operations, drain
and verification. PUT's 130 point reads per repetition are setup/verification,
not timed GETs. Fractions use inline plus blocking counts as the denominator;
they are not measurement-only dispatch savings. The opportunistic implementation
falls back if lifecycle or snapshot locks cannot be acquired immediately.
Counters do not identify which lock caused each fallback.

Every voter's counters were monotonic. At drained boundaries, inline plus
blocking was bounded by admitted/completed public RawRead; for this point-GET
fixture it exactly equaled completed RawRead minus recorded backend errors.
The errors remained visible on routed attempts and were not relabeled as
executed reads. Group/member counts show actual grouping; maximum group size
reached 64, within the configured bound. Maximum size and registry peak are
process-lifetime observations. Registry peak 65 remains within its 128 limit;
64 client workers do not imply a 64-reservation internal peak.

All before/after candidate public and async queues drained. Every matrix's
after-verification snapshot has zero active groups, active members, queued and
in-flight reads, and zero public queued/running work, request count and encoded
bytes on all three voters. All 63 owned executing process lifetimes exited.

## Evidence and retained observer failures

Original evidence is `/tmp/kv9-groups-resident-c64-tmpfs-bracket`. The three
matrices and all six unchanged independent checks passed on their first
attempt. No timed cohort was rerun or discarded.

The separately prepared extra observer initially required zero blocking
fallback. Its first invocation correctly rejected the recorded nonzero count
against that mistaken requirement. `full-audit-first.py` and
`full-audit-first.log` preserve the exact observer and rejection;
`observer-correction.json` explains the source-supported correction. The revised
observer checks monotonic counters, positive inline GET/mixed activity, the
public RawRead population bounds and full drain. Production, workload and shared
validators were unchanged. The initial inventory builder also rejected the
system Redis invocation path because it is a symlink; its source/log are
retained. The corrected inventory explicitly records the resolved executable
and symlink provenance, without changing Redis or rerunning any trial.

- Complete independent identity, outcome, CPU and drain audit: `final-audit.json`, SHA-256 `33e862de351ecbfd382e98a8eb22bcaaa79a4c765e8eeb5915e3b0eb9302382f`.
- Per-voter grouping and execution deltas: `group-counter-summary.json`, SHA-256 `24a9513f93b7c7bbb8a3e134f69d9c1464152d1ce536a7acc11fd3372e4300b4`.
- Latency/CPU and whole-trial sync-stage observations: `latency-cpu-stage-details.json`, SHA-256 `6c2d871cd1de7170e036856d6c4e1c9dc790b4cce5f827d0820bddb537b117a7`.
- Every repetition's acknowledged p99 bucket: `per-repetition-latency-check.json`, SHA-256 `8a9e38f58630b707028b7604ec480910a314a7246bd34dd31dcd4fd47f2059d2`.

The complete inventory is `/tmp/kv9-groups-resident-c64-tmpfs-inventory.json`: **1,706 files / 2,244,613,459 bytes**, SHA-256 `b7c6b0c3fc287ef236b6177320263ff617d479613565d331f7d5af09a20c7c23`.
Every original was reopened and its size/hash independently verified;
`/tmp/kv9-groups-resident-c64-tmpfs-inventory-verification.json` records the
second pass. Exact source copies, release/client builds, raw matrices,
histograms, mount observations, CPU/identity records, all observer attempts
and unchanged helper sources are included. The report itself is excluded to
avoid a circular hash.

Each matrix retained 51 tmpfs data files after byte/hash verification before
scratch cleanup: 701,894,049 bytes before, 708,743,035 for resident reads and
706,493,218 after. These later disk copies do not change runtime volatility.
All original retained evidence remains available. No hosted CI, GitHub mutation,
runtime edit or shared-validator edit was performed by this task.

This experiment does not replace durable-disk, MinIO, actual Chaos Mesh or proof
composition acceptance. Exact-2cb NetworkChaos evidence is not resident-runtime
fault acceptance. The PUT decrease and remaining Redis gap remain visible
inputs to the next write-path investigation.
