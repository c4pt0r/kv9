# Direct peer request body: ownership, bounds and progress

This candidate replaces the two-stage peer send path with one bounded envelope
queue consumed directly by the tonic request body. The accepted control is
`5ee897a2f58c57bdf17ea1757c96224adf0f0dbb`. This is a transport experiment;
Raft state transitions, fresh read confirmation, applied-index checks, exact
write outcomes and inbound authority are unchanged. No throughput or latency
improvement is claimed before matched measurement.

The change is more than deleting the 16-slot intermediate batch channel.
`grpc/direct_body.rs` replaces the original envelope mpsc with a mutex-protected
`VecDeque`, allowing queue consumption, request-body ownership and receiver-waker
ownership to use one synchronization boundary. There remains one owned worker
per peer, one 4096-envelope queue and one current route watch value. Each tonic
body builds its batch from immediately available traffic without a batching
timer. The existing gRPC endpoint, authentication, connect timeout, PING
keepalive and reconnect backoff are retained.

## State and linearization points

For one queue, write its locked state as `(Q, S, R, A, W, P)`: FIFO envelopes,
sender-open flag, receiver-open flag, active RPC token or absence, optional body
waker, and optional backlog-progress timestamp. An envelope captures its route
allocation when admitted. A body captures both its route allocation and a fresh
RPC token. The two identities serve different purposes: even a reconnect to the
same unchanged route must invalidate the preceding body.

Fresh tokens are distinct live `Arc<()>` allocations. Retaining a token keeps
its allocation alive, so its address cannot be reused while a stale body can
still compare it. Ghost natural-number labels may name these allocations in a
mathematical argument; there is no wrapping integer counter in the code.

The queue mutex orders each operation below. The outer route mutex orders
registration and enqueue; queue operations never acquire that outer mutex.

| Operation | Locked transition |
| --- | --- |
| `Sender::try_send` | Reject if receiver closed or queue full; otherwise append the captured envelope. Empty-to-nonempty admission sets `P=now`. Take `W` for a later wake. |
| `Receiver::open` | Install a fresh `A`, take the preceding `W`, and give existing backlog a fresh connection-local progress budget. |
| `Body::poll_next` | After cooperative admission, require `R` and exact token equality with `A` before inspecting `Q` or changing `W`. Inspect at most 128 entries and retain only exact route matches. |
| `invalidate(token)` | If the supplied token still owns `A`, clear `A` and take `W`; otherwise change nothing. Both `Session` and `Body` Drop use this operation. |
| `Receiver::drop` | Close `R`, clear `A`, `W` and `P`, and remove all queued envelopes. Retained bodies cannot keep admission open. |
| `Sender::drop` | Close `S` and take `W`. The active body can drain remaining traffic and then report EOF. |
| `Receiver::discard_outage` | Remove the bounded queue and clear `P` after a failed connection attempt. |

Explicit wake calls happen after unlocking. Removing an entire discarded queue
also releases its envelope buffers outside the mutex. Polling still performs
bounded buffer allocation/deallocation and Waker clone/replacement while locked.
The argument uses the pinned Tokio/tonic waker implementation and ordinary
nonpanicking allocator behavior, not arbitrary reentrant RawWaker vtables.

## Safety argument

The initial state has an empty queue, open owners, no active body and no waker
or backlog timestamp. The following invariants hold by induction over the
locked transitions:

1. `0 <= len(Q) <= 4096`. Only successful admission increases the length, and
   it checks capacity under the same lock. Polling and discard only remove
   entries. Receiver closure removes the entire queue and rejects later sends.
2. A stored `W` belongs to the current `A`. Only a poll that has just checked
   exact ownership may install it, in the same critical section. Opening a
   replacement, invalidation and closure remove the preceding waker. Sending
   can take a waker but cannot replace it with another body's waker.
3. After invalidation of token `t`, a body with `t` cannot inspect or remove a
   queued envelope, install a receiver waker, or clear a replacement token.
   The ownership check and those effects share the lock, so a concurrent poll
   is wholly ordered before invalidation or is rejected afterward. A stale
   Drop takes the no-op branch. Fresh allocation identity prevents A/B/A reuse.
4. Every returned envelope has the body's captured route identity. The only
   path into the output batch uses `Arc::ptr_eq`; address equality is
   insufficient. Entries are popped from the front once, preserving the order
   of delivered entries from the inspected prefix. Stale entries may be dropped.
5. `P` is absent exactly when `Q` is empty. Admission to an empty queue arms it;
   removing the last entry or discarding the queue clears it. With nonempty
   backlog, only opening a new RPC or dequeuing a nonempty valid batch resets
   its age. Repeated sends and stale-only inspection do not constitute progress.

These invariants cover queue access after the invalidation point. A batch
already dequeued before that point may have reached tonic/HTTP2 and may still
arrive after cancellation. No instantaneous retraction of emitted bytes is
claimed. Conversely, an old route body may discard newly queued foreign-route
traffic before its owner observes the route watch. Loss on either boundary is
permitted by the existing best-effort Raft transport contract; Raft retransmits.
Root, sender, destination, store and receive-authority checks remain in force
at the receiver. This queue cannot create a quorum or application certificate.

`Session` is constructed before tonic receives the body. The peer owner retains
that guard across its RPC selection and drops it on completion, route-change
cancellation or task abort. A replacement session is created only after the
previous owner future is dropped. Thus a retained HTTP2 body is fenced even if
it was never polled. Dropping the receiver is an additional independent closure
boundary, including abort of a worker before its first poll.

