# Command receipt FIFO: volatile c64 diagnostic

The constant-time FIFO maintenance candidate `c7313ec` did not improve
end-to-end throughput in this bracket. Relative to all four surrounding
`a00e39f` baseline repetitions, pooled GET was 0.608% lower, PUT 0.691% lower
and mixed throughput 0.934% lower. Both candidate PUT repetitions were below
all four baseline repetitions; read and mixed repetition ranges overlap.
Client p99 buckets did not change. These short results do not establish a
general regression size, but they provide no performance-selection benefit.

Keep `a00e39f` as the performance development baseline and retain FIFO as an
isolated representation experiment. The earlier async-write +18.4% PUT result
belongs to its separate comparison and is not attributed to FIFO. The next
step is a fresh CPU profile of the current async-write runtime before selecting
another request-path change. No runtime promotion or roadmap item completion
is claimed here.

All database directories used explicitly volatile tmpfs. Normal Raft quorum,
sync calls, exact receipt/fence decisions, unknown outcomes and retry rules
remained enabled. Redis was standalone with persistence disabled and zero
replicas. This is a memory-path diagnostic with different guarantees, not
disk/power-loss durability, sustained capacity or cross-host acceptance.

## Fixed inputs and workload

| Input | Revision or SHA-256 |
| --- | --- |
| Async-write baseline | `a00e39f9f8da7bb381df19afd2eed63e8a1f1b75` |
| Receipt FIFO candidate | `c7313ecd9918c3f1a8269dcf4bcf31ba5ba69c73` |
| Baseline release executable | `b50da6f48e4ed88416892a3490653afb979a410a8c8c935abfc5d1e6008ccfbc` |
| Candidate release executable | `f2402a9321e19dd937c980496255df2402b090b1aa08445139223d555fb27150` |
| Unchanged Ready client | `b47d5f408cd9a36ce7a8117917eb0fba530b0616125b4fff38980a255ddd12dc` |
| Unchanged Redis client | `57947d5a94c739dd58ca98e2c9a85c40814040826f13f99e85cd0e2c7fb2f636` |

Clean release builds came from `/tmp/kv9-redis-async-write-wait-build` and
`/tmp/kv9-redis-receipt-fifo-build`. All 415 baseline and 417 candidate recorded
source files were rehashed and retained. Standalone Cargo JSON verifies empty
feature lists for the root, engine, Raft and server packages, not just the root
binary. `default-feature-build-evidence.json` records those retained build
graphs. The fixed client keeps its original Ready `892b2a1` provenance.

The unchanged protocol ran async-write before / FIFO / async-write after.
Each matrix contained GET100, PUT100 and GET50/PUT40/DELETE10 with two
three-second repetitions and reversed KV9/Redis target order: 36 cohorts.
There were 64 workers, one outstanding operation per worker, 64 hot keys,
23-byte keys, 128-byte values and 32 warmup operations. KV9 retained 1,500-ms
deadlines and at most six attempts for explicit NotLeader routing; uncertain
writes were not retried. Redis 7.0.15 retained no retries, no pipelining,
`save ""`, `appendonly no`, zero replicas and one I/O thread. Every trial stopped
by duration below its operation cap. Missing GETs are successful nil/None;
the fixed KV9 performance client does not expose hit/miss counts.

The unchanged affinity guard verified outer CPUs 0-31, clients 0-1 and the
three voters 2-5 before measurement. The previous owned Chaos fixture was
terminal and cleaned, all eight existing namespace UIDs matched, and the next
fixture was held. Only the proof launcher was stopped; active provers finished
normally before timing. Root independently verified the complete descendant
subtree was quiescent. Proof and Chaos resumed after all measured fixtures
exited. No extra live status polling or profiling was added. Logical CPU masks
and one shared host do not establish physical isolation.

## Throughput, latency and CPU

Rates divide successful logical operations by cohort elapsed time, including
drain. Parentheses retain repetition one / two. Percentage changes pool all
four surrounding baseline repetitions.

