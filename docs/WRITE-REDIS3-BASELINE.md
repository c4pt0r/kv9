# Selected KV9 writes versus Redis with two replicas

Measured 2026-09-12. The selected three-voter KV9 reaches 136,519.558 point
writes/s at concurrency 64, against 229,760.166 with Redis WAIT 1 and
232,465.084 with WAIT 2. BatchPut(64) reaches 887,520.285 input items/s,
against about four million. The loaded batch tail is the largest immediate
problem: KV9 p99 is 9.437–9.568 ms versus 1.245–1.294 ms for Redis.
This establishes the write baseline; it is not a new optimization result.

The independent audit accepts 12 two-second smoke cohorts and 24 ten-second
timed cohorts in two opposite target orders. All 18,993,624 measured logical
calls succeed with one data-command attempt, covering 259,615,761 input items.
There are no measured unknown writes, errors or dropped slots. Initial native
setup contains one explicitly safe extra attempt; that is outside measurement.

## Matched configuration and limits

All targets run on one host. Every target has three server processes sharing
CPUs 2–5; client workers share CPUs 0–1. Background fixture containers and
helpers are isolated onto CPUs 6–15 and 22–31. No build, profiler or codec
runs during timing; benchmark retention compression happens after writer
reaping and before the next cohort. The original container CPU allocations
and namespace maps are restored and independently checked afterward.

The workload has 4,096 mutable keys plus an unchanged sentinel, 128-byte values,
seed 71, 128 warmup calls, one call in flight per connection, concurrency 1 or
64, and a ten-million-call cap. Point writes use native Put versus SET;
batches use atomic BatchPut(64) versus MSET(64). Both Redis panels use one
primary and two replicas. The client sends the write and WAIT together on the
same connection and consumes both under one 1,500-ms absolute deadline;
WAIT has a 1,000-ms timeout. An uncertain write is never retried.

