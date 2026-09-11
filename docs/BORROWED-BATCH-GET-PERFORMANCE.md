# Borrowed batch context and actual Redis GET performance

The subsequent [parallel stream comparison](PARALLEL-STREAM-GET-PERFORMANCE.md)
uses this candidate as its control and improves both point and batch-size-one
reads. The numbers below remain the original borrowed-context comparison.

The borrowed-context candidate raises BatchGet(1) from **174,684–176,433** to
**199,195–200,266 successful calls/s** in this matched diagnostic. Gains paired
within each repeat are **12.9% and 14.6%**. Mean whole-call latency falls from
362.5–366.1 to 319.3–321.1 microseconds. The candidate batch path now trails
its point GET path by 3.4–4.4%.

Point GET remains near **207,259–208,330 calls/s**, with 307.0–308.6 microsecond
mean latency. Paired throughput changes are -0.14% and -0.84%; these two short
runs establish no point-read improvement. Actual Redis **GET** achieves
**503,585–507,146 calls/s**, with 126.1–127.0 microsecond mean latency: 2.430–2.434
times the candidate point-GET throughput. Redis parity remains open.

Unlike the [previous comparison](POINT-BATCH1-PERFORMANCE.md), this run includes
both actual Redis GET and MGET(1), measured with the same new Redis executable.
Redis MGET(1) reaches 470,601–474,841 calls/s. An MGET containing one key is not
substituted for GET in the point-read pairing.

## Source and measurement contract

| Role | Exact revision | Executable SHA-256 |
|---|---|---|
| Previous async batch server | `af4c4e31bdef2b1294931c27e9802bc04e6aeaf5` | `8217d3520ee719c548754296518fabd8e20d82aa5fb78a8ada2a1e526c98a758` |
| Borrowed-context server | `850f0deeed8d03426d28d65bbcdd94a503f16cf7` | `916738e118bf3d9f2c373f4fe81d7523db0db0f9a401acda9bfbc0450c9dd684` |
| Shared native client | `03c1c776a5dd7d1cc67491ab253e02ce51665bf8` | `22ca0883ca2900fab0b457d87ebc6bc50662d7145af0846304f02b5841a6d30b` |
| Shared Redis GET/MGET client | `b8ec38f660786412705f350d96ab086f0e7f6c60` | `61349111171ded5cb7fff9085c7886381788e5f4c7a407e91a5dda618d124892` |

