# Jemalloc server performance and memory tradeoff

The Linux allocator candidate `629bee4fd9dcca02529703a06eefe9250fd5d1ac`
reaches **304,863–305,903 successful single GET calls/s**, compared with
287,948–291,813 for the same-run parallel-stream control `f2c4e85`.
Paired gains are **4.47% and 6.24%**. Whole-call mean latency falls from
219.2–222.1 to **209.1–209.8 us**; both p99 buckets improve.

BatchGet(1) reaches **295,771–297,225 calls/s**, with paired gains of
5.25% and 2.88%. These rates equal items/s only for batch size one.
Actual Redis GET reaches **507,769–508,136 calls/s**, leaving a
**1.666x / 1.661x** throughput gap. Redis parity remains open; this comparison
updates neither larger-batch nor write-throughput claims.

The cost is higher memory use: the sum of measured per-voter mean RSS rises
from **43.1–43.9 MiB to 54.0–54.3 MiB**, approximately 10.2–11.2 MiB or
23.1–25.9% more across three voters. The modest throughput/latency gain is
therefore a tradeoff, not a free improvement. Keep the isolated candidate for
further validation; do not switch master/default allocation based only on this
short, small-working-set diagnostic. Larger working sets, memory pressure and
mixed/write traffic need separate evidence. Continue eliminating unnecessary
allocations and copies rather than treating allocator replacement as completion
of the performance work.

