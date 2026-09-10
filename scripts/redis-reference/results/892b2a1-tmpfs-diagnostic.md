# Ready runtime on volatile tmpfs: storage and CPU diagnostic

This is a **diagnostic-only volatile-storage measurement** of exact
`892b2a178450309859113c942f1738c070130eb5`. It provides **no disk durability or
power-loss guarantee** and is excluded from the durable paired benchmark and
production acceptance. The normal database binary, quorum rules and sync calls
were unchanged; all three voter data directories were placed on tmpfs.

The 24-trial first attempt completed and passed independent report, outcome,
CPU and storage-identity checks. Every measured call was acknowledged; there
were zero UnknownWrite, admission refusal, NotLeader refusal, read failure or
client rejection outcomes. All after-verification queued, running, in-flight
and encoded-byte reservations were zero. These snapshots are asynchronous,
not an atomic quiescence guarantee. All 39 observed owned process identities
exited before final audit.

## Bounded measurement

The wrapper reuses the existing reference driver, exact Ready release and
pinned clients. It changes the database storage location and explicitly limits
the sweep to concurrency 1 and 64: three mixes, two 3-second repetitions and
both targets produce 24 trials. It retains 64 hot keys, 23-byte keys, 128-byte
values, 32 warmup operations, two client runtime threads, the 1,500-ms deadline,
existing admission limits and uncertain-write retry prohibition. Client CPUs
are 0–1; all database voters share CPUs 2–5. The second repetition reverses
concurrency and target order. Calls issued before the deadline drain into the
reported cohort duration. Reaching either operation cap invalidates the trial;
none reached it.

Redis 7.0.15 remains a standalone memory reference with `save ""`, `appendonly
no`, no replicas and one I/O thread. It has one outstanding request per worker,
no pipelining and no retry. Its successful nil GET response is expected in mixed
traffic; KV9's successful `None` response is likewise successful, although its
performance report does not export hit/miss counts. Redis and three-voter KV9
do not provide equivalent semantics or durability here.

| Mix | Concurrency | Volatile KV9 pooled ops/s | KV9 repetitions 0 / 1 | Redis memory pooled ops/s | Unsuccessful calls |
|---|---:|---:|---|---:|---:|
| GET | 1 | 12,501.5 | 13,981.1 / 11,022.0 | 144,462.0 | 0 |
| GET | 64 | 74,657.0 | 79,861.1 / 69,452.3 | 480,853.1 | 0 |
| PUT | 1 | 9,435.4 | 9,811.3 / 9,059.4 | 139,252.3 | 0 |
| PUT | 64 | 59,533.7 | 61,049.0 / 58,018.7 | 457,317.4 | 0 |
| GET 50 / PUT 40 / DELETE 10 | 1 | 10,238.5 | 10,881.0 / 9,596.0 | 139,773.1 | 0 |
| GET 50 / PUT 40 / DELETE 10 | 64 | 57,797.9 | 61,403.2 / 54,193.1 | 475,287.6 | 0 |

Rates pool acknowledged operations over both cohort durations. Full
per-operation and outcome histograms remain in the original reports. The
following percentiles describe successful calls only and are histogram
intervals, not exact estimates.

| Mix at concurrency 64 | KV9 mean ms | KV9 p99 interval ms | Redis mean ms | Redis p99 interval ms |
|---|---:|---|---:|---|
| GET | 0.856 | 1.049–2.097 | 0.133 | 0.131–0.262 |
| PUT | 1.074 | 1.049–2.097 | 0.140 | 0.131–0.262 |
| Mixed | 1.106 | 2.097–4.194 | 0.134 | 0.131–0.262 |

## What the diagnostic supports

The separate [disk matrix](fe650ed-892b2a1.md) measured 32.8 single-call writes/s
and 836.4 writes/s at concurrency 64 with this exact runtime; volatile tmpfs
measured 9,435.4 and 59,533.7. This large difference supports prioritizing
persistence/proposal pipelining for disk throughput. It does not establish how
much of the tmpfs result can be achieved while preserving disk durability.
The runs used different storage and a shorter concurrency sweep, were not
randomized across substrates, and do not establish a precise causal speedup.

