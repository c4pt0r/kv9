# Matched point GET and BatchGet(1) performance

The subsequent [borrowed-context comparison](BORROWED-BATCH-GET-PERFORMANCE.md)
records the next candidate and adds an actual Redis GET control. The results
below retain the original async-batch revision and MGET(1) reference.

The async native batch-read change raises BatchGet(1) throughput from
109,097–112,477 to 176,734–177,385 successful calls/s in this matched diagnostic.
The paired gains are 62.0% and 57.7%; whole-call mean latency falls from
568.7–586.4 to 360.6–361.9 microseconds. The observed p99 histogram intervals
move from 1.180–1.229 ms to 0.508–0.524 ms.

The matched control confirms point GET remains near 208,000–211,000 calls/s
with both server revisions. The native batch path had missed the asynchronous
read preparation already used by point GET. This does not retroactively make
the historical point client and workload identical to this measurement. The new
batch path still trails matched point GET by 15.8–15.9%. Redis MGET(1) achieves
477,838–508,964 calls/s: 2.70–2.87 times the new batch throughput in the paired
repeats. Redis parity remains open.

## Compared sources and common workload

| Role | Exact revision | Standalone binary SHA-256 |
|---|---|---|
| Old server | `3bd1751bddf9a85b07447b64a12adf28ed0df8a9` | `82cf715e6d8d1ea8898db1ad3624431958a21213c50ebc00ae5320943cc03991` |
| Async batch server | `af4c4e31bdef2b1294931c27e9802bc04e6aeaf5` | `8217d3520ee719c548754296518fabd8e20d82aa5fb78a8ada2a1e526c98a758` |
| Shared KV9 client | `03c1c776a5dd7d1cc67491ab253e02ce51665bf8` | `22ca0883ca2900fab0b457d87ebc6bc50662d7145af0846304f02b5841a6d30b` |
| Redis client | `3bd1751bddf9a85b07447b64a12adf28ed0df8a9` | `d5c2069f00787a37a14b1cb25848aad05c3d2c684d9609b914becbafc0ec9e2b` |

The four KV9 arms use the same default-feature release client, ordinary tonic
streaming endpoint, key selection, scheduler and whole-call accounting. The
v2 config explicitly chooses point GET or native BatchGet; this comparison does
not combine historical clients or change the server between point and batch
arms of the same revision. The Redis reference uses MGET with one key.

Each arm has 64 closed-loop workers, 4,096 hot keys plus one sentinel, 128-byte
values, batch size one, 100% reads, seed 71, 128 warmup calls and a 1,500 ms
measurement window. The run ID and payload match within each repeat. Every
cohort starts a fresh backend. The second repeat reverses the five-arm order.
No cohort reached the 10-million-call safety ceiling or dropped an offered slot.
Initialization and readback are excluded from measured throughput.

Client CPUs are 0–1; the entire measured server group shares CPUs 2–5. Owned
background containers use 6–15,22–31, excluding the measured cores' SMT
siblings. Other host services and BuildKit are unconstrained: this is a short
shared-host diagnostic, not an exclusive-host sustained benchmark. No builds,
tests, proof checks, fault injection, or independent audits overlapped timing.

KV9 uses three independent voters, normal quorum-confirmed reads and normal
WAL sync calls, with data on volatile tmpfs. Redis is standalone memory with
save/AOF disabled and no replicas. This is not equivalent fault tolerance or
power-loss durability. The read-only measurement does not establish write or
larger-batch performance for the new server.

## All ten measured cohorts

Calls/s equals items/s only because batch size is one. Latencies cover the
whole API call, including client preparation and response validation. Quantiles
are the retained histogram bucket bounds in microseconds; they are not exact
samples or averages of percentiles. Every measured call succeeded.

