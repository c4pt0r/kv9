# Read-barrier waiting diagnostic

Retained endpoint counters for the two jemalloc single-GET cohorts give mean successful read-establishment durations of **127.253 us** and **126.706 us**. The enclosing public read backend timer is **128.878 us** and **128.340 us**. The admission-to-preparation timer is about **0.084–0.085 us**.

The counters cover 465,690 and 467,255 successful leader reads respectively, including warmup/verification outside the measured throughput interval. They cannot be used as an exact decomposition of the 209-us measured end-to-end latency. The read-establishment timer is nested within the backend timer; these means must not be added. This is an offline diagnostic, not a new timing run.

In crates/raft/src/driver.rs, read_barrier_async starts this timer before minting/registering a context and stops it after ReadTicket::wait resolves. It includes registration, owner scheduling, quorum/apply progress and completion notification. It does not isolate network latency or quorum wait alone. Low sampled Raft CPU therefore does not show that this path is cheap in elapsed time.

After screening the already implemented authorization-map experiment, measure the read lifecycle's registration-to-submission, quorum observation, apply catch-up and return-notification delays on the retained best control. Correlate them with group size and queue depth before choosing the next change. Preserve the sealed ReadIndex group rule, normal quorum confirmation, applied-index fence, admission and cancellation ownership. No lease, stale read or relaxed acknowledgement is proposed.

No production source was changed for this diagnostic. Input hashes and exact counter deltas are retained in analysis.json. PID/exporter/node/schema identity and unsaturated counter checks pass. Further within-window instrumentation is needed for a causal latency breakdown.

Retained local evidence: `/tmp/kv9-read-barrier-diagnostic-first`; analysis SHA-256 `66d4a5fec259d8df776a74eb26ecd0fd4ee3b3e1c07242373b940931ebe34beb`. The throughput comparison is [documented separately](JEMALLOC-SERVER-PERFORMANCE.md).

## Retained success-counter deltas

Each mean is `(after.sum_ns - before.sum_ns) / (after.count - before.count) / 1000`.

| Cohort | Timer | Count delta | Sum delta ns | Mean us |
|---|---|---:|---:|---:|
| 002-new-point-p00064 | public_raw_read_prepare_queue | 465,690 | 39,039,312 | 0.084 |
| 002-new-point-p00064 | public_raw_read_backend | 465,690 | 60,017,145,019 | 128.878 |
| 002-new-point-p00064 | raft_read_establishment | 465,690 | 59,260,329,324 | 127.253 |
| 009-new-point-p10064 | public_raw_read_prepare_queue | 467,255 | 39,681,941 | 0.085 |
| 009-new-point-p10064 | public_raw_read_backend | 467,255 | 59,967,365,746 | 128.340 |
| 009-new-point-p10064 | raft_read_establishment | 467,255 | 59,204,164,624 | 126.706 |
