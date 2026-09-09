# Raft route ownership and connection cancellation

Tracking: [#47](https://github.com/c4pt0r/kv9/issues/47). This extends the
[receive authority boundary](RAFT-RECEIVE-AUTHORITY.md). An endpoint is a routing
choice for an already authorized node/store binding. It cannot grant membership,
replace a store incarnation, or change the root certificate.

## Failure and implementation

The original gRPC transport changed its address map but retained an outbound
sender whose worker captured the old address. A real two-endpoint regression
kept both endpoints healthy: after the map reported B, A received 149 subsequent
messages and B received none. A disconnected worker also had no owned cancellation
handle. The TCP fallback retained an old stream across the same kind of update.

`GrpcTransport` now stores one `PeerRoute` per peer under a mutex. That entry
owns an immutable `PeerDestination` allocation and, once needed, one `PeerSender`.
Registration and enqueue use the same lock. Serialization happens before taking
it; socket I/O and async waits happen after releasing it. The queue sender never
escapes the entry. A same-address registration leaves the allocation and task
unchanged. A changed address installs a fresh allocation and replaces the latest
value in a watch channel; intermediate address updates may be coalesced.

Every queued envelope retains its admission-time destination allocation. A
connection session accepts an envelope only if `Arc::ptr_eq` matches its own
allocation. Returning from A to B to A therefore cannot accept old-A messages
into the new-A connection. Keeping the allocation alive prevents pointer reuse
while any queue/session reference can still name it; there is no wrapping
integer generation counter in Rust. Old queued messages may be dropped, as can
messages on an ordinary broken connection. Raft supplies retransmission.

One outer task selects between a destination update and the entire session
future. An update drops that future, including connect, backoff, RPC and pending
batch state, before constructing the next session. No additional task is spawned
for an address update. A finished task can be replaced on the next enqueue.
`PeerSender::drop` aborts its owned task; closing the watch channel also wakes the
outer selection. Failed-connect draining and discarded-generation processing
have finite per-poll work. Tokio must continue scheduling the task for cancellation
to finish; this is not synchronous cancellation of arbitrary external I/O.

The watch borrow is cloned and released before any await. This matters because
watch borrows hold a lock, and `changed` is cancellation-safe in a select;
see the [pinned Tokio watch contract](https://docs.rs/tokio/1.53.1/tokio/sync/watch/struct.Receiver.html).
A bounded queue retains at most `PEER_QUEUE` envelopes, a session builds at most
`MAX_BATCH_MSGS` envelopes, and there is one owned task and one current watch
value per peer. Route churn does not create an unbounded list of retired tasks.
These are ownership/count bounds; they do not newly establish byte bounds for
arbitrarily large individual Raft messages or a global peer admission policy.

The synchronous TCP fallback stores one shared route/stream lock per peer. It
serializes a write with changes to that peer's address, leaving other peers free
to progress. An address change drops the old stream. Connect has a 200 ms timeout;
frame writing shares one 200 ms absolute deadline across partial writes. These
bounds still depend on OS scheduling/socket behavior. Explicit shutdown and drop
close owned outbound streams and wake the listener; shutdown is idempotent.

## Linearization and safety proof

For one peer, registration linearizes at the locked replacement of its current
destination, and send linearizes at the locked `try_send` with that destination.
The lock gives a total order extending non-overlapping calls. Thus an enqueue
following a completed update carries the new allocation. Concurrent operations
may occur in either lock order. The receiver filter permits only messages whose
allocation equals the session allocation. Therefore no newly enqueued message
for B can be serialized into the old A session, even if notification races with
queue consumption. Processing an update drops any batch still owned by the old future.
A batch handed to the old network session before cancellation may still arrive
later. Such packets remain subject to the existing receiver/root/store authority
checks; endpoint changes do not retract previously transmitted packets.

The table never replaces a live worker on update. Enqueue creates one only when
none exists or its retained `JoinHandle` is terminal. By induction, there is at
most one live owned worker per peer. Per-peer proofs compose because different
entries do not share mutable destinations or queues. Shutdown ends future
admission and eventually destroys the owned futures under fair task scheduling.
The TCP per-peer lock gives the same send/update order directly around the write.

`proofs/tla/route/RouteOwnership.tla` models one authorized peer and an arbitrary
sampled envelope/batch, arbitrary natural-number generations, task ownership,
connect/backoff/blocked phases, route changes, drops, closure and termination.
The sampled-envelope projection proves generation binding for each envelope;
it does not mechanically prove FIFO or the Tokio channel implementation. Ghost
natural-number generations name successive live allocations in the Rust mapping.
The model's main `RTNext` is unbounded. Only `RouteMC` bounds update count for TLC;
the TLAPS induction has no generation bound.

The audited `RouteOwnershipProof` contains 17 declarations and 197 obligations:
initialization and induction preserve type/ownership/batch invariants; action
lemmas establish admission-time generation capture, emission binding and current
route delivery. Stable-route progress decomposes into task creation, observing
the route, connectivity, discarding stale queue entries, admission, batching and
emission. Closure has a separate termination proof. Positive proofs require
fresh caches, exact inventories, pinned SANY/TLAPS tools, no omitted proof or
unapproved module assumption, and clean exact obligation counts.

## Progress assumptions and validation

Delivery requires a stable authorized route, a healthy endpoint, fair task/I/O
scheduling, eventual queue admission and retransmission. The fair continuation
excludes indefinite route churn, drops, outages and shutdown. Fairness is stated
on each needed action; it is not inferred from a timeout or a final matching
state. Termination requires fair cancellation processing. No singleton discovery
service, route coordinator, root creator or extra database dependency is added.

The protocol gate runs two endpoint sizes under two TLC fingerprints, stable
delivery and termination continuations, and witnesses for address reuse and
actual delivery. Eight isolated protocol faults each have baseline, failing and
restored TLC/TLAPS evidence: reused generation, duplicate worker, missing route
observation fairness, a foreign queued generation, enqueue under an old session,
a batch retained across a route change, missing retransmission fairness and a
worker retained after close. Two semantic proof-audit controls and eight output
controls reject proof holes, a custom axiom and incomplete/fabricated results.

Real-socket tests retain healthy address migration, frozen-reader migration,
original same-address recovery, TCP migration and TCP listener destruction.
Additional tests cover idempotent updates, 5,000 updates with one task, queue
capacity, A/B/A generation isolation, and termination during connection retries.
Five isolated Rust source controls must fail their exact intended assertion and
pass again after restoration. Existing admission/incarnation checks remain
required. Commands:

```sh
cargo test --locked -p kv9-raft --lib
python3 scripts/check-route-controls.py --output /tmp/kv9-route-controls-new
python3 scripts/check-route-protocol.py --jar /path/to/tla2tools.jar \
  --tlapm /path/to/tlapm --output /tmp/kv9-route-protocol-new
```

Dedicated actual Chaos Mesh address migration of an authorized existing store,
with old/new endpoint observations, catch-up receipts and complete concurrent
histories, remains an explicit #47 acceptance requirement. The existing
19-window matrix exercises other fault/recovery boundaries; passing it does not
substitute for the new migration cell. Neither single-host Kind nor two local
socket endpoints establishes cross-host failure tolerance or throughput targets.
