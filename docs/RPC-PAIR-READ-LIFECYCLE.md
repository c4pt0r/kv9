# Two-worker read lifecycle: confirmation and result notification remain dominant

The refreshed diagnostic on `d3dcea0355dd6c4ec23f0833708f6dc978412093`, derived from accepted
uninstrumented two-worker source `5ee897a`, measures a **112.716-us GET**
sampled barrier and **113.940-us BatchGet(1)** barrier. Confirmation contributes
about 59% and result notification about 36%. The current notification interval
remains approximately 41 us, so result delivery and receiver scheduling remain
useful optimization targets.

| Sampled stage | GET mean us | BatchGet(1) mean us |
| --- | ---: | ---: |
| Start to admitted invocation | 2.925 | 2.806 |
| Invocation to observed confirmation | 66.634 | 67.316 |
| Confirmation to result send | 2.387 | 2.410 |
| Send to receiver observation | 40.770 | 41.408 |
| Total sampled barrier | 112.716 | 113.940 |

GET has **25,983** successful samples; BatchGet(1) has **25,461**. All five
histograms use identical successful populations, with zero trace errors or
other profile outcomes. The four stage sums equal the total exactly in integer
nanoseconds: 2,928,702,229 for GET and 2,901,032,188 for BatchGet(1). All twelve
endpoint documents pass the unchanged 31-metric validator, with stable process/
exporter identity, unsaturated counters and fresh quiescent drain boundaries.

## CPU evidence

The unchanged CPU analyzer accepts 2,956 GET and 2,954 BatchGet(1) on-CPU
samples inside their five-second measurement windows, with no lost samples
and all 32 aggregate time bins covered. GET leaf categories include
scheduling/synchronization **17.19%**, RPC/framing/buffers **15.53%**,
allocation/copy/comparison **13.02%**, kernel network **6.46%**, metadata
lookup **3.42%** and engine/storage **2.10%**. Other and unknown symbols remain
visible; these categories do not form a complete causal decomposition.

These are on-CPU populations, not end-to-end latency fractions. They must not
be added to the elapsed-stage percentages or used to predict additive gains.
Neither the phase trace nor this CPU sample establishes a NIC/kernel ceiling
or the benefit of DPDK.

## Exact source and acceptance

The six executable instrumentation files are byte-identical to the earlier
`2ec6fcb` diagnostic; only the diagnostic document's source-scope paragraph
changes. Production runtime.rs is byte-identical to accepted `5ee897a`, retaining
two async workers and event interval eight. Instrumentation changes no quorum,
read-group, successful-pump, applied-index, deadline or cancellation authority.
This remains a diagnostic build, separate from performance candidate selection.

Focused local validation passes **214 Raft tests**, four observability tests,
formatting and all-target Raft/server Clippy. Production-runtime streaming/
unary leader-kill and original-directory restart histories and independent
audit pass **358 calls: 332 OK, 26 unknown**; all seven processes exit.

Recording session 32881 exits 0. The unchanged CPU analyzer and original
fixture readback exit 0 in session 74397. All **3,324,772 measured calls**
succeed with one SDK attempt each. Both complete 4,097-key nonce-zero datasets,
six fresh two-export drains, 194 in-window resource observations and eight
owned fixture lifetimes pass readback. Both bounded PID-scoped perf recordings
and all their helper lifetimes finish; original raw recordings are retained.

The independent reusable lifecycle reader first reproduces the historical
accepted per-node counts and integer sums exactly, then accepts the current
run on its first attempt. All 36 bound inputs remain unchanged. Its reader
and the original analyze/readback helpers remain frozen. No hosted CI runs.

## Next experiment and interpretation limits

The historical [629-derived trace](READ-LIFECYCLE-WAIT-RESULTS.md) reported
82.119/83.775-us confirmation and 37.779/39.261-us notification stages.
Those are not contemporaneous matched controls, so their differences do not
isolate event interval or worker count. The accepted uninstrumented performance
result remains [344,412–344,790 GET/s](RPC-WORKER-PAIR-PERFORMANCE.md).

The dedicated Raft owner can wake a parked receiver from outside the RPC
runtime's workers. Pinned Tokio 1.53.1 schedules such remote-ready tasks via
its global injection queue. Its default polling interval adapts from a task-poll
EWMA toward a 200-us heuristic, clamped to 2–127 polls and initialized at 61;
this is not a latency bound. Workers can also drain the global queue when
local work is absent. These facts do not prove all 41 us is global-queue delay.

Next test the already studied fixed global-queue interval of eight on the
current two-worker/event8 control. The prior 629-derived experiment regressed
and remains rejected; this is a separate interaction test after a material
worker-population change, motivated by the refreshed notification evidence.
Keep the new candidate free of instrumentation and compare both throughput
and mean/p99. Stop before broad acceptance if it regresses or provides no useful
gain. Quorum and applied-index requirements remain mandatory.

Each API has one instrumented five-second recording on the shared host,
64 closed-loop clients and 128-byte values. Clients use CPUs 0–1, voters share
2–5, and profiler/helpers use 6–15,22–31. No build, test, fault or audit overlaps
recording. Ordinary Raft sync calls use volatile tmpfs; disk/power-loss and
cross-host behavior are outside scope. Approximately one in 64 successful
registrations is sampled. Lifecycle endpoints include warmup/verification,
so their stages do not exactly partition the measurement-only end-to-end
latency. Confirmation includes local processing, transport, quorum and owner
observation; notification includes delivery and scheduling. Instrumentation
and profiling can perturb both. Sustained/write/mixed/larger-batch and failure
latency remain separate requirements.

Raw evidence: `/tmp/kv9-rpc-pair-read-profile-run-first`. Independent stage analysis:
`/tmp/kv9-rpc-pair-read-profile-independent-preparation/results-first/analysis.json`. Source and release:
`/tmp/kv9-rpc-pair-read-profile`, `/tmp/kv9-rpc-pair-read-profile-release-first`.

| Artifact | SHA-256 |
| --- | --- |
| Server | `08e8103b5230933bf65b4b605fe864803f766a1b3cf3776d7bb796bf4e3611e1` |
| Build manifest | `0d724224a936d3195c877d9c28c069f9460c63ee3bf7d8c04a7deb30f0c68c26` |
| CPU analysis | `26d3183c41698820df6675908af0eafeaf7b77599bd15f64069d4320eaf4a877` |
| Fixture readback | `5462b0e73dd676955382885b003ed0e04477bde45a3d2e8603a43795944bbd12` |
| Independent lifecycle analysis | `838b0564d4e6a12b1455f4aab9d6a8e06bdd4662c171597975e5f6574c421199` |
| Process audit | `f19a2c71fbaba068910a69a6826281bebd0ceb21e68724116f29f0f6b4c35802` |
