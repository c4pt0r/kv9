# Ready versus proposal queue: targeted c64 diagnostic

The exact proposal-queue candidate improved this local **disk PUT** diagnostic
from 838.0 to 1,018.2 acknowledged operations/s. GET and mixed disk changes were
small. In the separate **volatile tmpfs** diagnostic, its PUT and mixed rates
were lower by roughly 9% and 8%. This is a useful tradeoff to investigate, not
an established general throughput improvement, full acceptance result or
capacity claim.

All 48 cohorts completed their requested duration: 3,636,399 KV9 and 35,289,829
Redis operations succeeded, with zero measured refusals, unknown outcomes,
transport/read failures or client rejections. Independent checks passed after
repairing an observer import environment. Original failed checks remain retained.

## Exact artifacts and unchanged client

| Artifact | Revision / SHA-256 |
|---|---|
| Ready runtime | `892b2a178450309859113c942f1738c070130eb5` |
| Queue runtime | `95fb5cd968411009a41ba4f7ed287cd6597115fa` |
| Ready release executable | `e8232f8c612d92dad68b9209aa55d2d043d5a6fe3b22baac93c6735ee4c25b5d` |
| Queue release executable | `6e5ec0b8671b72768baa9ec2346dcc9fc6a0ed40afa402b230e50f8a35b30309` |
| Ready source inventory | `a49402a2fd5e12b97e778b5d4d5186ee58a88a54fef8d24abc42c6072079c31f` |
| Queue source inventory | `3d648786acf9a843769e0d1ce4259c2e2c6870a6d5d90ae043515e407b0c7515` |
| Fixed Ready persistent client | `b47d5f408cd9a36ce7a8117917eb0fba530b0616125b4fff38980a255ddd12dc` |
| Fixed Redis reference client | `57947d5a94c739dd58ca98e2c9a85c40814040826f13f99e85cd0e2c7fb2f636` |

Only the candidate server was freshly built, from a clean private worktree with
an isolated Cargo target and empty runtime features, on CPUs 6–31. The existing
Ready client executable, source inventory and build manifest were copied
byte-for-byte. The server and client are deliberately recorded with different
source revisions; neither artifact was relabeled. The same-revision full
benchmark build gate is not claimed here. Each actual workload report was
validated against its real, unchanged client manifest.

No measured operation exercised the new proposal-refusal metadata. These
success-only populations therefore do not establish how the old client handles
that new metadata. No client/parser change or uncertain-write retry was used
to make the comparison pass.

## Protocol and limits

Each runtime/storage combination has twelve cohorts: GET100, PUT100 and
GET50/PUT40/DELETE10, each with two three-second repetitions and separate
KV9/Redis cohorts. Concurrency is 64, with one outstanding operation per worker,
64 hot keys, 23-byte keys, 128-byte values and 32 warmup operations. KV9 retains
1,500-ms deadlines, at most six attempts for explicit NotLeader routing refusals,
and the original admission settings. It never retries an uncertain write.
Redis has no retry or pipelining. Neither operation cap was reached.

Runtime order was Ready disk, queue disk, Ready tmpfs, queue tmpfs. Within each
matrix, KV9 precedes Redis in repetition one and Redis precedes KV9 in repetition
two; mix order remains GET, PUT, mixed. Revisions were not randomly interleaved.
This is a targeted concurrency point, not a saturation sweep.

Both fixtures have three KV9 voters on one physical host over IPv4 loopback.
Clients use logical CPUs 0–1 and the three servers share CPUs 2–5. Root's
functional fixtures and archive work were finished before timing. Other host
activity, logical CPU topology, short repetitions and sequential revision order
remain limitations; no independent machine failure domains are represented.

Redis 7.0.15 is a standalone memory reference, with `save ""`, `appendonly no`,
zero replicas and one I/O thread, verified from live configuration/replication
observations. It does not match KV9 quorum durability. Missing GET values remain
successful nil/None responses in mixed traffic; the KV9 report does not export
hit/miss counts.

## Ordinary disk result

Rates are pooled acknowledged counts divided by total cohort time, including
drain. Parentheses preserve repetition one / two rather than concealing spread.

| Workload | Ready ops/s (repetitions) | Queue ops/s (repetitions) | Queue / Ready |
|---|---:|---:|---:|
| GET100 | 84,973.3 (85,623.7 / 84,323.0) | 85,503.5 (84,583.8 / 86,423.5) | 1.006 |
| PUT100 | 838.0 (815.5 / 860.5) | 1,018.2 (1,029.7 / 1,006.8) | 1.215 |
| Mixed | 766.0 (756.4 / 775.7) | 781.0 (778.3 / 783.7) | 1.020 |

PUT improved in both queue repetitions in this fixture. The Redis PUT reference
for the queue matrix nevertheless varied from 370,867.5 to 491,574.5 ops/s,
showing substantial host/order variation. It is retained rather than normalized
away or used to manufacture a stronger improvement claim.

Pooled acknowledged p99 histogram intervals were 1.05–2.10 ms for GET and
67.11–134.22 ms for PUT in both revisions. Mixed traffic occupied the
134.22–268.44-ms bucket for Ready and 67.11–134.22-ms bucket for queue. These
are coarse histogram intervals, not exact percentile measurements.