| Workload | Async write before ops/s | Receipt FIFO ops/s | Async write after ops/s | Change |
| --- | ---: | ---: | ---: | ---: |
| GET100 | 136,331.3 (137,135.8 / 135,526.8) | 135,889.9 (137,050.7 / 134,729.0) | 137,110.5 (138,187.5 / 136,033.5) | -0.608% |
| PUT100 | 78,917.2 (79,348.9 / 78,485.5) | 78,113.5 (78,017.1 / 78,210.0) | 78,397.0 (78,228.2 / 78,565.9) | -0.691% |
| Mixed | 93,163.8 (93,905.9 / 92,421.6) | 92,086.6 (92,908.7 / 91,264.4) | 92,746.4 (93,827.7 / 91,665.0) | -0.934% |

The contemporaneous standalone Redis reference remains much faster:

| Matrix | GET/s | PUT/s | Mixed/s |
| --- | ---: | ---: | ---: |
| Async write before | 508,906.7 (505,502.9 / 512,310.3) | 489,251.0 (485,874.9 / 492,627.2) | 499,444.1 (500,520.6 / 498,367.6) |
| Receipt FIFO | 507,920.3 (509,010.9 / 506,829.7) | 490,228.4 (492,368.7 / 488,088.2) | 500,791.7 (501,474.7 / 500,108.6) |
| Async write after | 507,808.8 (504,756.2 / 510,861.3) | 490,584.0 (492,684.9 / 488,483.1) | 500,667.0 (499,861.4 / 501,472.7) |

Every individual KV9 GET p99 remains in the 0.524288-1.048575 ms bucket;
every KV9 PUT/mixed p99 remains in 1.048576-2.097151 ms. Every Redis p99 is
0.131072-0.262143 ms. These are histogram intervals, not exact percentiles.

CPU observations below retain repetition one / two. Server cores sum all
three voters; clients had two allowed CPUs.

| Matrix / workload | Server cores | Client cores | Busiest client thread cores |
| --- | ---: | ---: | ---: |
| Async write before / GET100 | 3.091 / 3.152 | 1.418 / 1.406 | 0.712 / 0.703 |
| Async write before / PUT100 | 3.491 / 3.498 | 0.890 / 0.891 | 0.448 / 0.442 |
| Async write before / Mixed | 3.449 / 3.443 | 1.043 / 1.022 | 0.520 / 0.513 |
| Receipt FIFO / GET100 | 3.093 / 3.116 | 1.416 / 1.392 | 0.706 / 0.692 |
| Receipt FIFO / PUT100 | 3.514 / 3.516 | 0.878 / 0.876 | 0.439 / 0.438 |
| Receipt FIFO / Mixed | 3.455 / 3.437 | 1.028 / 1.009 | 0.512 / 0.503 |
| Async write after / GET100 | 3.116 / 3.124 | 1.426 / 1.406 | 0.709 / 0.703 |
| Async write after / PUT100 | 3.506 / 3.511 | 0.883 / 0.881 | 0.440 / 0.439 |
| Async write after / Mixed | 3.459 / 3.448 | 1.042 / 1.017 | 0.518 / 0.507 |

Candidate PUT server CPU is slightly higher despite slightly lower throughput.
The result does not establish a useful reduction in CPU per successful request.
The unchanged oldest-first receipt scan remains; this experiment isolates FIFO
maintenance, so it does not establish the benefit of a different lookup method.

Existing successful write-stage means, in microseconds (repetition one / two):

| Stage | Async write before | Receipt FIFO | Async write after |
| --- | ---: | ---: | ---: |
| Public preparation queue | 18.70 / 15.89 | 21.65 / 16.33 | 20.81 / 16.01 |
| Proposal submission | 10.58 / 11.23 | 9.28 / 11.87 | 9.72 / 11.03 |
| Application wait | 429.83 / 439.15 | 427.85 / 437.97 | 429.52 / 437.98 |
| Logical proposal wait | 442.55 / 452.34 | 439.23 / 452.20 | 441.08 / 450.80 |
| Public backend | 446.85 / 456.99 | 443.58 / 456.83 | 445.43 / 455.62 |
| Command apply | 38.46 / 39.52 | 37.57 / 39.38 | 37.44 / 39.86 |

