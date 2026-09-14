# Segmented WAL vectored-write performance

Measured 2026-09-14 UTC. The isolated `writev` candidate does not improve the
selected baseline in this write screen. At concurrency 64, point throughput
changes **-0.612%** and BatchPut(64) throughput **-1.020%**; both pooled p99
intervals worsen. Keep candidate `cfd9c92` experimental and selected runtime
`11113f6` unchanged. Two short repetitions do not establish statistical
significance or a general result for real-disk or cross-host deployments.

All eight two-second smokes and sixteen ten-second timed cohorts pass the
independent audit. All **6,961,558 measured calls** succeed once, covering
**56,767,027 input items**, with no measured errors, unknown writes or dropped
slots. Smokes separately retain 734,946 successful calls and 6,796,554 items.

## Throughput and latency

Rates pool work over summed elapsed time. Latency merges original integer
histograms; percentile bounds are not averages of per-run percentiles.

| API | c | Role | Calls/s | Items/s | Mean us | p99 us |
| --- | ---: | --- | ---: | ---: | ---: | --- |
| point | 1 | selected | 19,225.607 | 19,225.607 | 51.911 | 68.608–69.631 |
| point | 1 | vectored | 19,289.972 | 19,289.972 | 51.738 | 68.608–69.631 |
| point | 64 | selected | 135,426.616 | 135,426.616 | 472.454 | 761.856–770.047 |
| point | 64 | vectored | 134,597.788 | 134,597.788 | 475.363 | 770.048–778.239 |
| batch64 | 1 | selected | 5,791.344 | 370,646.003 | 172.559 | 225.280–227.327 |
| batch64 | 1 | vectored | 5,798.055 | 371,075.527 | 172.360 | 223.232–225.279 |
| batch64 | 64 | selected | 14,037.009 | 898,368.574 | 4,558.186 | 8,781.824–8,912.895 |
| batch64 | 64 | vectored | 13,893.771 | 889,201.320 | 4,605.168 | 9,043.968–9,175.039 |

Loaded point throughput falls 0.373% in forward order and 0.853% in reverse;
its p99 interval worsens in both. Loaded batch throughput falls 0.123% and
1.912%. Forward batch p99 improves, but reverse p99 worsens and the pooled
interval rises from 8.782–8.913 ms to 9.044–9.175 ms. Low-concurrency changes
are small and change direction across repetitions: pooled point +0.335% and
batch +0.116%. Neither run order was discarded or repeated.

The candidate combines three segmented-WAL frame writes into a short-write-aware
`writev` loop without changing CRC, frame bytes, `fsync`, Raft commit, durable
apply or response fences. It starts from selected `11113f6`; it does not include
the independent CRC optimization. Fewer syscalls alone did not improve this
measured end-to-end workload. These results do not identify which downstream
stage dominates or establish that vectored I/O is slower on physical disks.

The [separate CRC screen](WRITE-CRC-PERFORMANCE.md) remains the strongest
measured write candidate: 139,402.831 loaded point calls/s and 1,063,493.134
loaded batch items/s. It was measured in an earlier campaign, so those numbers
are not a same-recording CRC/vectored comparison. The subsequent [full CRC regression](WRITE-CRC-FULL-REGRESSION-PERFORMANCE.md)
now passes read/write/mixed coverage; exact-main integration remains next.

## Configuration and acceptance

The control is `11113f68f6a5df77da1ffb4fcec850953716ffa3`; candidate is
`cfd9c927f8ecd33974100f696e6b08b227d25a41`; fixed native v3 client is
`0be806d9671e2c50701a64aa7889c8859b7648ba`. Three voters share CPUs 2–5,
clients CPUs 0–1, with helpers and owned background containers on CPUs 6–15
and 22–31. Each cohort uses 4,096 keys plus sentinel, 128-byte values, seed 71,
128 warmup calls, concurrency 1 or 64 and a ten-million-call cap. Both complete
opposite orders are retained. Uncertain writes are not replayed.

Both roles execute normal synchronization calls on explicitly **volatile tmpfs
WAL**, using loopback on one shared host. This is not a physical-disk, power-loss,
cross-host or sustained-capacity result. Redis was not rerun here. Separate
[source/recovery](WRITE-SEGMENT-VECTORED-RECOVERY.md) and [21-window actual Chaos
Mesh qualification](WRITE-SEGMENT-VECTORED-CHAOS.md) retain their original scope.

Timing session **95283** ended with `4c9a5f/0`; independent audit **16167**
ended with `e16a89/0`; summary arithmetic ended with `516849/0`. All 28 original
driver/auditor/smoke controls and five summary arithmetic controls pass.
Acceptance verifies 64 timed and 32 smoke process lifetimes exited, 48 fresh
voter drains, 48 writer/listener bindings and 3,074 resource samples. Original
container CPU assignments and namespace maps are restored.

The audit independently decodes all **64,532,128,628 logical WAL bytes**:
57,600,154,762 timed plus 6,931,973,866 smoke bytes. Combined resident allocation
is **45,490,188,288 bytes**. Codec work occurs after writer reaping and before
the next cohort, outside measured windows. Every original resource cap,
retention floor and restore reserve remains in force.

Two startup wrapper errors are retained: an unprivileged log open was refused
(`ecfab8/1`), then nested sudo changed the identity Git uses for source ownership
(`498f9b/1`). Neither created a cohort or executed a workload. Running the frozen
command from the existing root wrapper without a second sudo fixed the
invocation. No global Git trust configuration or benchmark code changed.
Successful smoke session 29714 ended with `9d5bd6/0`; independent smoke
readback passed at `3cd8ad/0`.

The [cache cleanup](write-storage-cache-cleanup-v1/result.json) reclaimed an
observed 89.93 GB before this campaign while preserving old benchmark and
Chaos/recovery data. This campaign now consumes its own retained storage;
future runs must use fresh free-space observations.

## Evidence and next step

[Portable original reporting evidence](https://github.com/c4pt0r/kv9/blob/1dc5c62b768cdde5d55aed09a60c527066b1a116/docs/write-segment-vectored-performance-v1/README.md)
preserves the complete summary, both repetitions, original reports and audit
receipts. Its 2,351 metadata files total 315,866,547 decoded bytes in 19 archive
parts (38,252,019 compressed bytes). Separate publication and independent byte
readback pass. Large local WAL objects and executables remain catalog/manifest
bound. The archive verifier does not rerun original source/runtime acceptance.
The evidence is on a separate branch; the main original-evidence archive budget
is unchanged.

Do not promote this isolated vectored candidate or spend full read/mixed
qualification on a claimed write gain it did not show. Prioritize the complete
CRC regression matrix, then the still-unmeasured frame-buffer comparison.
Keep the vectored source and all original measurements for a future justified
real-disk or combined-candidate experiment. Do not assume gains compose.
Dynamic multi-Raft and automatic splits follow the write phase. No original
industrial roadmap checkbox closes here; CI remains local.
