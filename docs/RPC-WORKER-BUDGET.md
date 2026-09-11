# One RPC worker per replica: scheduling experiment

This isolated candidate starts from `917243fd1b501843d75899cc167f7eff32b5bbb2`
and changes the existing multi-threaded Tokio runtime to one async worker per
replica. It preserves event interval eight, adaptive global-queue scheduling,
all I/O/time drivers and the existing blocking pool. This is not a current-thread
runtime: the worker still executes independently of external `block_on` calls.
The peer-enqueue experiment is excluded.

Pinned Tokio 1.53.1 normally selects workers from TOKIO_WORKER_THREADS or
available_parallelism. In the matched fixture all three voter processes share
CPUs 2–5; independently sizing each runtime against that same affinity can
create competing workers in addition to the three separate Raft owner threads.
The hypothesis is that a smaller async worker population reduces work stealing,
cross-worker handoff and scheduling contention for memory-resident requests.
It may also reduce useful parallelism or delay peer service behind expensive
public handlers. Both outcomes require measurement; no gain is presumed.

## Conditional safety and progress boundary

Only worker count changes. Request handlers, bounded streams, protocol state
transitions, admission, cancellation, deadlines, transport routes, Raft owner
uniqueness, tick policy, quorum confirmation and applied-index fences remain
unchanged. The Raft and engine source trees and dependencies are identical to
the base. Async execution remains concurrent by interleaving, with blocking
work using its existing separate pool.

Under the original arbitrary-interleaving safety and synchronization refinement
premises, one-worker execution selects the same guarded application transitions
or stutters. Induction therefore preserves the base's safety invariant for this
delta. It supplies no new quorum, durability, ownership or acknowledgement
authority. This conditional argument does not close outstanding machine-checked
composition or executable-refinement obligations in the base.

There is no real-time or unconditional progress guarantee. A task that blocks
the sole worker can delay every RPC on that replica; callback finiteness,
cooperative scheduling, available blocking workers and eventual I/O remain
premises. Process and fault tests must exercise this actual constructor, since
unit tests creating their own runtimes do not establish that integration.
No additional service-critical singleton is introduced across the replicated
database: each replica retains its own worker and existing failure budget.

## Experiment and acceptance

Compare against the exact event8 control with unchanged CPU masks, clients,
payloads and five-second repeated/reversed GET and BatchGet(1) cohorts. Keep
Redis as the same-run reference. Report throughput, mean/p99 latency, outcomes
and resource scope. An explicit one-worker choice is experimental, not an
established optimum for larger CPU budgets, batches, writes or mixed traffic.

Run focused checks, production-runtime leader-restart histories and correctness
smoke before timing. A useful result still requires broad local checks and
exact-source actual Chaos acceptance before selection. Sustained and write/mixed
behavior remain required before general promotion. No hosted CI is dispatched.