These deltas include setup, warmup, measurement, drain, verification and metrics
export boundaries. Both revisions use the same async application-wait timing
boundaries. Every per-voter outcome/count/sum/bucket is retained and checked for
nonnegative deltas, matching counts and absent saturation/clamping. The nested
intervals overlap and include wall waiting; their means cannot be added as
independent CPU or latency components. Stage values do not establish a new
bottleneck or explain the small end-to-end differences causally.

## Complete outcomes and retained evidence

All **5,537,388 KV9** and **26,975,936 Redis** measured logical
operations succeeded. Final refusals, unknowns, read/transport failures and
client rejections were zero. Permitted NotLeader routing attempts remain
visible; a successful logical operation need not be one RPC attempt.


All three matrices passed the unchanged public/read-group/inline-fallback
checks. Legal fallback remains visible. The async-apply audit applies the same
checks independently to both baselines and the candidate: bound 128,
monotonic process-lifetime peak, stopped=false, and zero queued/in-flight work
at both boundaries. In each matrix voter 2's peak grows 0 to 1 during the first
GET trial and 1 to 64 during the first PUT trial; it then remains 64. Other
voters stay at zero. These are whole-trial/lifetime observations, not cumulative
admissions or individual-response attribution. Public request/byte occupancy,
read queues/groups and async-apply reservations all drained.

Raw evidence: `/tmp/kv9-async-write-receipt-fifo-c64-tmpfs-bracket`.
Session 89720 completed all 36 cohorts and six unchanged matrix/outcome checks.
All six additional audit/summary helpers passed their first invocation.
All 63 owned executing lifetimes exited before proof/Chaos release. No cohort
was discarded or rerun, and this bracket had no rejected observer attempt.
The separate earlier Chaos build-preparation failure is not part of this
performance cohort and is retained with its own attempt.

- `final-audit.json`: SHA-256 `ed2e192f2e30f121e97bad8ea3a43a0a8b634b95d8bdb126aeed95fab67720ab`.
- `async-apply-audit.json`: SHA-256 `153ca468b3c6c6d090adab935a89d09b83cfaa3569016ada4434e0c266e3e072`.
- `group-counter-summary.json`: SHA-256 `1db5a96991f5e66377d644059c21b0e8aded3142f35e3905c645b881cbaec303`.
- `per-repetition-latency-check.json`: SHA-256 `cfd2bd8995d00ddfb12f0f29a085d091556e90d300baa281a8a7bb0ef80caf94`.
- `latency-cpu-stage-details.json`: SHA-256 `8a5c03e3357ec34db331e2a65f63776e3044bfb64cbc621429a90d31ae17af33`.
- `write-stage-audit.json`: SHA-256 `e8ae6002e1afefedd10a9d6f36993aafa913cf1ed98fbb4003a0e6e5b88aaaf6`.
- `default-feature-build-evidence.json`: SHA-256 `904cbbd0bb35edaa147265bf2710beddce3da23cc6a045f92b1413162753cf86`.

Inventory `/tmp/kv9-async-write-receipt-fifo-c64-tmpfs-inventory.json` contains **1,728 files /
2,587,602,557 bytes**, SHA-256
`65cb50a13283aeaadaf55d9a1974d016499793e871cd218f475732379f2238af`. Every original file was reopened and verified
against its recorded length/hash. Verification is
`/tmp/kv9-async-write-receipt-fifo-c64-tmpfs-inventory-verification.json`.
Original matrices, source/build identities, scripts, histograms, CPU samples,
volatile-data copies and completed pause record remain intact. The Markdown
report is outside the inventory to avoid recursive hashing.

The normal quorum/sync protocol and fixed clients remain unchanged. Copies of
tmpfs data after execution do not supply runtime durability. Exact-FIFO Chaos,
durable storage and inherited checked protocol composition are not accepted by
this diagnostic. FIFO is not selected for performance promotion; the main
runtime and original roadmap checklist remain unchanged. No hosted CI ran.
