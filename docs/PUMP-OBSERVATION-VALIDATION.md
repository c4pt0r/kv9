# First Raft pump observations

The clean release build `9599e118bb490fe7320c8dee6fd7a9f7bd8fa5cd` completed
54 performance trials and 18 independently checked complete-history guards on
2026-09-09. Three new fixed histograms separate pump service, actual idle sleep
and loop-entry spacing. Production scheduling, durable acknowledgement and
quorum reads are unchanged. This document publishes that measured build; it does
not relabel the executable as this later documentation commit.

All 257 source hashes and retained workload/database/calibration binaries match
the build inventory. The measured database population was **6,777 operations**,
all successful. The separate in-memory calibration completed 2,200,536 measured
operations, also without unsuccessful outcomes. Eleven isolated corrupt-evidence
controls were rejected and the original matrix revalidated after each control.
Performance trials do not claim complete histories; their surrounding guards do.

The exact [measurement protocol](BENCHMARKS.md) is the same as the
[first release baseline](BENCHMARK-VALIDATION.md): three repetitions, 5-second
closed-loop measurement admission plus drain, 32 acknowledged warmup operations,
8 mutable keys plus a sentinel, 128-byte values, one or four clients, WAL/MinIO,
and separate loopback calibration. Clients use CPUs 0–1; voters and MinIO share
CPUs 2–5 on the same development host. The host is not exclusive. The benchmark
keeps its 5-ms refusal backoff, six-attempt cap, 1,500-ms logical deadline,
100-ms MinIO flush interval, pinned object-store image and default durability.
No deliberately injected faults occur in these performance trials.

## Results and populations

Each throughput range contains the three complete trial rates, including drain.
For each pump metric, the table shows the minimum and maximum of **nine means**:
one mean per voter per repetition. Each mean is the exact count/sum difference
between that voter's fresh before/after snapshots. These snapshots enclose the
whole workload lifecycle, including initialization, warmup, measurement, drain,
verification and background activity. They are not measurement-only client
populations, synchronized global snapshots or individual request critical paths.

| Backend | Mix | Clients | Successful ops/s | Service mean, ms | Idle mean, ms | Iteration mean, ms |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| wal | read | 1 | 24.95–24.96 | 0.344–0.383 | 20.050–20.053 | 20.396–20.435 |
| wal | read | 4 | 97.63–98.41 | 0.342–0.368 | 20.050–20.051 | 20.393–20.420 |
| wal | write | 1 | 16.12–16.62 | 5.671–6.769 | 20.051–20.052 | 25.722–26.820 |
| wal | write | 4 | 29.89–31.13 | 9.530–14.038 | 20.051–20.052 | 29.584–34.089 |
| wal | mixed | 1 | 20.04–20.25 | 3.034–3.271 | 20.051–20.052 | 23.086–23.323 |
| wal | mixed | 4 | 46.57–48.75 | 6.162–8.166 | 20.050–20.052 | 26.215–28.217 |
| minio | read | 1 | 24.95–24.95 | 0.705–1.049 | 20.051–20.052 | 20.757–21.147 |
| minio | read | 4 | 97.44–99.00 | 0.558–0.981 | 20.050–20.051 | 20.610–21.033 |
| minio | write | 1 | 10.65–10.84 | 10.315–12.402 | 20.051–20.052 | 30.367–32.453 |
| minio | write | 4 | 22.24–23.54 | 12.667–19.371 | 20.050–20.052 | 32.720–39.423 |
| minio | mixed | 1 | 14.88–16.26 | 5.631–6.896 | 20.051–20.052 | 25.683–26.948 |
| minio | mixed | 4 | 34.44–36.41 | 9.830–12.222 | 20.050–20.052 | 29.881–32.274 |

The [324-row population CSV](benchmarks/2026-09-09-9599e11-pump.csv) retains each
voter/trial/metric count, sum, mean and p50/p95 bucket interval. Every row was
independently recomputed from 216 raw voter snapshots, checking process/exporter
identity, schema version 2, bucket/count conservation, weighted duration bounds
and percentile ranks. Cumulative minima/maxima are not subtracted or presented
as interval extrema. Empty and other outcomes remain in the complete raw reports;
all three pump metrics had positive successful sample counts and zero failed
samples in every measured database trial.