CPU observations expose a second limit. At concurrency 64, the three KV9
processes together used 3.580–3.628 cores for GET, 3.625–3.633 for PUT and
3.653–3.671 for mixed traffic out of four allowed logical CPUs. KV9 client CPU
was 1.004–1.065 cores for GET, 0.773–0.824 for PUT and 0.814–0.828 for mixed;
its busiest thread stayed below 0.532 cores. The Redis client used 1.414–1.547
cores out of two, with its busiest thread below 0.774; its server used about
one core. Both clients retained headroom under this protocol. These results
support profiling database protocol/read-path CPU alongside durable-write
pipelining; the remaining gap to the standalone Redis reference is substantial.
They do not identify a particular hot function or prove a universal CPU ceiling.

Exporter sync-stage means in the volatile pure-write intervals were roughly
0.11–0.22 microseconds, versus approximately 8 milliseconds in the disk
concurrency-64 intervals. Original per-voter Raft and engine counts remain
separate in `/tmp/kv9-redis-ready-tmpfs-summary.json`. These before/after
snapshots include initialization, warmup, measurement, drain and final reads;
they are not isolated measured-phase averages. Fast tmpfs sync calls are not
evidence of stable-storage persistence.

All processes ran on one shared host without independent failure domains.
Logical affinity is not exclusive physical-core isolation. Root compilation,
runtime fixtures and archive compression were deferred throughout measured
trials, but this does not establish whole-host isolation. The two short
repetitions retain visible spread; no peak-only or sustained-capacity claim is
made. No hosted CI was invoked.

## Storage and artifact provenance

The wrapper wrote `diagnostic_only: true`, `volatile: true`,
`power_loss_durability: false` and the explicit tmpfs description before database
startup. For each running voter, it retained `/proc` command line, process and
executable identity, device ID, `findmnt` output and process `mountinfo`.
Actual data paths were `/dev/shm/kv9-ready-tmpfs-e1bo6b70/n1`, `n2` and `n3`,
all observed on tmpfs. The driver data path was a symlink into that uniquely
owned directory during execution.

After stopping owned children, the wrapper hashed all original data, copied it
to the output directory and compared both the copy and still-existing original
against those hashes. It replaced the driver data symlink with this retained
evidence copy before deleting only its owned tmpfs scratch. The retained copy
contains 51 files and 695,974,354 bytes; its disk location does not retrospectively
provide runtime durability. The independent audit rehashed every retained data
file and checked the scratch had been removed.

The focused auditor also delegates to the unchanged complete-trial validator.
Four deliberately invalid evidence controls were rejected at their intended
assertions: a false power-loss claim, a disk mount, a foreign voter data path
and a corrupted retained-copy hash. This checks observer discrimination; it is
not a runtime fault-injection or power-loss test. The existing durable driver,
validator and paired renderer were unchanged.

- Runtime release SHA-256: `e8232f8c612d92dad68b9209aa55d2d043d5a6fe3b22baac93c6735ee4c25b5d`.
- Source tree SHA-256: `a49402a2fd5e12b97e778b5d4d5186ee58a88a54fef8d24abc42c6072079c31f`.
- Redis client SHA-256: `57947d5a94c739dd58ca98e2c9a85c40814040826f13f99e85cd0e2c7fb2f636`.
- Redis server SHA-256: `1b2950213684d355e0e49fe6702c7bfb9d40b78f0da9ce289fc0e8f2a295e540`.
- Raw output: `/tmp/kv9-redis-ready-tmpfs-attempt1`; log: `/tmp/kv9-redis-ready-tmpfs-attempt1.log`.
- Matrix SHA-256: `3d2721993fc6c87bbef13c4d4c38e6d8961a4941e5fd7763f36deac2e82476f2`.
- Protocol SHA-256: `8f6a4e163da2c5d5adbce58d538eb1c87a3ec5d152530b59b786701128e732ef`.
- Independent audit and controls: `/tmp/kv9-redis-ready-tmpfs-check1.json`.
- Complete certainty/occupancy: `/tmp/kv9-redis-ready-tmpfs-outcomes-final.json`.
- Diagnostic summary: `/tmp/kv9-redis-ready-tmpfs-summary.json`; derivation: `/tmp/kv9-ready-tmpfs-summarize.py`.
- Final identity/cleanup audit: `/tmp/kv9-redis-ready-tmpfs-final-audit.json`.
- Raw file manifest: `/tmp/kv9-redis-ready-tmpfs-file-manifest.json` (458 files, 736,530,672 bytes).

The original wrapper, its execution-time hash and invocation are retained in
the output. The first runtime attempt and first independent audit both passed;
no failed or partial measurement was omitted.
