# Next isolated-read latency investigation

Finish the frozen ThinLTO point/batch comparison before building another
candidate. This plan has no new profile, benchmark result or implementation.
Track the work under #20 and the checkpoint in #9.

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

Bound diagnostic storage and sampling before execution. Retain offered,
selected, completed, dropped and unmatched populations, including cancellation,
route replacement and shutdown. A missing trace is unknown timing, not zero
latency. Observer overflow must not affect admission, retries or protocol
progress. Quantify instrumentation overhead against the uninstrumented control;
instrumented throughput cannot be advertised as a speedup.

## Execution and decision

1. Check the default and diagnostic feature builds, including the explicit
   `kv9-raft/read-stage-timing` Clippy configuration missing from the historical
   record. Add focused controls for context/generation matching, bounded loss
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
