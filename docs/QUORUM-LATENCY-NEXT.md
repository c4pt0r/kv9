# Next isolated-read latency investigation

The full ThinLTO point/batch comparison and [main integration](RELEASE-THIN-LTO-MAIN-INTEGRATION.md)
now pass. Begin from selected `11113f6`, whose server bytes reproduce the qualified
candidate. The [fixed-rate write-tail diagnosis](BATCH-WRITE-FIXED-RATE-RESULTS.md)
is also complete; its tail and arrival-accounting limitations remain open.
The opt-in [quorum-message trace](QUORUM-MESSAGE-TRACE.md) and its
[actual capture/overhead comparison](QUORUM-TRACE-RESULTS.md) now pass after
replacing the original contended recorder with immutable slots. Preserve both
original campaigns. The next implementation tests bounded owner polling against
observed local inbox residence, with the original mutex predicate and deadlines,
explicit CPU accounting and no assumed gain. Track this under #20 and #9.

The immediate objective is to locate avoidable work inside a fresh Safe
ReadIndex round. Keep quorum confirmation, sealed membership, successful
whole-pump completion, apply/view fences and durable write acknowledgements.

## Evidence to reuse

The [CRC read-stage diagnostic](READ-STAGE-RESULTS.md) observed a 20.636-us c1
mean between ReadIndex submission and exact quorum confirmation. That interval
includes local processing, transport, remote work and the returning response;
it does not identify network latency alone. Its population includes setup and
verification, so it cannot be subtracted from measurement-only client latency.

The earlier [request-body diagnostic](https://github.com/c4pt0r/kv9/blob/a75c005/docs/RAFT-BODY-HANDOFF-RESULTS.md)
already measured the 16-slot batch channel: c1 heartbeat/response means were
0.444–0.475 us. Its different population cannot be added to the read-stage
table. It does not support another channel replacement as the primary target.
The [experiment index](PERFORMANCE-EXPERIMENT-INDEX.md) also records rejected
direct-body, worker-count, watchdog and queue/event-frequency experiments.

## New observation boundary

Start from the candidate actually selected after full qualification. Use a
separate diagnostic feature and retain an uninstrumented control. Reuse existing
read contexts and message identities; do not add protocol fields or put
diagnostic state into a correctness decision.

| Interval | Boundaries and interpretation |
| --- | --- |
| Leader dispatch | Exact ReadIndex submission to admission of that context's heartbeat into the existing peer queue. Includes owner/Ready processing and serialization before enqueue. |
| Peer queue | Successful enqueue to dequeue for the same destination generation. Exclude dropped/stale envelopes from completed-duration statistics and retain their counts. |
| Leader-observed round trip | That leader's outbound heartbeat boundary to receipt of the matching follower response, before Raft inbox admission. Includes transport and remote work; retain each follower separately. |
| Follower processing | Receipt and validation of a context-bearing heartbeat through inbox residence, Raft step and response publication, all on that follower's local clock. |
| Leader completion | Matching response receipt through inbox residence, Raft step, exact quorum confirmation and the existing successful pump fence. A received response alone does not establish a completed read. |

Use per-process monotonic intervals. Correlate records by process lifetime,
region, term, context, peer and route generation. Never subtract timestamps
from different process clocks. Report each replica's local spans separately;
do not sum follower means or infer which response completed a quorum without
observing the actual confirmation boundary. Preserve group versus request
counts when several readers share a sealed context.

A context identifies a sealed read group, not an individual heartbeat emission.
The pinned raft-rs 0.7.0 `Raft::bcast_heartbeat` reuses
`read_only.last_pending_request_ctx()`. Periodic heartbeats can therefore repeat
the same context before confirmation. Record emission and response counts;
exclude ambiguous matches from per-message round-trip statistics and retain
their population. A separately labeled first-dispatch-to-first-response span
can describe a context's progress, but cannot identify which transmission
produced that response. Add a repeated-context control before recording.

Route generations are also local: `PeerDestination` uses an `Arc` allocation's
identity, while the protobuf envelope carries no equivalent connection token.
Retain that lifetime when correlating local enqueue/dequeue events; a raw
address alone can be reused after allocation release. Do not claim an exact
cross-process connection join from the current wire fields. Lost observations,
duplicate contexts or route replacement must yield explicitly unmatched or
ambiguous timing, without changing message delivery.

Bound diagnostic storage and sampling before execution. Retain offered,
selected, completed, dropped and unmatched populations, including cancellation,
route replacement and shutdown. A missing trace is unknown timing, not zero
latency. Observer overflow must not affect admission, retries or protocol
progress. Quantify instrumentation overhead against the uninstrumented control;
instrumented throughput cannot be advertised as a speedup.

## Source boundaries reviewed in candidate 02d0c01

Recheck these boundaries against the implementation selected after the broad
comparison. This map is source inspection, not new timing or instrumentation.

| Boundary | Existing location and observation constraint |
| --- | --- |
| Group submission | `AsyncReadQueue::submit` in `crates/raft/src/async_read.rs`; distinguish deferred admission from an admitted sealed group. |
| Leader dispatch | `NodeDriver::step_inner` in `crates/raft/src/driver.rs` calls `RaftPeer::pump` before `GrpcTransport::send`; the interval includes owner and Ready processing. |
| Peer queue | `GrpcTransport::enqueue`, `receive_for_destination` and `coalesce_queued` in `crates/raft/src/grpc.rs`; record failed admission and stale-generation discards as well as successful dequeue. |
| Inbound processing | `RaftGrpcService::batch_raft` validates and decodes before `RaftInbox::send`; `RaftInbox::drain` in `crates/raft/src/work.rs` precedes `RaftPeer::step_message`. Keep validation, inbox residence and Raft processing distinct. |
| Exact confirmation | `AsyncReadQueue::confirm`, called from `NodeDriver::step_inner` on exact `ReadState` contexts; an inbound response alone is insufficient. |
| Successful completion | `NodeDriver::step_observed` calls `AsyncReadQueue::complete` only after successful pump processing and completion publication, with the unified apply watermark. Keep this fence and later waiter resumption separate. |

## Execution and decision

1. Check the default and diagnostic feature builds, including the explicit
   `kv9-raft/read-stage-timing` Clippy configuration now confirmed during main
   integration (the older diagnostic record had a different command). Add
   focused controls for context/generation matching, bounded loss
   accounting and confirmation/fence ordering. Existing accepted tests keep
   their original source scope.
2. Freeze the source, build, clients, event schema and acceptance reader before
   recording. Begin with the existing c1 GET workload and its complete lifecycle
   accounting. Use a loaded contrast only to answer a specific unresolved
   question. Keep local recording isolated from compilation, archival and fault
   injection; retain the existing CPU and storage guards.
3. Identify a dominant measured interval before changing implementation. State
   which earlier rejected experiment is relevant and what new evidence would
   justify revisiting it. If the new observation does not identify a removable
   cost, publish that result and refine the unresolved boundary.
4. Evaluate a resulting optimization with fixed uninstrumented clients, both
   run orders, operation-specific means/tails and complete outcome accounting.
   Apply the appropriate proofs, recovery tests and actual Chaos Mesh gates
   before promotion. DPDK requires separate evidence of a relevant network-I/O
   bottleneck; a large loopback quorum interval alone is insufficient.

Redis read parity remains open. Dynamic multi-Raft and automatic range splits
follow that performance milestone under the existing roadmap; this diagnostic
plan completes none of those implementation or industrial acceptance items.
