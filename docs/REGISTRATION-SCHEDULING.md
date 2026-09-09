# Registration execution and capacity

Tracking: [#36](https://github.com/c4pt0r/kv9/issues/36). This is a scheduling
repair at the Raft gRPC boundary, preserving the existing membership protocol.

## Failure and repair

The registration handler called a synchronous backend directly on a Tokio async
worker. That backend holds the catalog planner mutex and waits for Raft commit
and application. Those waits depend on network tasks scheduled on the same
runtime. Enough waiting RPCs can therefore prevent the network from making the
progress needed to complete the RPCs.

The hosted membership scene at `8c0feb0` retained a learner in `Joining` with 233
registration errors and repeated elections. A separate deterministic test holds
the backend on a one-worker runtime: authenticated discovery cannot progress on
the original code, but succeeds while registration is still held after the repair.
The test releases the backend from an external thread before asserting failure,
so the negative case terminates without an executor-dependent timeout.

`RaftGrpcService::register` now validates the authenticated identity and request,
tries to acquire a service-local capacity permit, and moves the synchronous call
to `spawn_blocking`. There is one permit because the current registration backend
already serializes catalog planning. An occupied slot returns `ResourceExhausted`
immediately; it does not enqueue another semaphore waiter or blocking task.
The blocking closure owns the permit until the actual backend call returns or
unwinds. Cancelling the RPC does not free capacity while its mutation is still
running. This matches [Tokio 1.53.1's cancellation contract](https://docs.rs/tokio/1.53.1/tokio/task/fn.spawn_blocking.html).

## Capacity invariant and proof argument

Let `A` be the available permit count and `O` the set of owners of this service's
single permit, including the short reservation before spawning. Let `J` be its
queued or running blocking registration jobs. The invariant is
`A + |O| = 1` and `|J| <= |O| <= 1`.

1. Initialization creates exactly one available permit and no owner/job.
2. Successful acquisition atomically transfers the available permit to the
   handler. A failed acquisition changes no state and spawns no job.
3. Moving the owned permit into the closure transfers ownership without cloning
   the permit. Spawning, queuing and starting that closure preserve the count.
4. Completion or unwinding drops the closure's permit and releases capacity.
   A reservation abandoned before spawning also releases its permit.
5. Cancelling the outer RPC only discards its wait for the job result. The job
   retains the permit; therefore cancellation cannot increase admission capacity
   while that backend call remains queued/running.

Induction over those transitions establishes at most one outstanding registration
backend job per service. This is an ownership argument using Rust RAII and Tokio's
semaphore/task contracts; it is not a new machine-checked theorem about Tokio or
Rust. Unbounded public-RPC queues elsewhere are outside this local bound.

The cancellation test holds the backend, cancels its caller, checks three overload
refusals with only one backend invocation, then releases it and verifies another
exact receipt. Moving permit ownership back to the RPC future makes that test
fail at `cancelled RPC released capacity before backend completion`.

## Protocol refinement and progress

Identity and request validation still precede backend execution. The backend's
ticket checks, catalog mutex, ordered planning barrier, submission term fence,
ConfChange application and exact registration receipt are unchanged. Successful
responses copy the backend's same term/index and membership sets; a task failure
returns an error. No timeout or cancellation is reclassified as a confirmed
failure to commit, and no receipt is synthesized from an applied watermark alone.

In the existing metadata and Ready models, deferring execution is stuttering;
each admitted backend retains its original ordered protocol transitions. New
overload refusals add no catalog or Raft transition. Thus this change preserves
the assumptions of the existing 47-declaration/534-obligation TLAPS inventory and
both TLC families. It does not mechanically prove their composition with Rust or
extend the create-keyspace model to the complete membership protocol.

Conditional progress requires a scheduled blocking worker, eventual catalog-lock
availability, a connected Raft quorum, successful storage/application, and client
deadlines sufficient to observe the receipt. Registration no longer occupies the
async worker while waiting for those conditions. The capacity gate itself makes
no fairness promise between many joining clients. The bounded retry policy and conditional seed coverage are documented in
[registration routing](REGISTRATION-ROUTING.md). Broader admission control and
cross-host failure domains remain separate obligations; no cluster-wide coordinator or singleton service is introduced.

## Validation

Run the deterministic scheduling/cancellation tests and existing wire tests with:

```sh
cargo test -p kv9-raft
scripts/dynamic-membership-e2e.sh
```

The full membership scenario retains its original deadlines, typed invalid-ticket
rejection, admission preservation, learner/promotion checks, exact failover receipt
and full-restart checks. Hosted acceptance also runs the full workspace, MinIO
recovery, both model families, the deductive inventory and actual Chaos Mesh with
full-history replay and every-voter I/O failure/recovery evidence.
