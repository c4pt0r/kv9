# Direct peer request-body queue: conditional design review

This is a sketch review, not implementation approval, a proof run, or a performance result. Replacing the intermediate 16-batch channel is feasible, but sharing the original receiver introduces ownership and liveness obligations absent from the borrowed single-owner loop. The existing transport is best effort; removing a buffer changes batching/backpressure/drop timing and does not preserve the exact delivery history.

## Required ownership boundary

Use a fresh immutable token for every RPC attempt, distinct from the existing PeerDestination allocation. Reconnecting to the same route must not let the old RPC body share the new attempt's receive authority. Under one short receiver mutex, check that token, then perform bounded poll_recv/try_recv/coalescing. Invalidation and installing the replacement token must use that same mutex. An atomic check before taking the lock is insufficient: an old body could pass the check, lose the race to replacement, then discard the replacement's envelopes as foreign-route traffic or receive same-route traffic for its old connection.

Tokio 1.53.1 Receiver::poll_recv documents that only the most recently registered waker is notified. A stale body must therefore return terminal without calling poll_recv at all. A watchdog must not poll the same receiver to observe availability. Old-body Drop must conditionally clear only its own token and must never close/drop the shared receiver or invalidate a newer token. Construct the owning session guard before handing the body to tonic; cancellation before first poll, connection failure, early RPC return and peer-worker abort must all invalidate it. Do not assume dropping the RPC future synchronously destroys tonic's request body.

A dequeue selected under the lock before invalidation may already be in tonic/HTTP2 and cannot be recalled. State the boundary as no old-session queue access after invalidation, with existing per-envelope route filtering and unchanged receive-side authority checks. Do not claim instantaneous retraction of already emitted frames. Keep locks out of socket I/O/awaits and preserve the peer-registration/enqueue lock ordering.

## Independent progress supervision

A timer inside poll_next cannot fire when flow control stops polling the body. The peer owner must independently select RPC completion, route revocation and the progress watchdog. Arm/reset the watchdog from actual pending work and valid current-session dequeue progress; polling an empty body, inspecting stale envelopes or repeated enqueue notifications is not delivery progress. Idle streams must not reconnect every three seconds merely because no messages exist. A lost notification between checking state and registering interest must not leave the watchdog unarmed.

Keep bounded synchronous work and cooperative yield behavior. Every inspected stale envelope counts toward the turn bound; returning Pending after reaching that bound must arrange a wake because buffered messages might otherwise never trigger another producer notification. The first valid message must flush without waiting to fill a batch. Retain 128-message coalescing, the existing soft 1-MiB target (one legal envelope may cross it), root digest/auth metadata, the 4096-envelope queue bound, connect budget, keepalive, bounded outage draining and reconnect backoff. These are application-level bounds; this sketch does not establish a whole-stack memory or scheduler latency bound.

## Focused regressions

1. Retain an old body after invalidation, including same-address reconnect and A→B→A. Poll it after a new body has registered an empty-receiver waker. Enqueue exactly one new marker; prove only the new body wakes/receives, queue length is unchanged by stale polls, and dropping the old body leaves the replacement active. Exercise invalidation/dequeue lock ordering with controlled synchronization.
2. Establish a request body that is no longer polled while queued work exists and RPC remains pending. Prove the owner's watchdog expires independently. Also prove an idle healthy stream does not churn and resumes on its first message. Do not let a server abort alone satisfy the watchdog assertion.
3. Feed more than one turn of stale envelopes before a valid marker: assert bounded inspections, arranged continuation, prompt cancellation, FIFO within the active generation, immediate first-message flush and existing count/byte bounds.
4. Abort the peer worker before the body is first polled and while it is Pending; retain the body artificially. Prove it cannot continue draining after abort, has no unbounded/detached task ownership, and replacement ownership remains usable. Cover early RPC success/error and sender exhaustion without an empty reconnect spin.

Existing tests to retain: route_updates_keep_one_owned_worker_and_same_address_is_idempotent; route_generation_filter_rejects_old_queue_entries_even_after_address_reuse; both queued_batch_* tests; dropping_transport_terminates_a_retrying_worker; registered_address_change_replaces_live_peer_stream; certified_route_generation_never_regresses_or_reinterprets_an_address; peer_worker_escapes_a_frozen_reader_and_reaches_replacement; route_change_escapes_a_frozen_reader_at_a_distinct_address; connect_budget_escapes_a_backlog_flooded_endpoint; peer_worker_escapes_established_blackhole_and_reaches_replacement. Keep existing root-digest, token, destination/sender identity and receive-authority tests unchanged. Existing borrowed-receiver tests do not establish stale-body/waker safety.

No core Raft state transition is intentionally changed. Conditional safety reasoning must still account for arbitrary permitted transport loss/reordering and unchanged authorization checks; it does not prove the new ownership/watchdog implementation or liveness. The c1 lifecycle results motivate measuring a hypothesis, not attribution of confirmation time to this channel hop.

Reviewed source hashes:

- /tmp/kv9-rpc-worker-pair/crates/raft/src/grpc.rs: `9083374c5ac34abac7ed8eb5967b1c5fdb693b397e6f6570131e9691e422ac25`
- /home/dongxu/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/tokio-1.53.1/src/sync/mpsc/bounded.rs: `34957f69bed085770fed49f9c57c78d2f6739ac39b9ba6186c2134038ca97d79`
