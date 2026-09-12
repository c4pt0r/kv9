# Bound remote task polling during RPC traffic

The read-stage diagnostic at `40f014f` found a successful send-to-receiver-resume
mean of 42.449 us (37.47% of the observed asynchronous read wait) at c64 GET,
versus 1.653 us at c1. Quorum confirmation remained 65.425 us at c64 and
20.636 us at c1. These are instrumented, whole-client-lifecycle populations;
neither a performance-selection result nor a pure scheduler/transport trace.

This candidate sets Tokio's global task queue polling interval to eight on the
existing two-worker public/peer RPC executor. Its socket event interval remains
eight. The Raft owner is a separate synchronous thread; its oneshot completions
wake asynchronous RPC tasks from outside that executor. Sustained local task
traffic can postpone polling the global queue. The locked Tokio 1.53.1 scheduler
otherwise chooses a variable interval from its task-poll time estimate, targeting
200 us between global queue checks and clamping the task count to 2–127.

This is a specific scheduling hypothesis, not proof that all of the observed
42.449 us belongs to that queue. An interval of eight bounds the number of task
selections between prioritized remote checks; it does not bound wall-clock delay,
OS scheduling, individual task execution or end-to-end latency. Local queues
still receive prioritized turns; no new executor, worker, queue or service is
introduced. Per-request deadlines, cancellation and reservations are unchanged.

ReadIndex, sealed read groups, quorum, the whole-pump successful completion and
unified apply fence, read-view validation and durable write acknowledgements are
byte-identical to the diagnostic base. No core protocol or proof is changed.
The existing proofs remain conditional on their stated scheduling assumptions;
this configuration is not a new proof of Tokio or the full Rust implementation.

Qualification uses default-feature builds, so the optional stage observer is
absent from the timed server. Root runs the Raft/Server correctness suite and
Clippy locally, retains a clean source-bound release after cache invalidation,
then ordinary leader-loss/restart histories and the complete existing c1/c64
GET/mixed opposite-order performance screen. Throughput, per-operation mean
and p99, all outcomes, drained resources and CPU placement remain mandatory.
The c1 quorum boundary is a separate remaining target. Do not claim a new
default, Redis parity, cross-host capacity or candidate Chaos acceptance from
this hypothesis alone.