| Cohort | Calls | Calls/s | Mean us | p50 us | p95 us | p99 us |
|---|---:|---:|---:|---|---|---|
| 000-old-point-p00064 | 312,038 | 208,005.2 | 307.521 | 315.392–319.487 | 389.120–393.215 | 438.272–442.367 |
| 001-old-batch1-p00064 | 163,679 | 109,096.8 | 586.367 | 548.864–557.055 | 983.040–991.231 | 1212.416–1228.799 |
| 002-new-point-p00064 | 314,895 | 209,898.6 | 304.731 | 315.392–319.487 | 389.120–393.215 | 438.272–442.367 |
| 003-new-batch1-p00064 | 265,160 | 176,733.6 | 361.907 | 376.832–380.927 | 454.656–458.751 | 520.192–524.287 |
| 004-redis-mget1-p00064 | 716,874 | 477,838.1 | 133.775 | 119.808–120.831 | 180.224–182.271 | 229.376–231.423 |
| 005-redis-mget1-p10064 | 763,591 | 508,964.0 | 125.602 | 118.784–119.807 | 153.600–155.647 | 231.424–233.471 |
| 006-new-batch1-p10064 | 266,154 | 177,385.3 | 360.559 | 376.832–380.927 | 450.560–454.655 | 507.904–511.999 |
| 007-new-point-p10064 | 316,461 | 210,944.4 | 303.233 | 311.296–315.391 | 385.024–389.119 | 425.984–430.079 |
| 008-old-batch1-p10064 | 168,745 | 112,476.6 | 568.741 | 532.480–540.671 | 942.080–950.271 | 1179.648–1196.031 |
| 009-old-point-p10064 | 312,822 | 208,514.1 | 306.751 | 315.392–319.487 | 389.120–393.215 | 438.272–442.367 |

Total: **3,600,419 measured calls, all successful**, with zero refused, unknown,
read-failed, client-rejected or dropped operations. The eight KV9 cohorts made
one SDK transport attempt per measured call. All 4,097 initialized keys per arm
were read back with exact nonce-zero values and unchanged sentinels.

## Correctness and evidence

The [local unit, Clippy, compiled-control and process gates](ASYNC-BATCH-READ-ACCEPTANCE.md)
and [actual 11-window Chaos Mesh histories](ASYNC-BATCH-READ-CHAOS.md) passed at
the exact async server revision before timing. The change retains the existing
Raft barrier, epoch authorization and same-snapshot fallback. This performance
run is aggregate read-only evidence, not an independent linearizability history.

Raw matrix: `/tmp/kv9-point-batch1-matched-diagnostic-attempt1/cohorts`.
The outer wrapper and child both exited 0; the wrapper records
`restoration_complete=true`, no child cleanup errors, and exact restoration of
all three owned containers' configured/effective CPU sets and identities.
The original failed smoke attempt remains retained with its Redis argv[0]
startup error; the corrected five-arm smoke passed independently before timing.
Neither smoke contributes timing results.

Driver SHA-256:
`3a0da24f5d325f1be0276c5494dfb3763671a050fb6f917e5f884b961a361c77`.
Outer wrapper SHA-256:
`508d36780da7b96bae4fab949c2d9c6d60da46d9c9c70e56f789c3ea522dbe0d`.

The independent read-only audit accepted all ten original cohorts: 36 owned
process lifetimes exited, 291 in-window resource samples, 24 qualifying fresh
drains, and 24 voter/listener/mount bindings. All 264 retained files totaling
44,376,895 bytes matched their original hashes. Container identities, original
namespace UID maps and exact configured/effective CPU restoration were checked.

The first independent audit adapter stopped after four native cohorts because
it expected the native-only `attempt_reasons` field in Redis reports. That
failed audit and its log remain retained. The corrected schema branch accepted
the same ten original cohorts; no workload was rerun or evidence rewritten.
Audit directory: `/tmp/kv9-point-batch1-matched-independent-first`.

| Artifact | SHA-256 |
|---|---|
| Raw matrix | `129f00469e4764fd5743e50cf7add70879189abcb317b29f3757bef23ad551bd` |
| Outer completion/restoration | `c07a17e59dddeefcdf3315a39f50a3915d5fdc1ebff1583a93026461e54975a8` |
| Independent audit | `0641807a3cb9d11cd56ed5938de2d9154b708af051f617520b25d8f5eb2cc091` |
| Independent input inventory | `0bc2ed7cc29bb5f084678f0095c7f8fdbbf7cb5dfb9936d46605fd60346c2a5e` |

All checks ran locally. No hosted CI was dispatched, no default/main promotion
was made, and the broader performance/consistency/availability roadmap gates
remain open.

## Next optimization target

Profile the remaining batch/point difference and the normal streaming request
path under the same measurement contract. Source inspection identifies a
repeated first-key region lookup on the same immutable view and additional
batch allocation/copy work as candidates; neither is yet a measured explanation
of the remaining gap. Preserve complete context/epoch checks, bounded inline
work and Raft quorum semantics. Repeat representative larger batch and write
cohorts after a measured improvement, before claiming broader performance gains.