## Bounded work and conditional progress

Each polling iteration removes one entry and decreases the remaining inspection
budget. At most 128 entries are inspected, including a leading stale prefix.
The byte check occurs before the next entry, retaining the previous soft 1-MiB
target: one legal envelope can cross it, and a first envelope may exceed it.
The count bound is unconditional on payload size; byte accounting assumes
representable Rust sizes. This is not a hard 1-MiB batch limit, queue-byte bound,
or total tonic/HTTP2 memory bound.

If the inspection budget ends on stale-only traffic with queued work remaining,
the body registers its current waker and self-wakes after unlocking. It therefore
does not require a new producer event to reach a later valid envelope. If the
queue is empty, waker registration and the empty check share the mutex with
admission: a producer either precedes the check or takes the registered waker.
An available valid message is returned without waiting for a larger batch.

The removed Tokio receiver previously charged cooperative task budget. The new
body explicitly calls `tokio::task::coop::poll_proceed` before locking and marks
progress for inspected envelopes or terminal readiness. An empty pending poll
restores the charge. Exhaustion yields before queue/waker access, bounding
consecutive ready batches under Tokio's cooperative execution contract. An
invalidated body can first return Pending on budget exhaustion and terminate
on a subsequent poll; it cannot acquire queue ownership during that yield.

The stalled-stream watchdog is a separate future polled by the peer owner,
not a timer that depends on HTTP2 polling the request body. The actual call
path has at most one live `Session::stalled` waiter per queue. It enables its
Notify interest before reading the predicate: an earlier transition leaves a
permit, and a later transition wakes the registered owner. Notifications are
hints; the locked timestamp and token determine the outcome.

An already-expired backlog returns before `select`, so continuously ready
notifications cannot postpone the expiry decision. A timer wake checks the
current timestamp again, so an old armed deadline cannot expire work whose
valid dequeue has reset the budget. No explicit notification is needed for a
dequeue that moves the deadline later or empties the queue; an earlier wake
merely rechecks and rearms or becomes idle. New work on an empty queue always
notifies and receives a new budget. Connection establishment/backoff has its
own timeout; a newly opened RPC starts its own three-second progress interval.

Progress here means valid dequeue into a body batch, not remote receipt,
application or quorum acknowledgement. An empty queue disables this watchdog
even if HTTP2 retains an earlier frame. Keepalive and subsequent traffic retain
their separate recovery roles. Eventual recovery requires fair owner/I/O
scheduling, finite local operations, a stable authorized route, eventual
connectivity and retransmission/admission. Mutex contention and scheduler
starvation have no proven wall-clock bound; three seconds is a policy budget,
not a real-time theorem.

## Proof boundary and validation

This is a source-mapped deductive implementation argument. It is not a new
machine-checked proof of Tokio, HTTP2, Rust memory ordering or whole-system Raft
refinement. The existing [route ownership](RAFT-ROUTING.md) and
[outbound batching](RAFT-COMPLETION-PROOF.md) proofs keep their documented
assumptions. The extra per-RPC token, waker and watchdog argument above is not
silently attributed to those older model inventories. Core consensus and
durability proof composition remains a separate open requirement.

The 16 direct-body component tests cover same-route and A/B/A retained-body
ownership, waker/Drop fencing, controlled invalidation cleanup, abort before
first poll, receiver and sender closure, FIFO/count/soft-byte rules, bounded
stale continuation, idle and unpolled watchdog behavior, old timer revalidation,
non-progress notifications and cooperative yielding. Existing real gRPC tests
retain distinct-address migration, frozen-reader recovery, established
blackhole recovery and connection-budget coverage.

`scripts/check-route-controls.py` retains the original five route/TCP source
controls and adds four direct-body controls. Each requires the original test
to pass, an isolated semantic fault to fail its named assertion, and restored
source to pass. Zero selected tests, compilation failure, abnormal termination
or a different assertion cannot count as an accepted fault.

The first new source-control run is retained at
`/tmp/kv9-direct-peer-body-route-controls-first`. Its watchdog mutant reached
the expected comparison, but the test borrowed a MutexGuard inside `assert_eq!`.
The failing assertion poisoned the queue and cleanup panicked again, producing
SIGABRT instead of the required ordinary test failure. The gate rejected that
run. Copying the timestamp before the unchanged comparison fixes the test's
failure path; the rejection predicate was not weakened. Pre-edit files and the
separate early-expiry hardening are recorded in
`/tmp/kv9-direct-peer-body-review-correction`.

The final candidate passes all **224 kv9-raft tests** (193 library, 19
integration and 12 documentation tests) and Clippy with warnings denied.
The final source-control recording at
`/tmp/kv9-direct-peer-body-route-controls-bound-tests` passes all nine triples
(**27 compiled executions**), explicitly hashing the split-out body tests as
well as the production files. The corrected earlier recording is also retained;
the final run strengthens the source binding without changing production code
or test predicates. The validation result and log hashes are retained in
`/tmp/kv9-direct-peer-body-review-correction/result.json`.

Process E2E, matched c1/c64 throughput and latency, and actual Chaos Mesh
acceptance are separate gates. A focused test pass or this argument does not
select a performance version. All routine checks run locally; this candidate
does not change the manual-only GitHub workflow policy.
