# Engine CRC table: write throughput and latency improve

Candidate `ca0002c7` passes the first isolated c64 write screen. Against the
same-recording `5ee897a` control, pooled point PUT throughput increases
**4.476%**, and BatchPut(64) throughput increases **38.486%**. Mean and p99
improve in both original repetitions. This is a useful write increment to
continue through recovery and broader workload acceptance; it is not general
promotion or Redis parity.

The [CPU profile](WRITE-APPLY-CPU-PROFILE.md) identified the old engine CRC loop
as 7.807% of point-write and 41.903% of batch-write selected CPU samples.
The [candidate and scoped proof](https://github.com/c4pt0r/kv9/blob/ca0002c7f8e9ee6f595efcc9f4151085ccce87cb/docs/CRC32-TABLE.md)
replace eight bit steps per byte with a constant byte table. Checksum bytes,
coverage, fragments, WAL formats and commit/apply/sync ordering are unchanged.
No read-credit candidate, ownership rewrite or RPC change is combined here.

## Same-recording results

Two ten-second repetitions run the complete six-cell order forward and then
reverse: point PUT and BatchPut(64), each with control, candidate and Redis.
All runs use c64, 4,096 keys plus sentinel, 128-byte values, 128 warmup calls,
seed 71, a ten-million-call cap and a 1,500-ms deadline. Clients use CPUs 0-1;
all three KV9 voters, or Redis, share CPUs 2-5. Helpers and three owned test
containers use CPUs 6-15,22-31 during timing; their original CPU settings are
restored afterward. Other host services remain unconstrained.

| Operation | Role | Successful calls/s | Successful keys/s | Mean us | p95 us | p99 us |
| --- | --- | ---: | ---: | ---: | --- | --- |
| Point PUT/SET | Control | 118,888.863 | 118,888.863 | 538.189 | 737.280-745.471 | 860.160-868.351 |
| Point PUT/SET | CRC candidate | 124,210.707 | 124,210.707 | 515.117 | 704.512-712.703 | 827.392-835.583 |
| Point PUT/SET | Redis | 493,585.759 | 493,585.759 | 129.511 | 163.840-165.887 | 235.520-237.567 |
| BatchPut/MSET(64) | Control | 9,795.438 | 626,908.052 | 6,531.843 | 9,043.968-9,175.039 | 11,665.408-11,796.479 |
| BatchPut/MSET(64) | CRC candidate | 13,565.331 | 868,181.199 | 4,716.673 | 6,750.208-6,815.743 | 8,912.896-9,043.967 |
| BatchPut/MSET(64) | Redis | 94,638.541 | 6,056,866.634 | 671.487 | 958.464-966.655 | 1,007.616-1,015.807 |

All latency values describe a complete logical call. Percentiles are histogram
bucket intervals. Pooled rates divide summed calls/keys by summed complete
cohort elapsed time; means divide summed latency by summed call count. Pooled
percentiles merge the original buckets, without percentile averaging or
division by batch size.

| Operation | Repeat 0 QPS change | Repeat 1 QPS change | Pooled mean change |
| --- | ---: | ---: | ---: |
| Point PUT | +4.853% | +4.099% | -4.287% |
| BatchPut(64) | +40.267% | +36.721% | -27.790% |

Point p99 decreases in both repeats: 851.968-860.159 to 819.200-827.391 us,
and 860.160-868.351 to 835.584-843.775 us. Batch p99 decreases from
12.059-12.190 to 8.323-8.389 ms and from 11.272-11.403 to 9.830-9.961 ms.
The second batch tail remains worse than the first; both original populations
are retained. Two repeats do not establish statistical significance or a
sustained capacity limit.

The remaining Redis throughput ratios are **3.974x for point writes and
6.977x for 64-key batch writes**. The memory reference uses standalone Redis
7.0.15, save/AOF disabled, one I/O thread and no pipelining. KV9 retains three
voters, normal quorum and sync calls, with volatile tmpfs WAL. These are not
equal durability or fault semantics. The two-core batch Redis client is near
its budget; the observed rate is not an intrinsic Redis server ceiling.

## Accounting and source identity

The separate six-cell correctness smoke passes with 1,506,767 measured calls.
The first timing run exits zero, all twelve cohorts finish, and the unchanged
independent auditor accepts **17,094,412 measured calls / 165,788,461 input
keys**, all successful with one observed attempt per call. Refusals, unknown
writes, read failures, client rejections and dropped slots are zero. Setup has
eight additional leader-routing attempts, all logically successful; these are
outside measurement and remain in the all-phase accounting.

The audit confirms 40 exited process lifetimes, 24 fresh drains, 24 exact voter
writer/listener bindings, 2,304 resource samples, 2,336 source-file checks,
1,291 retained files / 35,338,645,112 bytes and exact outer CPU/namespace
restoration. Full paginated deterministic data readback passes. Aggregate
benchmark reports do not constitute a complete linearizability history.

| Role | Revision | Binary SHA-256 |
| --- | --- | --- |
| Control server | `5ee897a2f58c57bdf17ea1757c96224adf0f0dbb` | `0d5ffa081482945d213b88aef12222afab44a44ed03bf46cf27bd292db7b1711` |
| CRC server | `ca0002c7f8e9ee6f595efcc9f4151085ccce87cb` | `b33b5d302f901aac5cf6a95449b9bb5dd68473747e011220dbed7aed2e591d13` |
| Native benchmark client | `0be806d9671e2c50701a64aa7889c8859b7648ba` | `8da9af469f962a938027d1970141bbe4622f7d42b2b795f720e288fb3f8d5957` |
| Redis benchmark client | `0be806d9671e2c50701a64aa7889c8859b7648ba` | `5a8ac274b8cc6548a072f5305b04936a08cc4d1ba84d50150f675bab563049af` |

The original CRC release manifest hash remains
`8c2ea115afc1ec8b6c82224dc6449d1d2439f5f27b5758e45090589752d186e8`.
The frozen driver and auditor have separate pure contracts (8 and 15 passing),
and exact launch inputs are frozen before timing. No builds, tests, profiles,
faults or analysis run during measurement. Source, accounting, storage and
cleanup predicates are unchanged from the accepted v3 infrastructure.

The first ancillary prelaunch check rejected the unrelated existing Redis
service; its identity was recorded and preserved under the shared-host scope.
A subsequent smoke-summary reader used a native field name for the Redis
attempt count and failed before timing. Both failures remain retained; the
corrected reader respects the original schema, and timing starts only after
the smoke gate passes. Neither failure caused a fixture rerun or changed a
frozen validator. The auditor's first compact closeout also retains its
subsequently corrected percentile unit labels; the actual audited values and
statistics are unchanged.

## Remaining gates

Local source validation already passes 144 engine tests, 709 workspace tests,
all-target Clippy and formatting. The source-mapped Lean proof checks 22
theorems, all 256 actual compiled entries and five rejected controls, with
only the allowed foundation axioms.

The same exact server binary now also passes the leader-loss/original-directory
restart fixture and its unchanged independent auditor on both streaming and
unary RPC. All **368 complete-history calls** remain accounted for: **335 OK
and 33 unknown outcomes** across the injected failures. Both atomic batch and
overlapping point histories pass; the auditor confirms two client and five
server lifetimes, and fresh drained publications from all three voters in
both cases. Unknown outcomes are retained and checked, not silently retried
or reclassified as successful acknowledgements. This is ordinary WAL process
recovery, not power-loss testing or Chaos Mesh acceptance.

The correctness client is built separately from the same clean `ca0002c7`
source (590 identical inputs). A fresh recovery artifact directory preserves
the original timed server, Cargo output and manifest unchanged, adding the
actual client build and an explicitly identified aggregate manifest
`769f0b474d219ae7fa42ceb4e0bb0502dbd906f150ea1f70fe1f0889df13de8e`.
The [assembly record](../scripts/redis-reference/crc-write-screen-v1/process/build/assembly.json)
and [process audit](../scripts/redis-reference/crc-write-screen-v1/process/audit/audit.json)
bind this distinction and the actual histories.

Actual Chaos Mesh injection and the broader c1/c64 read/mixed/write matrix are
next. General/default promotion remains open until these gates finish.
Owned-buffer movement and index changes remain subsequent
experiments, with independent measurements and correctness obligations.

The [evidence index](../scripts/redis-reference/crc-write-screen-v1/index.json)
binds 95 exact copies (18,938,740 bytes), including all twelve original client
reports/resource coverage, independent audit, launch gates and pooled
statistics. Large raw WALs remain local. See the
[complete readout](../scripts/redis-reference/crc-write-screen-v1/statistics/READOUT.md)
and [all populations](../scripts/redis-reference/crc-write-screen-v1/statistics/statistics-first.json).
CI remains local; no hosted workflow is dispatched.
