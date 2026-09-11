# Fixed RPC global queue interval: rejected screening

The candidate `5e46a61dc35b20868a5ceeb10fe05415bbd44a0c` is **not selected**. Setting the production Tokio global queue interval to eight scheduler ticks regresses both paired repetitions against the unchanged jemalloc control `629bee4`. GET throughput falls **1.952% / 1.213%**, mean latency increases **1.987% / 1.228%**, and both p99 buckets worsen. BatchGet(1) throughput falls **1.167% / 3.085%**, with worse mean and p99 latency.

This is a valid negative experiment, not a regression promoted to master. Preserve the candidate and raw evidence, and stop before full-workspace or candidate Chaos campaigns. The earlier best retained jemalloc result remains 304,863–305,903 GET/s; this same-run control reaches 299,893–301,116 GET/s under the documented shared-host protocol.

## Comparison

Both KV9 arms use the same jemalloc allocator, ordinary streaming RPC, three Raft voters and volatile tmpfs WAL with normal quorum/read barriers and sync calls. The sole executable change is the runtime builder global queue interval. Neither arm includes the sampled lifecycle instrumentation or the separate authorization-metadata optimization.

The fixed native and Redis clients, c64 closed-loop workload, 128-byte values, 4,096 keys plus sentinel, 128 warmup calls, 1,500-ms intervals and 10-million-call cap are unchanged. The second repeat reverses the six-arm order. Clients use CPUs 0–1; measured servers share 2–5. Owned background containers use 6–15,22–31 during timing and are restored afterward. No builds, tests, fault work, profiles or audits overlap timing. Other host services remain unconstrained. Redis is standalone with save/AOF disabled, so fault tolerance and power-loss durability are not equivalent.

| Cohort | GET or batch calls/s | Whole-call mean us | p99 bucket us |
|---|---:|---:|---|
| 000-old-point-p00064 | 301,116.2 | 212.393 | 372.736–376.831 |
| 001-old-batch1-p00064 | 291,805.4 | 219.132 | 376.832–380.927 |
| 002-new-point-p00064 | 295,239.4 | 216.614 | 376.832–380.927 |
| 003-new-batch1-p00064 | 288,399.7 | 221.724 | 385.024–389.119 |
| 004-redis-mget1-p00064 | 492,682.8 | 129.751 | 243.712–245.759 |
| 005-redis-get1-p00064 | 475,768.6 | 134.388 | 245.760–247.807 |
| 006-redis-get1-p10064 | 507,266.4 | 126.030 | 233.472–235.519 |
| 007-redis-mget1-p10064 | 489,795.3 | 130.510 | 231.424–233.471 |
| 008-new-batch1-p10064 | 290,432.2 | 220.174 | 385.024–389.119 |
| 009-new-point-p10064 | 296,254.7 | 215.877 | 376.832–380.927 |
| 010-old-batch1-p10064 | 299,678.6 | 213.376 | 372.736–376.831 |
| 011-old-point-p10064 | 299,893.1 | 213.259 | 372.736–376.831 |

Quantiles are retained bucket intervals, not exact samples or averaged percentiles. These are short paired diagnostics, not sustained capacity or write/mixed results. Aggregate mean voter RSS is 53.87/54.25 MiB for old GET and 54.00/54.53 MiB for new GET; batch values are 54.55/54.43 and 54.38/54.68 MiB. The experiment offers no demonstrated throughput/latency benefit.

## Correctness and evidence

Focused checks passed: 23 point-stream tests, 15 async-read tests, five async-batch-read tests, formatting and all-target server Clippy with warnings denied. The real production-runtime stream/unary E2E passed leader loss and original-directory restart: **352 calls, 327 OK and 25 unknown**, both complete atomic histories valid. Unknown writes were not blindly replayed. The separate six-arm correctness smoke passed before timing. Unit tests that construct their own runtime are not evidence that the changed production constructor ran; the actual process histories and frozen release bindings establish that coverage.

Timing session **9006 exited 0**. The reviewed independent auditor passed on its first execution: **6,493,395 successful measured calls**, no extra SDK attempts, 40 exited lifetimes, 24 fresh drains, 24 writer/listener bindings, 351 resource observations, 2,324 source checks, and 264 retained files totaling 44,378,077 bytes. All three container CPU masks were restored and historical namespace maps preserved. Endpoint observations attest absence of allocator/interposition overrides and TOKIO_WORKER_THREADS for every voter before and after its client. All original validation predicates remain; the extra environment check strengthens source/configuration attribution.

## Interpretation and next action

Tokio 1.53.1 sends newly notified off-runtime tasks to the shared injection queue. Eight replaces the default adaptive interval, which targets approximately 200 microseconds of average poll work and clamps to 2–127 ticks. It is not always more frequent than adaptation: slow-poll workloads can select an interval below eight. Empty local queues already drain injection promptly, parked workers are notified, and running/already-notified tasks may coalesce wakeups. LIFO/cooperative processing can execute multiple polls per tick, so eight is neither a fixed poll count nor a wall-time bound. I/O/timer event polling is unchanged.

The sampled send-to-receiver delay therefore cannot be attributed entirely to the global queue interval. This screening rejects the selected knob/value for the measured workload; it does not prove that scheduler changes cannot help or that a hardware ceiling exists. Next inspect peer-message processing and I/O scheduling within the approximately 82-us invocation-to-confirmation stage, using another narrow candidate and matched screening if the source hypothesis is concrete. Preserve the sampled diagnostic and avoid claiming that a timer adjustment has identified a pure network cost.

The source argument is a scheduling refinement: no Raft group sealing, exact confirmation, successful-pump condition, applied-index fence, channel/result, reservation, cancellation, deadline or write-acknowledgement rule changes. Actual operation ordering and deadline outcomes can differ under different schedules. Existing core proofs provide no latency guarantee; this result requires no weakened safety premise. No new algorithm or unconditional liveness theorem is claimed.

## Retained artifacts

- Source: `/tmp/kv9-rpc-global-queue-interval`, pushed branch `codex/rpc-global-queue-interval`.
- Release: `/tmp/kv9-rpc-global-queue-release-first`.
- Local gates: `/tmp/kv9-rpc-global-queue-local-first`.
- Process histories: `/tmp/kv9-rpc-global-queue-process-e2e-first`.
- Runtime: `/tmp/kv9-rpc-global-queue-matched-diagnostic-attempt1/cohorts`.
- Preparation, independent readback and statistics: `/tmp/kv9-rpc-global-queue-comparison-preparation`.

Server SHA-256 `159552d32aed7ea4535ac6203ae40ab880725279354eab5e1d6bc19aac5166f6`; build manifest `7d2af1b25a99a912ea2ff697717fd53cfad172f48f08d31dc0fad7560672e039`; independent audit `4ce5604b8efbfaf822b4a4f14f8296450fabc6b5d98e86cb58f9b1a047de1375`; final statistics `f6b4e489eb9f6a6d5f14f60a48d11272bedf2006e58c5f13be28436e72048d43`. The initial derived statistics file is retained; its inner percentile labels said nanoseconds inside a microsecond object. The final file corrects labels only, with unchanged values and raw reports.

No hosted CI was dispatched. Redis parity, write/mixed performance, broader candidate promotion and automatic range splits remain open.