KV9 uses the selected default release with its normal Raft commit, apply,
response and synchronization fences, with explicitly volatile tmpfs WAL.
Redis 7.0.15 uses appendonly=no, save disabled, no eviction, a 64-MB limit and
one I/O thread. WAIT 1 requires at least one replica acknowledgment; WAIT 2
requires both. [Redis documents](https://redis.io/docs/latest/commands/wait/)
that WAIT applies to earlier writes on the connection and does not make Redis
strongly consistent. This panel does not establish equal durability, disk
fsync cost, power-loss recovery, independent-host tolerance or sustained capacity.
It includes no fault injection and is not a full-history linearizability test.

Redis topology, process identities, effective configuration, replication IDs
and healthy links are checked before and after each run. Full/partial resync
counters do not change. After client exit, a fresh SET + WAIT 2 barrier precedes
byte-identical verification of all 4,097 workload keys on both replicas.
These observations do not prove continuously healthy replication between them.
Native final scans independently validate the deterministic value bodies,
write-key membership and sentinel; they do not identify the exact issued nonce
set or reconstruct a complete operation history.

## Results

Rates pool successful work over the sum of both cohort durations. Latency is
whole logical call latency, including replication confirmation for Redis.
Percentiles merge original histograms and show bucket bounds; no percentiles
are averaged. One batch call contains 64 input items. CPU figures are estimated
cores, recomputed from retained in-window process user/system ticks; all three
server processes are included. Sample-selection gaps at measurement boundaries
are at most 150 ms. Individual process probes are sequential and can extend
outside the exact client interval; CPU figures are sampled interval estimates,
not per-stage timings.

| Workload | c | Target | Calls/s | Items/s | Mean us | p50 us | p95 us | p99 us | Server CPU | Client CPU |
| --- | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| point | 1 | kv9 | 19,290.290 | 19,290.290 | 51.739 | 50.688–51.199 | 59.392–59.903 | 66.560–67.583 | 2.522 | 0.149 |
| point | 1 | redis-wait1 | 76,115.856 | 76,115.856 | 13.016 | 12.800–12.927 | 13.952–14.079 | 16.256–16.383 | 1.261 | 0.488 |
| point | 1 | redis-wait2 | 64,539.864 | 64,539.864 | 15.371 | 15.104–15.231 | 16.384–16.639 | 19.200–19.455 | 1.133 | 0.422 |
| point | 64 | kv9 | 136,519.558 | 136,519.558 | 468.670 | 446.464–450.559 | 638.976–647.167 | 753.664–761.855 | 3.560 | 0.423 |
| point | 64 | redis-wait1 | 229,760.166 | 229,760.166 | 278.386 | 278.528–282.623 | 307.200–311.295 | 323.584–327.679 | 1.187 | 1.157 |
| point | 64 | redis-wait2 | 232,465.084 | 232,465.084 | 275.150 | 274.432–278.527 | 282.624–286.719 | 299.008–303.103 | 1.184 | 1.133 |
| batch64 | 1 | kv9 | 5,827.820 | 372,980.449 | 171.481 | 165.888–167.935 | 190.464–192.511 | 217.088–219.135 | 1.858 | 0.143 |
| batch64 | 1 | redis-wait1 | 23,676.846 | 1,515,318.119 | 41.115 | 40.448–40.959 | 43.520–44.031 | 52.224–52.735 | 1.065 | 0.418 |
| batch64 | 1 | redis-wait2 | 22,388.937 | 1,432,891.973 | 43.543 | 43.008–43.519 | 47.104–47.615 | 56.320–56.831 | 1.019 | 0.397 |
| batch64 | 64 | kv9 | 13,867.504 | 887,520.285 | 4613.642 | 4325.376–4390.911 | 6815.744–6881.279 | 9437.184–9568.255 | 3.320 | 0.412 |
| batch64 | 64 | redis-wait1 | 62,707.665 | 4,013,290.580 | 1015.724 | 1032.192–1040.383 | 1163.264–1179.647 | 1245.184–1261.567 | 1.999 | 1.362 |
| batch64 | 64 | redis-wait2 | 62,485.523 | 3,999,073.448 | 1019.294 | 1032.192–1040.383 | 1179.648–1196.031 | 1277.952–1294.335 | 2.001 | 1.361 |

The complete [machine-readable summary](write-redis3-baseline-v1/summary.json)
retains every repetition, report hash, latency bound and per-process CPU rate.
Across the two orders, throughput spreads range from 0.069% to 1.882%; the
largest is KV9's loaded batch. Two short runs do not establish a confidence
interval or a general ranking between WAIT 1 and WAIT 2. WAIT 2's slightly
higher loaded point rate here is an observed result, not a claimed advantage
of waiting for more replicas.

Relative to KV9, Redis WAIT 1 throughput is 3.946× at c1 point, 1.683× at c64
point, 4.063× at c1 batch and 4.522× at c64 batch. The corresponding WAIT 2
ratios are 3.346×, 1.703×, 3.842× and 4.506×. KV9's c64 server CPU totals are
3.560 cores for point and 3.320 for batch, versus about 1.19 and 2.00 for Redis.
The evidence supports investigating write CPU cost and loaded batch latency;
it does not attribute the entire gap to CRC or justify removing durability fences.

## Acceptance and retained failures

The exact timed root session is 42336, terminal `2cb856`, exit 0; the smoke
session is 9034, terminal `6e43ea`, exit 0. The repaired independent audit is
session 59024, terminal `b37127`, exit 0. It checks 96 timed process lifetimes,
24 fresh three-voter drains, 24 writer/listener bindings and 4,663 in-window
resource samples. Another 48 owned smoke lifetimes have exited. Original
source, executable, Cargo, configuration, storage and isolation bindings pass.

All native retained WAL bytes are independently decoded and hash-checked:
28,757,958,679 timed logical bytes plus 3,457,580,392 smoke bytes. Timed retention
contains 1,100 original files and 2,200 observed codec lifetimes. Combined
physical smoke/timing retention is 22,904,745,984 allocated bytes. Original
32/96-GiB tmpfs/retained preflights, 16/64-GiB runtime floors, the additional
96-GiB compression floor and all logical/physical retention limits remain intact.

The first enclosing auditor exited 1 (`179a68`) because it read Redis cleanup
fields from native smoke records. The repair also handles the adjacent native
captured-executable-hash schema explicitly, preserving Redis executable-path
checks. Five focused acceptance/refusal controls pass (`75924a`). The other
21 named functions and all timing/retention acceptance predicates are unchanged.
Two relocation literals still identify the original executed driver and
retention producer. The repaired audit reads the same original cohorts once;
no workload was rerun. The failed auditor, exact diff, controls and receipts
remain in the evidence. Preparation's missing sibling history import and the
summary draft's empty histogram indexing error are also retained.

## Source attribution and next step

| Role | Revision | Executable SHA-256 |
| --- | --- | --- |
| Selected KV9 server | `11113f68f6a5df77da1ffb4fcec850953716ffa3` | `dd028cb2f61633dda133b05d814a6d173a79a83145d8ced2f81dc0cf8e0f33bc` |
| Fixed native v3 client | `0be806d9671e2c50701a64aa7889c8859b7648ba` | `8da9af469f962a938027d1970141bbe4622f7d42b2b795f720e288fb3f8d5957` |
| Redis v4 client | `fc07bd64f02e4079e248388062d9ffc7148c295c` | `504c55e2ef0ddf350753e380171f1e418ea5070dbd25752f42dc75fec913b9de` |

The report is published on an evidence branch based on the exact selected
server revision. Its added documents do not redefine the measured source.
The [evidence inventory and readback](write-redis3-baseline-v1/README.md) bind
original reports, helpers, topology/dataset observations and audit outputs;
large local WAL objects and executables remain separately hash-bound.

Next, compare the isolated CRC slicing candidate `e748620` with this selected
server using the same fixed native client and write protocol. Its source-mapped
proof, workspace tests, clean release and ordinary recovery already pass;
its four-binary Chaos image is built, probed and loaded into Kind. Actual
candidate Chaos Mesh histories and matched A/B performance remain pending.
A write-only screen must be followed by full point/batch and mixed-read
regressions and actual fault acceptance before default promotion. Preserve
Raft, synchronization, unknown outcomes and loaded p99. Reads remain at
ThinLTO/Safe ReadIndex. Dynamic multi-Raft and automatic range splits follow
the write phase with their existing storage, proof and recovery dependencies.
No hosted CI was dispatched and no original industrial checklist item closes.