One-client Get logical means were 40.065–40.076 ms on WAL and 40.069–40.079 ms
on MinIO. Their p50/p95/p99 intervals remain histogram bounds, not exact
percentiles. The nearly fixed 20.05-ms idle component is visible at every load.
Service time grows materially under writes: at four clients, per-voter means
range from 9.530 to 14.038 ms on WAL and 12.667 to 19.371 ms on MinIO. Loop
spacing grows with it because the current loop sleeps after doing its work.

These results support investigating both scheduling wait and persistence/apply
cost. They do not isolate a causal share of client latency, prove two sleeps per
read, or demonstrate an optimization. In particular, loop entry occurs before
the actual tick call; it is not a timestamp of a tick delivered under the peer
lock. Separately captured populations must not be added into a purported client
latency decomposition. The earlier baseline used a different executable and a
shared host, so small throughput differences do not establish an improvement.

The remaining #41 work is local-submission/inbound/outbound timing, bounded
per-turn and queued work, a single-owner wakeup contract and independently
specified tick deadlines. The scheduling algorithm needs its TLA+/TLAPS proof,
Rust synchronization map, deterministic race controls, actual Chaos Mesh and a
controlled paired release comparison before performance claims. No wakeup,
election-clock or batching change is included here.

## Validation and retained evidence

The implementation passed 501 workspace/doc tests, denied-warning Clippy,
formatting and workflow lint. Three tests exercise a deliberately blocked real
transport drain, a background pump's actual sleeps and first-iteration boundary,
and terminal failure without an extra idle sample. Six isolated latency source
controls each compiled, selected one test, failed their intended behavioral
assertion and passed after source restoration. The real MinIO workload E2E passed
all five cases; all 12 corrupted workload-report controls were rejected.

The separate 42-trial release observer experiment passed with the new 26-metric
inventory. One-thread recording had a median 41.77 ns per operation versus
34.59 ns with recording disabled. In-memory snapshot/JSON medians were 81.67 us
for sparse and 89.59 us for dense histograms. These are observer-only experiments;
they exclude filesystem publication, RPCs and database throughput. They were run
after the database measurements, not concurrently with them.

| Evidence | Development-host path |
| --- | --- |
| Release binaries and exact source inventory | `/tmp/kv9-pump-observation-build-1` |
| Raw matrix, histories, snapshots and retained checkpoint descriptors | `/tmp/kv9-pump-observation-measurement-1` |
| Eleven corrupted benchmark copies and revalidations | `/tmp/kv9-pump-observation-benchmark-controls-1` |
| Independent raw pump-population audit | `/tmp/kv9-pump-observation-publication-audit.json` |
| Six isolated implementation controls | `/tmp/kv9-pump-observation-controls-1` |
| Real MinIO workload E2E and twelve corrupt-report controls | `/tmp/kv9-pump-observation-minio-e2e-1`, `/tmp/kv9-pump-observation-workload-controls-1` |
| Release observer experiment | `/tmp/kv9-pump-observation-overhead-1` |

These local paths are not downloadable GitHub artifacts. Hosted CI separately
retains exact-revision smoke and fault/proof evidence. The full proof and actual
Chaos Mesh gates remain required for the pushed observation increment; their
acceptance links are tracked on #41 and #9. #40 was separately accepted and closed
at `ef6e90e` after all its exact-revision gates passed. #13 and the broader
industrial roadmap remain open; these small single-host measurements do not
establish storage-capacity, cross-machine failure-domain or industrial-throughput
acceptance.

| Artifact | SHA-256 |
| --- | --- |
| Build manifest | `e5d51a103d9397fae2d8c080cb17fd04287aca81ce2fda184dff5fd8843d6abc` |
| Protocol | `cdded1950f2b43bbdd8e91e2cceb6856d47b5e811d0f995afcaf559a231ff949` |
| Host inventory | `c6e459cb9db018fcaaf519d9288e270fa6796dd00d541c6c1078f949d76bba39` |
| Full matrix report | `84824d33e0b61617311274f19d292a13946b7681746186c33e7b46106fa96503` |
| Published pump CSV | `ca077f2258202b5d4a95c5499d65537c292037ffa0102b55d2999b2a66e87dfc` |
