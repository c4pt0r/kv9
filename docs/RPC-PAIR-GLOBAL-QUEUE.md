# Global queue interval eight with two RPC workers

This isolated candidate starts from accepted `5ee897a2f58c57bdf17ea1757c96224adf0f0dbb`.
Its only executable change selects `global_queue_interval(8)` on the existing
two-worker runtime. Event interval eight, the separate blocking pool, dedicated
Raft owner, all drivers and the ordinary streaming RPC protocol are unchanged.
No profiling instrumentation is included.

## Measured hypothesis

The current two-worker-derived diagnostic `d3dcea0355dd6c4ec23f0833708f6dc978412093`
finds approximately 41 us from successful read-result send to receiver
observation, about 36% of its sampled barrier. This is delivery plus scheduling,
not pure global-queue delay. A dedicated Raft owner can wake a parked receiver
from outside the RPC runtime, causing remote-ready work to enter its global
injection queue. Already running/notified receivers need not take that path.

Pinned Tokio 1.53.1's adaptive interval targets a 200-us poll-time heuristic,
clamps its task count to 2–127 and starts at 61. A fixed interval overrides the
heuristic. Workers can also pull global work when local work is absent; neither
setting supplies a task-poll or wall-clock latency bound. If the current adaptive
interval exceeds eight, preferred global checks become more frequent; if it is
below eight, they can become less frequent. The diagnostic did not record the
actual adaptive interval. The setting may improve remote completion scheduling
or increase shared queue lock traffic and disturb local work. The experiment
assumes no gain.

The earlier `5e46a61` interval-eight experiment on the default-worker jemalloc
control regressed and remains rejected. This is a new interaction test after
the accepted event interval and worker-population changes. Its control remains
clean `5ee897a`; historical gains and losses cannot be combined algebraically.

## Conditional safety and progress

The builder setting changes runtime scheduling opportunities, not guarded
application transitions, messages, read-group identities, cancellation,
original deadlines or admission bounds. Fresh quorum confirmation, successful
pump completion, applied-index coverage, view/epoch validation and write
acknowledgement rules remain unchanged.

Assume the base's initial states satisfy invariant I, every guarded application
transition preserves I under arbitrary interleavings, and unchanged runtime/
synchronization primitives refine these transitions or stutter. Selecting a
different enabled task preserves I by induction over each finite execution.
This conditional exact-delta argument does not close machine-checked composition
or executable refinement of the whole base, nor prove Tokio's scheduler.

Progress remains conditional on cooperative scheduling, finite callbacks,
lock/storage progress and a live quorum. Synchronous dynamic-member metadata
authentication and decoding/copying can still occupy RPC workers. This setting
creates no new cluster-wide service-critical singleton and no hard progress bound.

## Screening and acceptance

Run focused checks, actual production-runtime stream/unary leader-restart
histories and correctness smoke before the unchanged five-second matched
GET/BatchGet(1)/Redis screen. Preserve clients, CPU allocations, payloads,
deadlines, repetition/order and full outcomes. Unit tests creating their own
runtimes alone do not exercise the changed production constructor.

Reject the candidate before broad validation if throughput and mean/p99 are
not useful. A useful result still requires local workspace checks and exact-
source actual Chaos Mesh acceptance. Sustained/write/mixed, larger batches,
other CPU budgets and master/default promotion remain separate gates. All
routine validation is local; no hosted CI is dispatched.
