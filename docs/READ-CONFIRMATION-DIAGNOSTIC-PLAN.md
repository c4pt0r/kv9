# Next diagnostic: locate read-confirmation waiting

Use the selected CRC runtime `ca0002c7`, not the held metadata or receipt
experiments. The existing CRC lifecycle recording already identifies the
submission-to-confirmation interval; repeating its four coarse stages would
not answer the next question. Determine which local queues and owner work
account for that interval before changing their scheduling.

## Source boundaries

1. In `crates/raft/src/grpc.rs`, sample the residence of an `OutboundMessage`
   from successful per-peer enqueue to selection by `receive_for_destination`
   or `coalesce_queued`. Account for stale-destination drops separately. Keep
   route-generation identity, queue overflow and count/byte budgets unchanged.
2. Measure the existing bounded batch-channel send wait in `peer_session`.
   Distinguish successful queue admission, stream resolution, closed sender and
   progress-budget expiration. Queue admission is not a wire flush or peer ACK.
   Preserve all reconnect, route-cancellation and progress-budget behavior.
3. In `crates/raft/src/work.rs`, sample admitted-message residence in
   `RaftInbox` until its bounded drain. Preserve admission weight, FIFO order,
   byte/count bounds, retained-prefix handling and producer/owner notifications.
   Clock reads and metric updates must not introduce new nested locks.
4. Use the driver's existing pump/Ready and scoped read-lifecycle boundaries
   to relate owner processing to observed confirmation. Keep full successful
   pump, apply-position and view fences. Do not report the coarse confirmation
   interval as network RTT or claim that disjoint message samples describe one
   request. Add finer owner observations only where those boundaries are missing.

## Measurement and correctness contract

Instrumentation must be bounded and diagnostic only. Select a fixed sampling
rule before recording, use local monotonic timestamps, retain per-node/message
kind/outcome counts and explicit sampled denominators, and avoid an unbounded
context registry. No timestamp needs to cross the wire. A sampled message may
be dropped or canceled; retain that outcome without fabricating a completion.

Document an erasure mapping from instrumented source to the selected source:
excluding added observations, all queue, route, notification, message, deadline
and Raft state transitions remain the same. Focused tests must exercise sampled
success and drop/refusal paths, queue accounting and unchanged cancellation.
Existing source/proof gates remain applicable; observation counters alone do
not prove implementation refinement.

Collect c1 GET and c64 50% mixed traffic with the existing fixed v3 dataset,
client and CPU placement. Establish fresh drained endpoints and stable process
identities, retain complete outcome populations and exact binary/source pins,
and keep builds, faults and uninstrumented timing separate. Instrumented rates
are diagnostic accounting, not new performance claims.

Each reported row must state its population and timestamp boundaries. Unless
observations are explicitly joined to one exact read context, do not add sender,
receiver, owner and notification averages to reconstruct request latency.
Preserve unknown intervals. If the observations cannot distinguish the proposed
causes, report that limit before selecting a scheduling or transport change.

This plan is unexecuted. The next code change is the bounded diagnostic itself;
no new runtime optimization or DPDK benefit is established by this document.
