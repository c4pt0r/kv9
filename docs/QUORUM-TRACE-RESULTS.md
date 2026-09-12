# Quorum path capture with immutable event slots

The repaired diagnostic collector records all selected events in six process prefixes. The unchanged reader accepts 2,318 leader-local heartbeat-dequeue to response-validation candidates and 1,159 exact group-confirmation chains. This localizes the expensive round trip; it does not establish a database speedup or a network-only bottleneck. Selected runtime remains `11113f6`.

The [original collector result](quorum-trace-results-v1/REPORT.md) is retained in full: 1,083 observations lost to its shared mutex prevented complete-context timing. Isolated source [`7d45612`](https://github.com/c4pt0r/kv9/commit/7d456126b020050a3cf77b61c515764dda77ee65) replaces that mutex with bounded, uniquely reserved immutable cells. Sampling, event schema, acceptance reader and protocol boundaries stay unchanged.

## Fixed-client overhead comparison

Each original cohort runs closed-loop c1 GET for 5 seconds, with 4,096 keys plus sentinel, 128-byte values, seed 71 and 128 warmup calls. The fixed streaming client and ordinary three-voter quorum/sync configuration are unchanged. There is no c64 or Redis trial in this campaign.

| Cohort | Successful calls / attempts | GET/s | Mean µs | p99 µs |
| --- | ---: | ---: | ---: | ---: |
| control-c1-r0 | 141,505 / 141,505 | 28,300.825 | 35.215 | 45.056–45.567 |
| trace-c1-r0 | 140,055 / 140,055 | 28,010.845 | 35.577 | 45.056–45.567 |
| trace-c1-r1 | 139,764 / 139,764 | 27,952.655 | 35.650 | 45.568–46.079 |
| control-c1-r1 | 140,858 / 140,858 | 28,171.583 | 35.375 | 45.056–45.567 |

All 562,182 measured calls succeed once. No measured outcome is discarded. The diagnostic build reduces throughput by 1.025% / 0.777% and increases mean latency by 1.028% / 0.777% in the two orders. This includes both tracing and six read-stage histograms. Selected contexts incur more observation work than unsampled contexts; low aggregate overhead does not prove neutral per-sample timing. No statistical-significance or cross-campaign speedup claim is made.

## Process-local observations

All six captures have zero contended/full/poisoned/exhausted observations. The sampled prefix includes initialization, warmup, measurement, verification and post-client activity. It is not a measurement-only population. Means below must not be added across replicas or subtracted from client latency. Node 2 is the observed leader in both trace cohorts.

| Process | Events | Group chains | All-stage complete contexts | Inbox admission → drain mean µs | Outbound offer → dequeue mean µs |
| --- | ---: | ---: | ---: | ---: | ---: |
| trace-c1-r0 / n1 | 4,642 | 0 | 0 | 1.752 | 2.103 |
| trace-c1-r0 / n2 | 11,600 | 580 | 235 | 1.523 | 2.090 |
| trace-c1-r0 / n3 | 4,640 | 0 | 0 | 1.792 | 2.223 |
| trace-c1-r1 / n1 | 4,634 | 0 | 0 | 1.771 | 2.103 |
| trace-c1-r1 / n2 | 11,580 | 579 | 195 | 1.490 | 2.106 |
| trace-c1-r1 / n3 | 4,632 | 0 | 0 | 1.817 | 2.213 |

| Leader-local round trip | Matched candidates | Mean µs |
| --- | ---: | ---: |
| trace-c1-r0 / n2 → n1 → n2 | 580 | 17.566 |
| trace-c1-r0 / n2 → n3 → n2 | 580 | 17.006 |
| trace-c1-r1 / n2 → n1 → n2 | 579 | 17.092 |
| trace-c1-r1 / n2 → n3 → n2 | 579 | 17.111 |

The pooled leader-local round-trip means are **17.286 / 17.102 µs**. Exact group admission → confirmation means are **22.170 / 21.963 µs**; confirmation → completion eligibility is **0.285 / 0.297 µs**. Follower validation → driver-step means are **2.591–2.735 µs**, with driver-step → response-offer means **0.479–0.490 µs**. These overlapping and differently sampled intervals are not an additive decomposition.

All 2,318 leader-local candidate edges have the required intermediate observations. A unique observed match is still not a wire-level transmission identity. 729 response-step → confirmation spans are reversed because that response was stepped after the group had already been confirmed; those spans remain unavailable, not zero. The reader reports 430 complete all-stage leader contexts and 1,159 complete group chains. Follower contexts have no local leader-group chain. Three accepted → dequeued ticket spans are reversed because acceptance is observed after publication. All stage counts, distributions and unavailable reasons remain in [the numeric summary](quorum-trace-slots-results-v1/summary.json), [stage counts](quorum-trace-slots-results-v1/stage-counts.csv) and [span accounting](quorum-trace-slots-results-v1/span-accounting.csv).

## Next performance implementation

The largest measured span still contains outbound transport, remote processing and the return path. The new local follower observations identify roughly 1.75–1.82 µs of inbox residence before a roughly 0.49-µs driver-to-response segment. A bounded owner-poll experiment will test whether avoiding a park/wake cycle removes useful latency. This is a different variable from earlier rejected runtime worker/global-queue/body-handoff changes. Freeze one 32-µs polling budget, capped by the existing tick deadline, with the original mutex/condition-variable predicate retained as authority. Measure CPU cost as well as c1/c64 GET and mixed-load throughput, mean and tails; busy polling can reduce CPU efficiency or regress throughput on shared cores. No result or promotion is assumed.

Before timing: prove the polling steps refine the existing scheduling state machine, check stale-hint, publication/park, stop and deadline races, and qualify a clean default production build plus ordinary recovery. Keep notification coalescing `42e0117` separate. Any promotion requires full relevant API/fault checks and actual Chaos Mesh. DPDK remains conditional on evidence specific to kernel network I/O.

## Validation and exact evidence

The slot source passes 17 focused tests and 455 diagnostic Raft/server tests (one existing ignored), formatting and Clippy. These test populations overlap; [original source evidence](https://github.com/c4pt0r/kv9/tree/7d456126b020050a3cf77b61c515764dda77ee65/docs/quorum-trace-slots-source-v1) and the observational correspondence argument retain their limited scope. Diagnostic production build `ca78423c60abe2293eca018d8c964bb823904e486053b1e43ddb6ff0a2ffd12f` is bound to clean source `7d456126b020050a3cf77b61c515764dda77ee65`, with production quorum-trace/read-stage features and no test/RPC-experiment feature. Control and fixed-client hashes are retained in the original protocol and summary.

Runtime session **69144/0** (`153abe`) and independent readback **33748/0** (`692176`) are terminal. Six process captures, 16 exited owned lifetimes, 12 fresh drains, 24 metric documents, 12 listener bindings and outer restoration pass. The preparation is frozen at inventory `8ecafa87f3cbb9e51166757cf20561fa96938d0d39b9c649aab0b848fea93be5`. The second campaign changes only the observer implementation/source binding; all original workload durations, ordering, CPU placement, capacity and storage guards remain. No original failed capture or completed cohort is relabeled or replaced.

The [original evidence archive](quorum-trace-slots-results-v1/original-evidence.tar.gz) and [member inventory](quorum-trace-slots-results-v1/archive-inventory.json) retain exact acceptance inputs, readers, results and root receipts. Every archive member was independently read back without extraction (`7e04e6/0`): 103,793,205 raw bytes / 19,410,367 compressed bytes, SHA-256 `bb457dac0bb092fb1db2742d5edc0b2806f48a5e9f1673728d48cda099994e87`.

Shared-host loopback and tmpfs WAL remain explicit limits. No real-disk, independent-host, new recovery/Chaos, core-proof, Redis-parity, dynamic multi-Raft or split acceptance follows from this diagnostic. Fresh Safe ReadIndex, sealed groups, whole-pump/apply/view fences and durable ACK behavior remain intact. CI stays local.
