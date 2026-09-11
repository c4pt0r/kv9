# RPC global queue interval experiment

This isolated candidate starts from the retained jemalloc server `629bee4`.
The sole executable change sets the production Tokio runtime's global queue
interval to eight scheduler ticks. The runtime remains multi-threaded with
all drivers enabled and the original default worker count. No other builder,
transport, allocator, Raft, storage or admission setting changes.

The sampled read diagnostic found about 38 microseconds between successful
result send on a Raft owner thread and receiver observation. That interval
includes channel delivery and receiver scheduling; it does not isolate one
queue. Tokio 1.53.1 routes wakeups originating outside an executor worker
through its shared injection queue. Checking this queue more often may reduce
completion latency while local work remains ready. More frequent checks can
also add synchronization overhead or impair locality. Eight is an experiment
value, not a measured optimum or an operator configuration recommendation.
The explicit interval replaces the default adaptive policy, which targets
approximately 200 microseconds of average poll work and clamps its interval
to 2–127 ticks. Workers also check the injection queue when local work empties;
already running/notified tasks need not newly enter that queue. The candidate
therefore does not assume every observed notification delay has this cause.

The change affects scheduling of public handlers and peer transport workers.
It does not grant a read receipt or change when a write is acknowledged.
Every read still requires the original sealed group's exact ReadIndex
confirmation and applied-index coverage after a successful pump. Existing
deadlines, cancellation, bounded queues, stream slots, authentication and
write-completion ownership are unchanged. A scheduling change may alter which
valid operation wins a race or reaches a deadline; outcome histories remain
part of acceptance. Progress still assumes eventual task and owner scheduling.

Use existing focused stream/read correctness tests and real process
leader-kill/restart histories before screening exact release binaries against
the unchanged jemalloc control. Run matched GET and BatchGet(1) with the fixed
clients, reporting throughput, mean/p99 latency, all outcomes and memory.
Do not include lifecycle instrumentation in either timing arm. Reject the
candidate if it does not deliver a useful result; promising performance still
requires broader local correctness and actual Chaos Mesh acceptance before
promotion. No Redis parity, sustained capacity or write improvement is claimed.
