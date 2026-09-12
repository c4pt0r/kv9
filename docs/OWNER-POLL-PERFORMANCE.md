# Bounded owner polling: rejected performance experiment

Reject the fixed 32-us owner-poll candidate `2ca5fcc`. All eight matched
workload/repetition comparisons lose throughput and worsen mean and p99.
Every pooled workload also uses more server CPU. Main keeps the selected
uninstrumented ThinLTO runtime `11113f6`; no poll-budget sweep or losing-cohort
replacement was run.

The [source/proof checkpoint](https://github.com/c4pt0r/kv9/blob/2ca5fccb157b26b6c3c79eb52f7c7838f10a5c8c/docs/BOUNDED-OWNER-POLL.md)
and [clean build / ordinary recovery](OWNER-POLL-RECOVERY.md) remain valid within
their scopes. They do not establish that busy polling improves performance.
Safe ReadIndex, sealed groups, successful pump/apply/view fences and durable
write acknowledgements remain unchanged in both compared servers.

## Same-run point results

Both complete ten-second repetitions are pooled by operation count and elapsed
time. Quantiles merge raw histogram buckets; no percentile is averaged.

| Cell | Selected KV9 QPS | Poll candidate QPS | Change | Redis QPS |
| --- | ---: | ---: | ---: | ---: |
| c1 GET | 28,252.671 | 22,895.144 | -18.963% | 173,925.251 |
| c64 GET | 374,812.758 | 280,167.048 | -25.251% | 511,910.764 |
| c1 mixed 50:50 | 21,450.361 | 17,583.188 | -18.028% | 171,553.680 |
| c64 mixed 50:50 | 190,236.125 | 167,369.568 | -12.020% | 499,044.195 |

| Cell | Selected mean us | Candidate mean us | Selected p99 us | Candidate p99 us |
| --- | ---: | ---: | ---: | ---: |
| c1 GET | 35.280 | 43.561 | 47.104–47.615 | 81.920–82.943 |
| c64 GET | 170.629 | 228.306 | 323.584–327.679 | 368.640–372.735 |
| c1 mixed 50:50 | 46.510 | 56.763 | 65.536–66.559 | 121.856–122.879 |
| c64 mixed 50:50 | 336.286 | 382.247 | 557.056–565.247 | 679.936–688.127 |

The [complete readout](owner-poll-screen-v1/README.md) retains separate mixed
GET/PUT populations and Redis latency. [Every original repetition](owner-poll-screen-v1/PER-REPEAT.md)
shows the same regression direction; neither order is discarded.

## CPU cost and interpretation

| Cell | Selected server cores | Candidate server cores | Redis server cores |
| --- | ---: | ---: | ---: |
| c1 GET | 1.6084 | 3.3527 | 0.5273 |
| c64 GET | 3.0290 | 3.4364 | 0.9950 |
| c1 mixed 50:50 | 2.1301 | 3.3612 | 0.5284 |
| c64 mixed 50:50 | 3.5049 | 3.6655 | 0.9959 |

These [CPU estimates](owner-poll-screen-v1/cpu.json) sum the three voter
processes for KV9 and the single Redis process. They use each process's retained
sample interval, weighted by observed seconds. They include all process work
within those intervals, may omit measurement edges and are not exclusive
request service time. The original process resource samples and coverage are
published with the reports.

The increased CPU use and worse tails are consistent with polling competing
for the shared four server cores. This is an interpretation, not a measured
call-stack or causal attribution. The earlier 1.752–1.817-us follower inbox
residence did not establish that park/wake time was freely removable. This
screen rejects the concrete 32-us scheduling policy under the retained setup;
it does not prove that every polling architecture is slower.

## Protocol and accepted evidence

The screen retains 12 two-second smoke cohorts and 24 ten-second timed cohorts:
c1/c64 GET and point mixed 50:50, selected/candidate/Redis, two complete opposite
orders, 4,096 keys plus sentinel, 128-byte values and the unchanged v3 clients.
The fixed calls/deadlines/attempt limits, resource floors, actual final-byte
checks, full outcome accounting and source/build/identity checks remain intact.

All **49,184,881 measured calls** succeed with one attempt and no dropped slots.
Initialization routing attempts stay separately accounted. Independent readback
accepts 80 exited lifetimes, 48 fresh drains, 48 voter writer/listener bindings,
4,680 resource samples and 642 retained database files / 4,739,356,586 bytes.
All three owned containers recover their exact configured/effective CPU masks
and namespace maps. Runtime `53149`, independent audit `39270` and the bounded
statistics derivation terminate successfully on their first invocation.

The [publication inventory](owner-poll-screen-v1/archive-inventory.json) binds
2,077 archived files, including raw reports/configuration, process CPU samples
and coverage, source/execution/cleanup records, accepted audit and statistics.
High-volume host/call/dataset diagnostics and database directories remain
locally retained at their original paths, enumerated separately. The
[original accepted inventory](owner-poll-screen-v1/accepted-input-inventory.json)
retains the timed data hashes. Publishing this subset does not remove or
replace any original local fixture evidence.

KV9 uses three voters with quorum and sync calls on tmpfs WAL. Redis uses
standalone RAM with persistence disabled. This is a shared-host loopback point
screen, not full72, equal durability, statistical significance, sustained
capacity, independent-host or power-loss acceptance. No new actual Chaos run
is spent qualifying this rejected candidate, and no original industrial
checklist item closes.

## Next development step

Keep the selected runtime and record this rejection in the experiment index.
Before another scheduling or transport rewrite, reuse the existing traces and
all-branch profiles, then identify a concrete CPU/call-stack cost on the exact
selected build. Any new profile must run separately from performance timing;
do not infer a removable cost by subtracting unrelated interval means. No
retuning of this poll budget, blind worker/queue/executor revisit or lease-read
shortcut is selected. Core proof, actual Chaos, no service-critical singleton
except object storage and Redis read parity remain prerequisites for the later
dynamic multi-Raft and automatic range-split phase.