All four source trees were clean and frozen. All executables are default-feature
release builds using the same compiler. The native executable, shared dataset
generator, scheduler and accounting are unchanged from the previous comparison.
The [Redis client contract](https://github.com/c4pt0r/kv9/blob/b8ec38f660786412705f350d96ab086f0e7f6c60/scripts/redis-reference/POINT-GET.md)
requires actual RESP GET bytes and a single-bulk reply, rejects MGET-array
substitution, and labels the measured API explicitly. Its 13 Rust tests, 25
native/Redis Python tests and warnings-denied Clippy passed before release.
All 19 retained legacy report outcomes were reproduced without modifying them.

Every cohort uses 64 closed-loop workers, 4,096 hot keys plus a sentinel,
128-byte values, batch size one, 100% reads, seed 71, 128 warmup calls and a
1,500 ms measurement window. Each starts a fresh backend. The second repeat
reverses the six-arm order. Point GET pairs with Redis GET; BatchGet(1) pairs
with MGET(1). Setup and complete nonce-zero dataset readback are outside timing.
Latencies include preparation, the client call and response validation.

Client CPUs are 0–1; all measured servers share CPUs 2–5. Three owned background
containers use 6–15,22–31 during timing and return to their original configured
and effective CPU sets afterward. Builds, tests, proof checks, Chaos injection,
profiling and independent audits were terminal before timing. Unrelated host
services and BuildKit are unconstrained, so this remains a short shared-host
diagnostic rather than an exclusive-host sustained benchmark.

KV9 has three voters and retains normal Raft quorum-confirmed reads and WAL
sync calls, with data on volatile tmpfs. Redis has one memory instance, no
replicas, and save/AOF disabled. Fault tolerance and power-loss durability are
not equivalent. This pure-read, batch-size-one run does not update larger-batch
or write-throughput claims.

## All twelve cohorts

Calls/s equals items/s only for batch size one. Quantiles are the original
histogram bucket intervals in microseconds, not exact samples or averaged
percentiles. All **5,310,765 measured calls succeeded**, with no unknown,
refused, failed, client-rejected or dropped operations and no extra transport
attempts. No cohort hit the safety call ceiling.

| Cohort | Calls | Calls/s | Mean us | p50 us | p95 us | p99 us |
|---|---:|---:|---:|---|---|---|
| 000-old-point-p00064 | 312,967 | 208,626.8 | 306.597 | 315.392–319.487 | 389.120–393.215 | 442.368–446.463 |
| 001-old-batch1-p00064 | 264,679 | 176,433.3 | 362.522 | 380.928–385.023 | 454.656–458.751 | 516.096–520.191 |
| 002-new-point-p00064 | 312,536 | 208,330.3 | 307.044 | 319.488–323.583 | 389.120–393.215 | 438.272–442.367 |
| 003-new-batch1-p00064 | 298,838 | 199,195.5 | 321.072 | 335.872–339.967 | 405.504–409.599 | 471.040–475.135 |
| 004-redis-mget1-p00064 | 712,393 | 474,840.7 | 134.635 | 120.832–121.855 | 178.176–180.223 | 229.376–231.423 |
| 005-redis-get1-p00064 | 760,843 | 507,145.6 | 126.071 | 118.784–119.807 | 163.840–165.887 | 231.424–233.471 |
| 006-redis-get1-p10064 | 755,483 | 503,584.6 | 126.963 | 118.784–119.807 | 163.840–165.887 | 231.424–233.471 |
| 007-redis-mget1-p10064 | 706,018 | 470,600.9 | 135.846 | 120.832–121.855 | 178.176–180.223 | 229.376–231.423 |
| 008-new-batch1-p10064 | 300,424 | 200,266.4 | 319.348 | 331.776–335.871 | 401.408–405.503 | 454.656–458.751 |
| 009-new-point-p10064 | 310,935 | 207,258.7 | 308.613 | 319.488–323.583 | 393.216–397.311 | 450.560–454.655 |
| 010-old-batch1-p10064 | 262,086 | 174,683.9 | 366.148 | 380.928–385.023 | 458.752–462.847 | 532.480–540.671 |
| 011-old-point-p10064 | 313,563 | 209,023.7 | 306.022 | 315.392–319.487 | 389.120–393.215 | 446.464–450.559 |

## Acceptance and retained evidence

The candidate passed its [local tests, six scoped TLAPS theorems, process E2E
and actual eleven-window Chaos Mesh acceptance](BORROWED-BATCH-CONTEXT-ACCEPTANCE.md)
before timing. A six-arm correctness smoke also passed on its first run,
with 2,008,613 completed calls and all 20 owned process lifetimes exited. Smoke
throughput and instrumented profile rates are excluded from this report.

The timing wrapper and child exited 0, with exact container CPU restoration
and no cleanup errors. Independent read-only audit passed on its first attempt:
all 12 cohorts, 40 exited process lifetimes, 2,317 source-file checks, 350
in-window resource observations, 24 qualifying fresh drains and 24
voter/listener/mount bindings. All 264 retained files / 44,378,036 bytes matched.
Historical namespace UID maps remained unchanged. There was no measurement or
audit rerun. This aggregate read-only run is separate from linearizability
history acceptance.

- Raw matrix: `/tmp/kv9-borrowed-batch-matched-diagnostic-attempt1/cohorts`.
- Outer completion: `/tmp/kv9-borrowed-batch-matched-diagnostic-attempt1/summary.json`;
  root session 47824 exited 0.
- Independent audit: `/tmp/kv9-borrowed-batch-matched-independent-first/results-first`.
- Frozen runner/preflight: `/tmp/kv9-borrowed-batch-comparison-preparation`.

| Artifact | SHA-256 |
|---|---|
| Driver | `86e3f6b6d6353cb90969597d63153478db88579e4582fe3bd178df2cba618b79` |
| Wrapper | `e922939e675d722fe36c3b0890d31d6329be810dd7567b2e369643d10e2137fb` |
| Raw matrix | `2e90284f08886bc9837c5e37fa6045f0270c5341086e65ee3b899f8173ba8b1e` |
| Outer completion/restoration | `c07a17e59dddeefcdf3315a39f50a3915d5fdc1ebff1583a93026461e54975a8` |
| Independent audit | `98a72bc5b12a5203151647f87ad5f1de50beb01f528c9991b6789f30cf42eb99` |
| Independent input inventory | `6efebc49eebb7621ae25f1fcbfab6d791795afead668b01df84d34514b8f1419` |

The measured improvement is confined to the batch context path. The next
optimization should address the common point-read/RPC path: allocation and
copying, scheduling transitions, and synchronization. CPU samples identify
candidates but do not establish a complete latency decomposition or a hardware
ceiling. Preserve the Raft barrier and authorization contract while testing each
hypothesis. No hosted CI ran, no master promotion occurred, and the remaining
proof, fault, scale-out and performance roadmap gates stay open.
