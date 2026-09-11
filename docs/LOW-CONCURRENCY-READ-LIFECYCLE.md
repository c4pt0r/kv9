# Low-concurrency reads: confirmation dominates the sampled barrier

Follow-up: the [direct peer body experiment](DIRECT-PEER-BODY-SCREENING.md)
completed its c1/c64 screen and was rejected for lower throughput and worse
mean latency in all eight pairs. The accepted control remains unchanged.
The next hypothesis targets the remaining producer-to-watchdog notification;
the lifecycle measurements and their interpretation below remain historical
evidence, not attribution of that regression.

At c1, the accepted-source instrumentation measures a **24.448-us GET read
barrier**, of which **20.997 us (85.9%)** is admitted invocation to observed
confirmation. Queue admission contributes 1.473 us, confirmation to result send
0.285 us, and result notification 1.693 us. BatchGet(1) shows the same pattern.
This directs the next low-concurrency experiment toward the confirmation path.
It changes no production implementation and selects no new performance version.

| Sampled stage | GET mean us | BatchGet(1) mean us |
| --- | ---: | ---: |
| Start to admitted invocation | 1.473 | 1.665 |
| Invocation to observed confirmation | 20.997 | 21.374 |
| Confirmation to result send | 0.285 | 0.290 |
| Send to receiver observation | 1.693 | 1.719 |
| Total sampled barrier | 24.448 | 25.047 |

There are **2,141 GET samples** and **2,108 BatchGet(1) samples**. All five
histograms use the same successful population. The four stage sums equal the
total exactly in integer nanoseconds: **52,343,996** for GET and **52,799,303**
for BatchGet(1). All twelve endpoint documents pass the unchanged 31-metric
validator, with stable process/exporter identity, no invalid trace or non-success
profile outcome, unsaturated counters and fresh quiescent boundaries.

## Interpretation and next implementation

The [uninstrumented concurrency curve](GET-CONCURRENCY-CURVE.md) establishes
approximately 38 us client-visible GET latency at c1 versus Redis 5.8 us.
The phase table above comes from a different, instrumented recording and its
endpoint envelope includes warmup and verification. Do not subtract the table
from that end-to-end latency as an exact remainder.

The [historical c64 lifecycle recording](RPC-PAIR-READ-LIFECYCLE.md) measured
40.770 us for result notification, while this c1 recording measures 1.693 us.
Historical c64 ran perf and this c1 run does not, so both offered concurrency
and profiler overhead differ. These observations do not isolate the causal
effect of concurrency. The historical 41 us is not an established fixed cost
of one oneshot notification. Likewise the current 20.997-us confirmation stage
includes local processing, transport, quorum and owner observation; it is not
an irreducible network round trip.

The next candidate is the peer transport's intermediate batch queue. Today,
`GrpcTransport::enqueue` publishes generation-tagged envelopes to a bounded
queue. `peer_session` receives and coalesces them, then sends each batch through
a second 16-slot channel to the tonic request body. Investigate making that
body poll/coalesce the original queue directly, removing one queue handoff.
Whether this improves c1 and c64 must be measured; this document does not claim
that either channel is the measured cause of the entire confirmation delay.

That change must solve request-body lifetime and cancellation explicitly:
tonic owns a Send + 'static body, whereas the current receive queue is borrowed
across reconnects. An old body must never consume a replacement session's queue
or replace its registered waker. Preserve route-generation fencing, receive
authority/authentication, bounded stale-message inspection, backpressure,
connection timeout/backoff and stalled-stream recovery. Use source-mapped
ownership/progress arguments and adversarial transport tests before screening
the candidate. Raft quorum, successful-pump and applied-index fences remain
unchanged. Check both c1 turnaround and c64 throughput/mean/p99; do not promote
a candidate solely for improvement in one population.

The [conditional design review](PEER-DIRECT-BODY-REVIEW.md) identifies two
specific traps: stale request bodies can consume replacement traffic or replace
its receiver waker, and a watchdog polled only by the body cannot detect a body
that HTTP/2 stops polling. The implementation needs a fresh per-RPC ownership
token checked under the receiver lock and independently polled progress
supervision. That review defines adversarial tests; it is not implementation
approval or proof completion.

## Recording and correctness evidence

The recording reuses instrumented server `d3dcea0355dd6c4ec23f0833708f6dc978412093`
and its existing default-feature release, derived from accepted `5ee897a`.
The production runtime bytes are identical to the control. Native client
`03c1c776a5dd7d1cc67491ab253e02ce51665bf8` and its original release are unchanged.
No server/client rebuild or new instrumentation is involved.

Each API runs once for five seconds at c1, with 128 warmup calls, 4,096 keys plus
sentinel, 128-byte values, batch size one and the original deadlines/retry
configuration. SDK max-in-flight is one. The exact protocol is
`kv9-read-lifecycle-c1-v1`, explicitly lifecycle-only with CPU profiling and
throughput acceptance disabled. No perf process runs and no CPU-sample threshold
is reduced. This is a three-voter, shared-host, volatile-tmpfs diagnostic with
normal Raft sync calls; it proves neither disk durability nor sustained capacity.

The first recording exits 0 in root session **94390**. Both fixture readback
and the first independent lifecycle reader exit 0. All **258,976 measured calls**
succeed (130,047 GET and 128,929 BatchGet(1)), with one observed SDK attempt per
call and no dropped calls or failure reasons. Both full 4,097-key datasets and
sentinels match. All eight owned fixture processes exit, six listener bindings
and six fresh three-replica drain documents pass, and 195 resource observations
cover the measurements. The retained data comprise 66 files / 11,164,391 bytes;
original tmpfs scratch directories are removed only after preservation.

Clients use CPUs 0–1 and voters share 2–5. The observer and three owned background
containers use 6–15,22–31. Independent outer readback verifies the invocation,
container identities, original configured/effective CPU restoration and
historical namespace UIDs. No build, test, fault, timing benchmark or audit
overlaps recording; unrelated host services remain unconstrained.

Six pre-runtime contract tests reject false c1 labels, worker/SDK disagreement,
wrong source/configuration, CPU-profile scope and report/hash mismatches. The
review also restored the inherited cleanup-error rejection before freezing and
recording. The independent reader retains all applicable lifecycle predicates;
the separate readback carries over the original CPU analyzer's applicable
non-CPU report and retained-data checks. No fixture or failed predicate was
rerun to obtain acceptance.

## Retained artifacts

Raw recording: `/tmp/kv9-read-c1-profile-run-first`.
Outer isolation: `/tmp/kv9-read-c1-isolation-first`.
Independent result:
`/tmp/kv9-read-c1-independent-preparation/results-first/analysis.json`.
The [executed helpers and result](../scripts/redis-reference/read-lifecycle-c1-v1/README.md)
retain the exact bytes and their absolute-path dependencies.

| Artifact | SHA-256 |
| --- | --- |
| Server | `08e8103b5230933bf65b4b605fe864803f766a1b3cf3776d7bb796bf4e3611e1` |
| Recording driver | `e6cbd12d0c96a214ad25246157f26677a4d636203f2926b3cc02a12aacd8b9d3` |
| Fixture readback | `413fd96724d3598596a417a8fa2db9945ea19ef2b5833760df5b8127bf76caec` |
| Independent reader | `886e78076056d1ec3fdb8b7c0f4c62cec167e5bcbcd25e6b6944f7757079842f` |
| Independent result | `8af9e6d781fbf18485f91a94f19242e3e4f5bfd8e9d670c987ca0881cc7ee71c` |