For whole PUT trial envelopes, successful Raft sync calls per acknowledged
write per voter averaged approximately 0.136 in Ready and 0.122 in queue;
engine sync calls averaged 0.085–0.097 and 0.063–0.065 respectively. These
counters include initialization, warmup, measurement, drain, verification and
metric-export waits, and use all acknowledged writes in that envelope. They
are not a timed-operation-only cost or a proof of crash durability.

## Separate volatile tmpfs result

All voter Raft/engine data directories were on owned `/dev/shm` tmpfs paths,
with actual process arguments, mount types, devices and executable identities
observed. Normal quorum and sync calls remained enabled. **Tmpfs provides no
disk durability or power-loss guarantee.** Its numbers are not pooled into the
disk result or treated as durable acceptance.

| Workload | Ready ops/s (repetitions) | Queue ops/s (repetitions) | Queue / Ready |
|---|---:|---:|---:|
| GET100 | 86,944.1 (86,879.9 / 87,008.4) | 84,169.9 (85,373.4 / 82,966.5) | 0.968 |
| PUT100 | 67,160.1 (67,276.5 / 67,043.6) | 61,318.3 (64,340.5 / 58,296.0) | 0.913 |
| Mixed | 69,081.4 (69,242.7 / 68,920.0) | 63,326.4 (66,328.6 / 60,324.3) | 0.917 |

The queue's second tmpfs PUT/mixed repetitions were noticeably slower than its
first. This result does not establish the cause. All pooled p99 intervals were
1.05–2.10 ms, which is too coarse to resolve small differences within that bin.

Measured aggregate server CPU was about 3.6–3.7 cores during GET and tmpfs
write/mixed traffic, versus roughly 0.12–0.15 cores during disk write/mixed
traffic. KV9 clients used at most about 1.06 of their two allowed cores, with
no measured thread above 0.54 cores. Redis clients peaked at about 1.45 cores,
with no thread above 0.73. These samples show client CPU headroom at this
concurrency; they do not establish unlimited client or server capacity.

For reference, the pooled standalone Redis cohorts were:

| Storage label / runtime matrix | GET/s | PUT/s | Mixed/s |
|---|---:|---:|---:|
| Disk / Ready | 503,696.4 | 487,893.1 | 496,147.5 |
| Disk / Queue | 499,687.4 | 431,220.7 | 497,990.2 |
| Tmpfs / Ready | 502,693.9 | 486,867.3 | 501,011.8 |
| Tmpfs / Queue | 499,726.2 | 480,312.2 | 493,906.0 |

The storage label identifies the paired KV9 fixture; Redis persistence remained
disabled in every cohort.

## Complete evidence and independent checks

Raw evidence is `/tmp/kv9-ready-queue95-c64-diagnostic`, with separate
`disk-ready-attempt1`, `disk-queue95-attempt1`, `tmpfs-ready-attempt1` and
`tmpfs-queue95-attempt1` directories. It retains complete original reports,
per-operation/outcome histograms, CPU/thread samples, source/build identities,
actual process hashes, before/after status and queue ledgers, Redis live
configuration, all subprocess commands, logs and tmpfs data copies.

All public backend ledgers were zero after client drain and verification.
The queue candidate also had zero retained proposal requests/bytes; its observed
cumulative peak was 62 requests on disk and 64 on tmpfs, below the 128-request
limit. Ready has no proposal-queue ledger. These observations are not an
assertion that client concurrency bounds future residual server jobs.

`plan.json` preserves the first two tmpfs observer failures: the copied observer
under `/tmp` could not import its sibling `benchmark` module. Both failed
before executing validation. The unchanged observer then passed with explicit
`PYTHONPATH=/tmp/kv9-redis-comparison-baseline/scripts`; no timed cohort was
rerun, source/parser changed, or assertion weakened. Its isolated c64 adaptation
changes the expected shape from 24 to twelve cohorts while preserving storage
identity/copy checks and all four invalid-evidence controls. The original
orchestrator exit was 1 because of those import failures; later successful
observer attempts are separately recorded. A derived-summary helper's initial
Redis CPU field-name error is also retained with its corrected helper.

`final-audit.json` independently checks all 48 cohorts, exact source and runtime
identities, the fixed client, all outcome populations, process/CPU identities,
admission settings and residual ledgers. `latency-cpu-stage-details.json`
retains the additional interval/CPU/sync calculations with their scope.
`evidence-manifest.json` inventories the complete retained evidence, including
copied exact executables, source inputs, scripts and this report;
`evidence-verification.json` records a second complete hash/size read-back.
All 84 observed process lifetimes exited. Tmpfs cleanup first verified copies
of all 48 data files per fixture: 672,166,490 bytes for Ready and 614,709,356
bytes for queue, comparing the original and retained copy hashes before removing
only the owned scratch directories. Original disk and copied tmpfs evidence
remain available. No Kubernetes resource, unrelated service, shared Cargo target
or shared validator was changed; no hosted CI was triggered.
