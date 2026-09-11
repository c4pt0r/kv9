# Two persistent handler tasks per point stream

This experiment starts from accepted `5ee897a2f58c57bdf17ea1757c96224adf0f0dbb`.
It replaces the bounded per-request `JoinSet` with two persistent handler tasks,
each owning a bounded inbox and `FuturesUnordered`. The receive owner still
validates frames and reserves response capacity. Runtime worker count, event
interval, adaptive global-queue interval, client and all Raft transitions remain
unchanged. No diagnostic instrumentation or rejected scheduler changes are added.

Several completed children can notify the same parent task before it is polled.
This may reduce runtime scheduling work, while two parents retain independently
scheduled synchronous preparation. It also adds inbox operations and retains
per-child future allocation. Two tasks do not guarantee two occupied CPU workers.
The earlier single-owner set lost useful parallelism; no performance improvement
is assumed for this intermediate design.

## State and resource invariant

Let C = CHANNEL_LIMIT and, for one admitted stream, define:

- P: the response reservation held by the receive owner, either while awaiting
  a frame or transferring its validated job. P is zero or one.
- Q0, Q1: jobs queued in the two handler inboxes.
- A0, A1: active child futures holding response reservations.
- B: replies buffered in the original response channel.

Every job has exactly one owned reservation from that response channel. The
reservation cannot be copied; enqueueing, receiving and polling transfer its
ownership. Therefore the inductive invariant is:

`0 <= P + Q0 + Q1 + A0 + A1 + B <= C`.

The initial state has all counts zero. Each possible transition preserves it:

| Transition | Population change | Capacity/ownership argument |
| --- | --- | --- |
| Reserve before receiving | P increases by one | The response channel grants only an available slot. |
| Validate and enqueue | P decreases; one Q increases | The same owned reservation moves into one inbox. |
| Start polling a job | One Q decreases; its A increases | The worker moves the job into its active set. |
| Send a completed reply | One A decreases; B increases | Sending consumes that job's original reservation. |
| Consume a reply | B decreases | The response channel releases that slot. |
| Drop a pending/queued/active job | Its count decreases | Its owned reservation is dropped exactly once. |
| Reject a frame with a stream error | P decreases; B increases | The original reservation carries the error. |

Each inbox has capacity C. When the receive owner holds the reservation for
its new job, Q0 + Q1 <= C - 1. Consequently its chosen inbox cannot be full.
No other producer exists. The owner can use `try_send` without awaiting a
particular worker; a closed inbox or any send failure terminates the generation.
The round-robin index is always in 0..2 while input remains open. Both active
sets are independently limited to C, and the supervisor has exactly two tasks.
No unbounded stream queue or wait to fill a batch is introduced.

These counts exclude transport buffers already outside the original response
channel; the existing frame/message limits and HTTP/2 buffering contract still
apply. They also do not replace public API admission, which is separately owned.

## Cleanup and supervision invariant

The response stream, receive owner, both handler tasks and every queued job
hold references to the same stream semaphore grant. A worker declares its
grant guard before its active set, so cancellation/unwind destroys the active
children before releasing that guard. Each queued job owns an additional grant
reference. Thus the stream semaphore cannot become available while a queued
job, active child cleanup or response-stream lifetime still requires it.

Dropping Replies aborts its receive owner. Dropping the owner's JoinSet aborts
both workers; their inboxes and active sets are destroyed rather than detached.
A handler panic unwinds its worker. The owner observes that join failure and
terminates the generation, aborting the sibling. Cleanup can delay publication
of the failure; immediate cancellation before cleanup finishes is not promised.
Existing independently owned write-completion tasks retain their public
admission until exact apply settlement, even after observation is aborted.

Clean input EOF instead closes both inbox senders and waits for both workers
to drain their queued and active jobs. The owner exits only after both normal
joins. Replies keeps its grant while buffered responses remain unconsumed.
Shutdown, input errors and response closure retain the existing termination
behavior. Invalid frames never enter a worker inbox.

## Correspondence and conditional progress

Each frame dispatched by a worker invokes the unchanged Handler at most once,
with its original operation, payload and authorization. Cancellation may prevent
dispatch entirely. Authentication runs when the child is
polled; an inbox does not cache authorization results. The absolute deadline
is captured at frame reception and travels unchanged through its inbox. Tokio's
existing timeout may poll an expired inner future once; this change does not
invent a stronger no-synchronous-work-after-expiry guarantee.

For each response, the operation, ID and encoded-size check match the original
adapter. Out-of-order completion remains supported, and frame IDs do not imply
FIFO execution of overlapping operations. Erasing inbox transfers and worker
polling steps from an execution leaves an allowed interleaving of the same
handler calls and cancellations that prevent dispatch. Assuming the unchanged
handler contracts, reads still require
fresh quorum confirmation, the applied-index fence and an authorized immutable
view; successful writes still require their exact committed/applied receipt.
No previously confirmed read group is reused for a later invocation.

Each worker selects between ready children and its inbox and explicitly
consumes Tokio's cooperative budget after each iteration. A nonempty set may
poll multiple children within one iteration. Progress assumes fair runtime
scheduling, finite synchronous callbacks/destructors and eventual completion
or timeout of the underlying operations. A blocked worker can retain its share
of jobs; the sibling can progress while response capacity remains. No wall-clock
fairness bound or tolerance of indefinitely blocking authentication is claimed.

This is an inductive resource/ownership proof and conditional correspondence
argument for the adapter delta. It does not complete the open machine-checked
composition of Raft, storage, transport and implementation refinement.

## Acceptance plan

Retain the real HTTP/2 cancellation/held-destructor and panic/held-write controls.
Check sibling progress during blocked synchronous authentication, revocation
while a frame is queued, original queued deadlines, full-response backpressure
and clean EOF drain. Then run production-runtime leader-loss/restart histories
and a correctness smoke before the matched uninstrumented performance screen.
An unhelpful candidate stops at screening. A useful candidate still requires
applicable full local checks and actual Chaos Mesh acceptance before selection.
Mean and p99 latency matter alongside throughput. All CI runs locally.
