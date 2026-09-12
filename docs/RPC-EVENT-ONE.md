# Shared RPC executor maintenance at every task poll

This isolated experiment branches from selected CRC behavior on main `fddc121`.
The only executable change is `event_interval(8)` to `event_interval(1)` on the
existing two-worker Tokio runtime. Adaptive global-queue scheduling, all drivers,
blocking pool, allocator, peer streams and public request tasks stay unchanged.
The request-body observer and all held candidates are absent.

## Hypothesis and decision

The completed request-body diagnostic records about 0.44--0.48 us for c1
heartbeat/response offer-to-poll means. Under mixed traffic the leader records
0.99 us for heartbeat batches and 3.24 us for mixed batches. That does not
justify replacing this queue as the primary latency optimization. Earlier
separate confirmation-queue observations show materially longer sender/inbox
waits under mixed load. These populations cannot be added or treated as a
causal per-request breakdown.

Public and peer work share an executor. More frequent maintenance may service
socket readiness earlier while public tasks remain runnable. It also increases
polling/maintenance overhead and may reduce throughput or worsen tails. The
earlier fixed global-queue interval experiment changed a different scheduler
setting and remains rejected; this experiment does not repeat it. No gain or
pure kernel-I/O attribution is assumed.

Compare the fixed selected CRC control and standalone Redis in the existing
24-cohort forward/reverse ten-second screen: c1/c64 point GET and 50/50 GET/PUT.
Keep separate GET and PUT latency populations, all outcomes, exact source/client
bindings, CPU placement and original storage/cleanup checks. Both repetitions
must remain visible. A short screen cannot establish sustained capacity,
statistical significance, disk-durability parity or cross-host performance.
Hold the candidate if useful throughput and read latency do not improve
together; do not expand a losing candidate to full-matrix or Chaos runs.

## Conditional safety correspondence

Model the application as transitions of the existing public admission, Raft
owner, peer transport, storage and result-consumption state machines. Their
guards and updates are byte-identical in this candidate. The scheduler chooses
which ready task executes next; task order and socket readiness order were
already nondeterministic with two workers.

Erase executor-maintenance steps from a candidate execution. Each remaining
application transition is a transition of the selected implementation with the
same guard, values, ownership and effects. By induction on the projected trace,
every invariant preserved by the base under arbitrary task interleavings is
preserved by this scheduling delta. The initial application state is identical;
the induction step uses the unchanged transition, and erased maintenance steps
do not change application state. This is a conditional source correspondence,
not a proof of Tokio, a Rust memory-model refinement or a new Raft theorem.

In particular, no observer/clock authorizes a read, no new leader lease is
introduced, and no task scheduling decision substitutes for quorum ReadIndex,
group sealing, successful pump completion, applied-index coverage or epoch/view
fences. WAL sync, committed apply and write acknowledgements keep their existing
conditions. Deadlines and cancellation can fire at different real times; every
result still passes the same guards. No real-time equivalence follows.

Progress remains conditional on executor/OS fairness, eventual peer/network
availability and terminating callbacks/storage operations. More frequent
maintenance supplies neither a deadline guarantee nor a proof against a blocked
callback. No additional service or single point of failure is introduced.

## Local validation

Run formatting, server regression tests in default and experimental RPC builds,
and all-target Clippy with warnings denied using the retained-build lock. Unit
tests that construct another runtime do not exercise this constructor change.
Therefore bind a clean original default release and run actual stream/unary
leader-loss and original-directory restart histories, then independent history,
drain and lifetime auditing before timing. Keep unknown writes without replay.

A favorable screen is only a candidate for broader point/batch checks, proof
composition and actual exact-source Chaos Mesh failure injection. Existing
industrial availability/proof gates remain open. Routine CI is local; hosted
workflows remain manual. Selected production behavior remains CRC until accepted.
