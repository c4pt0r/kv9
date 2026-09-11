# RPC I/O event interval experiment

This isolated candidate starts from the retained jemalloc control `629bee4`.
Its sole executable change sets the production Tokio runtime's event interval
to eight scheduler ticks, instead of the pinned Tokio 1.53.1 default of 61.
The multi-threaded runtime, enabled drivers, worker selection, adaptive global
queue policy, allocator and all transport settings remain unchanged. This
does not include the rejected fixed global queue interval experiment.

Public handlers and peer transport tasks share this executor. The peer
transport already uses a persistent streaming RPC with bounded immediate
coalescing; it does not perform a fresh unary RPC for every Raft message.
The read-lifecycle diagnostic measured approximately 82 microseconds from
successful ReadIndex invocation to observed confirmation, including local
processing, transport and owner scheduling. This experiment tests whether
earlier socket-readiness processing under runnable public work helps that
path. It does not assume the whole interval is network or I/O polling delay.

An event interval controls maintenance opportunities, not a timer duration or
an absolute latency bound. Idle workers can poll I/O before the interval;
driver ownership, OS scheduling and task polling still matter. More frequent
maintenance may add synchronization and syscall overhead or reduce useful
task throughput. Eight is a screening value, not a demonstrated optimum.
The zero-time driver attempt can lose the shared-driver lock and return
without polling. Maintenance also handles scheduler statistics, shutdown and
deferred wakeups, so a measured effect is not necessarily pure socket I/O.
The unchanged adaptive global-queue policy may tune differently if the task
mix changes; no fixed global-queue value is introduced.

The existing event-driven Raft owner and monotonic election tick schedule are
unchanged. No transport route, reconnect/progress budget, group-sealing rule,
exact ReadIndex receipt, successful-pump requirement, applied-index fence,
write acknowledgement, admission limit, cancellation or deadline rule changes.
Different schedules can change operation ordering and deadline outcomes;
existing core safety arguments do not prove a latency or unconditional
progress bound for this configuration.

Run focused stream, async-read, async-batch and peer transport controls, then
actual production-runtime stream/unary leader-kill/restart histories and the
existing correctness smoke. Screen clean release binaries against the
unchanged jemalloc control with fixed clients, resources and reversed order.
Report throughput, mean/p99 latency, memory and every outcome. A negative
result is retained without promotion or a full candidate fault campaign.
A promising candidate still requires broader local checks and actual Chaos
Mesh acceptance. No Redis parity, sustained capacity or write gain is claimed.