The [new parallel-stream CPU profile](PARALLEL-STREAM-CPU-PROFILE.md) motivated
this experiment. The [implementation contract](https://github.com/c4pt0r/kv9/blob/629bee4fd9dcca02529703a06eefe9250fd5d1ac/docs/JEMALLOC-SERVER.md)
changes only Rust's global allocator in the Linux kv9 binary, using exact
`tikv-jemallocator` 0.6.1 with empty emitted allocator/sys feature arrays.
Core Raft, metadata, storage, request, admission and retry transitions are
unchanged. The native allocator is statically linked with its prefixed API,
O3 and background threads disabled by default. Optional support still exists;
no runtime/configuration tuning is applied in this experiment.

## Matched protocol and all twelve cohorts

The native `03c1c77` and Redis `b8ec38f` clients are unchanged. Each fresh cohort
uses 64 closed-loop workers, 4,096 keys plus a sentinel, 128-byte values, batch
size one, 100% reads, seed 71, 128 warmup calls, a 1,500 ms interval and a
10-million-call safety cap. The second repeat reverses the six-arm order.
Actual point GET pairs with Redis GET; BatchGet(1) pairs with Redis MGET(1).
All **6,558,446 measured calls succeeded**, with no refused, unknown,
failed, client-rejected or dropped calls and no extra transport attempts. No
cohort hit the safety cap.

Latency includes preparation, transport and response validation. Quantiles are
retained histogram bucket intervals, not exact samples or averaged percentiles.

| Cohort | Calls | Calls/s | Mean us | p50 us | p95 us | p99 us |
|---|---:|---:|---:|---|---|---|
| 000-old-point-p00064 | 437,781 | 291,812.9 | 219.160 | 208.896–210.943 | 335.872–339.967 | 389.120–393.215 |
| 001-old-batch1-p00064 | 423,637 | 282,396.7 | 226.444 | 215.040–217.087 | 348.160–352.255 | 401.408–405.503 |
| 002-new-point-p00064 | 457,351 | 304,862.8 | 209.784 | 196.608–198.655 | 323.584–327.679 | 364.544–368.639 |
| 003-new-batch1-p00064 | 445,875 | 297,224.5 | 215.144 | 200.704–202.751 | 327.680–331.775 | 372.736–376.831 |
| 004-redis-mget1-p00064 | 761,752 | 507,770.0 | 125.897 | 117.760–118.783 | 169.984–172.031 | 229.376–231.423 |
| 005-redis-get1-p00064 | 761,742 | 507,768.7 | 125.912 | 118.784–119.807 | 165.888–167.935 | 233.472–235.519 |
| 006-redis-get1-p10064 | 762,306 | 508,136.4 | 125.818 | 117.760–118.783 | 163.840–165.887 | 229.376–231.423 |
| 007-redis-mget1-p10064 | 742,079 | 494,666.9 | 129.230 | 118.784–119.807 | 176.128–178.175 | 231.424–233.471 |
| 008-new-batch1-p10064 | 443,725 | 295,771.3 | 216.196 | 202.752–204.799 | 327.680–331.775 | 368.640–372.735 |
| 009-new-point-p10064 | 458,916 | 305,903.0 | 209.072 | 194.560–196.607 | 319.488–323.583 | 360.448–364.543 |
| 010-old-batch1-p10064 | 431,308 | 287,496.1 | 222.417 | 212.992–215.039 | 335.872–339.967 | 393.216–397.311 |
| 011-old-point-p10064 | 431,974 | 287,947.8 | 222.104 | 210.944–212.991 | 344.064–348.159 | 393.216–397.311 |

Client CPUs are 0–1; all measured servers share CPUs 2–5. Three owned background
containers use CPUs 6–15,22–31 during timing and are restored exactly afterward.
Builds, tests, fault work, profiling and audits were terminal before timing.
Other host services and BuildKit are unconstrained. KV9 has three Raft voters,
ordinary streaming gRPC, normal quorum/read barriers and WAL sync calls on
volatile tmpfs. Redis is standalone with save/AOF disabled and no replicas.
Fault tolerance and power-loss durability are not equivalent. These are short
shared-host diagnostics, not sustained capacity or offered-load curves.

## Memory observations

RSS comes from the same existing in-window resource samples used by the
original auditor. HWM includes each process's setup lifetime; the sum of
individual HWM values is not a simultaneous cluster peak. Voter-number pairs
are descriptive and do not assume matching leader roles.

| Repeat | API | Sum of mean RSS MiB, old → new | Sum of sampled HWM MiB, old → new |
|---|---|---:|---:|
| 1 | point | 43.30 → 54.08 | 43.84 → 55.26 |
| 1 | batch1 | 43.07 → 54.25 | 43.56 → 55.29 |
| 2 | point | 43.15 → 54.32 | 43.67 → 55.20 |
| 2 | batch1 | 43.90 → 54.05 | 44.40 → 55.03 |

The full per-voter table and exact values remain in the independent report and
statistics.json. No extra live memory polling was added. This working set does
not characterize fragmentation, memory pressure or long-running reclamation.

## Source, build and acceptance scope

| Role | Revision | Executable SHA-256 |
|---|---|---|
| Parallel stream control | `f2c4e856d03f75ac6e4718f0aac7f4e7a1cf3d02` | `3ed7974e3eebd9dc6a0e91988d3913f2999fe44fa38d989737022481f993d305` |
| Linux jemalloc candidate | `629bee4fd9dcca02529703a06eefe9250fd5d1ac` | `d17f0ec296218c39e743312900a471a7827089e607bf3badbcb96037ca984acf` |
| Shared native client | `03c1c776a5dd7d1cc67491ab253e02ce51665bf8` | `22ca0883ca2900fab0b457d87ebc6bc50662d7145af0846304f02b5841a6d30b` |
| Shared Redis client | `b8ec38f660786412705f350d96ab086f0e7f6c60` | `61349111171ded5cb7fff9085c7886381788e5f4c7a407e91a5dda618d124892` |

Clean source and default-feature release bindings passed. The lockfile adds only
the allocator and its native sys package. Root separately retained the exact
Cargo build-script OUT_DIR, native configuration/archive and allocator/sys
feature arrays; ELF relocation readback connects the Rust allocation thunk to
_rjem_malloc/_rjem_mallocx. Build/runtime override variables were absent.

The matched driver adds only before-client and post-client allocator provenance
outside timing to the earlier fixture. All 48 per-voter endpoint records bind
PID/start/boot identity, absent allocator environment overrides and no prefixed
configuration symlink. Ordinary files at that path are ignored by jemalloc.
These observations attest their endpoints, not continuous configuration state.
All original workload, validator, deadline, drain and cleanup rules
remain unchanged. The wrapper and explicit native-client arguments are identical.

Focused binary checks and the actual stream/unary leader-kill/original-directory
restart E2E passed before timing. The E2E includes **373 calls: 344 OK and
29 unknown**, with both complete atomic histories valid and no cleanup errors.
Those are actual allocator-bearing server processes; independent library unit
test executables do not inherit the root binary allocator. The six-arm
correctness smoke also passed before timing and is not a throughput result.

Timing session **36793** exited 0. The independent auditor passed its first
execution: 12 cohorts, 40 exited lifetimes, 2,322 source-file checks, 353 measured
resource samples, 24 fresh drains, 24 voter bindings and all 264 retained files /
44,377,888 bytes. Container CPU configuration and historical namespace identities
were preserved. A derived statistics unit-label correction is retained; the
runtime and acceptance audit had no rerun or predicate change. Read-only timing
accounting is separate from the process and Chaos atomic-history checks.

Complete post-screening correctness and actual Chaos acceptance are tracked in
[JEMALLOC-SERVER-ACCEPTANCE.md](JEMALLOC-SERVER-ACCEPTANCE.md). No default/master
promotion or whole-implementation proof claim follows from this comparison.
No hosted CI was dispatched.

Raw timing: `/tmp/kv9-jemalloc-matched-diagnostic-attempt1/cohorts`.
Independent evidence: `/tmp/kv9-jemalloc-matched-independent-first/results-first`.
Native build/linkage: `/tmp/kv9-jemalloc-server-local-first/native-build`.

| Artifact | SHA-256 |
|---|---|
| Matched driver | `251dd4b26fd0df1fb64c162a00d612d459b821cbe83240b5995f6d8393bf1b74` |
| Wrapper | `e922939e675d722fe36c3b0890d31d6329be810dd7567b2e369643d10e2137fb` |
| Raw matrix | `a36190d3cdb7150f10dde4bd501a03466f19f1541cab6ad81e256bf7d1dc6fa5` |
| Outer completion | `c07a17e59dddeefcdf3315a39f50a3915d5fdc1ebff1583a93026461e54975a8` |
| Independent audit | `ba59b41501f35c8dad4019930f0337dee668d6744a9838e32ab8487fb22a88ba` |
| Independent inventory | `2d0b0399fe0e2b8079efa1c1ef200e6786e0bd6cba18da31c15dd09b9266896b` |
| Native build/linkage summary | `d2e2cb83a1513363ec6121076940740be13bdbf0bba2ce59a659a606b696f972` |
| Process E2E summary | `c9456e574124ba1d558ea8158eb25f92b16b2c21c82b63d3f1058b3610c7df91` |
