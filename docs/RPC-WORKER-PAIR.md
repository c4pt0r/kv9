# Two RPC workers per replica: scheduling experiment

This isolated candidate starts from `917243fd1b501843d75899cc167f7eff32b5bbb2`
and explicitly selects two asynchronous workers for the existing multi-threaded
Tokio runtime. Event interval eight, adaptive global queue scheduling, all
drivers, the separate blocking pool and dedicated Raft owner remain unchanged.
The unsuccessful peer-enqueue change is excluded.

The experiment tests an intermediate worker population under the existing
three-voter shared CPU budget. Two workers retain parallel RPC execution while
reducing independently sized runtimes' competing workers. This may reduce
scheduling/handoff overhead or instead constrain useful parallelism. No
performance improvement or generally optimal worker count is presumed.

## Conditional correspondence and limits

The only executable delta is worker count. Handler and protocol transitions,
bounded queues, cancellation, deadlines, admission, route generations, Raft
owner uniqueness and tick rules remain unchanged. Successful reads still need
their existing fresh quorum confirmation, successful pump, applied coverage
and view/epoch gates. Write acknowledgement conditions remain unchanged.

Assume the base's initial protocol states satisfy invariant I and every guarded
application transition preserves I under arbitrary task interleavings. Assume
the unchanged synchronization/runtime primitives refine those transitions or
stutter. The candidate merely selects their timing/order; induction on every
finite execution preserves I. This is a conditional exact-delta argument,
not a machine-checked Tokio proof or closure of the base's outstanding formal
composition and executable-refinement obligations.

Liveness remains conditional on finite callbacks, cooperative scheduling,
lock/storage progress, available blocking workers and a live quorum. Production
dynamic-member authentication can synchronously access metadata; decoding and
resident copies also occupy RPC workers. Two workers do not eliminate those
stall risks. The initial-voter read fixture does not cover dynamic-member
authentication or prove a wall-clock bound. Replica-local worker count creates
no additional service-critical singleton across the replicated database.

## Acceptance

Run focused tests, actual production-runtime stream/unary leader-restart
histories and correctness smoke before the unchanged five-second matched
GET/BatchGet(1)/Redis screen. Unit tests creating their own runtimes alone do
not exercise this constructor. Keep clients, CPU masks, payloads, deadlines,
repetitions and reversed arm ordering identical to the accepted control.

Only a useful throughput and mean/p99 result warrants broader local workspace
checks and exact-source actual Chaos acceptance. Sustained traffic, write/mixed,
larger batches and other CPU budgets remain separate promotion requirements.
All routine validation is local; no hosted CI is dispatched.
