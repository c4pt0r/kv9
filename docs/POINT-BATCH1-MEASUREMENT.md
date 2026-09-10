# Matched point GET and BatchGet(1) measurement

Tracking: #9, #20 and #50. This measurement-client extension supports a directly
matched API comparison on old and new servers. It changes no server behavior,
consensus, write acknowledgement, or client retry policy.

The existing `kv9-batch-benchmark` accepts two configuration schemas:

| Version | `read_api` | Behavior |
|---|---|---|
| 1 | Absent | Existing bounded BatchGet/BatchPut benchmark |
| 2 | `"batch_get"` | Explicit existing batch workload |
| 2 | `"point_get"` | Point GET, requiring `batch_size = 1` and `read_percent = 100` |

An explicit null, unknown selector, missing version-2 selector, or a selector
added to version 1 is rejected. The serialized legacy configuration omits the
new field, and report versions match configuration versions. Old artifacts
remain accepted by the independent validator without schema coercion.

Only warmup and measurement select the read API. Initialization still writes
and checks the deterministic nonce-zero dataset through batch operations;
final verification remains a batch read of the complete dataset plus sentinel.
These phases remain outside the timing window. Point-mode warmup/measurement
metrics use `get`, with a zero `batch_put` population, and the report model is
`bounded_native_point_get_diagnostic`. Setup/verification keep their original
batch vocabulary. The generic rate fields retain their names for compatibility;
display point-mode rates as calls/s, not BatchGet invocations.

One shared executable must drive old/point, old/batch1, new/point and new/batch1.
Keep dataset size, seed, key/value lengths, concurrency, duration, warmup,
logical deadline, retry policy, CPU placement and histogram resolution matched.
Server builds and the shared client build have separate source/binary identity
records. Redis MGET with one key is a separately labeled reference, not Redis
GET and not a quorum-replicated durability equivalent.

Key selection, scheduler, cutoff, one-call-per-worker ownership, whole-call and
SDK latency recording, failure populations, and drain accounting are shared.
Whole-call latency includes operation construction and response validation.
Point GET can avoid the one-element vector and follows different SDK/handler
branches; this API cost is part of the end-to-end comparison. Old/new results
with the same selected API are the direct server-change control.

Prost computes the chosen request/response payload sizes. For a single key,
the singular point fields and one repeated batch entry currently encode the
same payload lengths. Tests check actual encoded sizes and selected key/reply
identity. Equal payload sizes do not imply equal CPU cost or framing behavior.

The first proposed diagnostic is 64 workers and two reversed repetitions of
the four native arms plus Redis MGET(1): ten cohorts. Short shared-host samples
must be reported as diagnostics, with both repetition values/ranges, whole-call
mean and p50/p95/p99 intervals, exact error populations and preserved failed
attempts. Throughput alone does not establish Redis parity. This source change
contains no new measurement result; candidate-specific correctness and clean
build/fixture validation remain prerequisites for timing acceptance.
