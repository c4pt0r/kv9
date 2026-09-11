# Sampled asynchronous read lifecycle diagnostic on CRC

This diagnostic pair uses the current CRC runtime and the held read-credit-on-CRC
candidate. CRC starts from `86a689c8b13ae929a2abff2ef8c5794b50ebd639`, whose
production code is identical to `ca0002c7`. Credit starts from
`de37c71009e8199859b931e0037818f490e8d3f6`, plus the same retained-build
cache fix. The six executable instrumentation-file changes are ported unchanged
from historical diagnostic `d3dcea0355dd6c4ec23f0833708f6dc978412093` relative
to its `5ee897a` parent. The runtime scheduler and external streaming RPC stay
at their respective current parents.

These are diagnostic branches, not performance candidates. Preserve the
uninstrumented parents as the sources of throughput/latency acceptance. The
read-credit candidate remains held because its fresh mixed-read mean and p99
regressions outweigh its pure-read throughput benefit. The proposed four-cell
observation uses CRC/credit at c1 pure point GET and c64 GET/PUT mixed traffic,
with one fixed v3 client, no CPU profiling and no throughput acceptance.
No measurement is claimed by this source document.

A mixed hash of the process incarnation and checked request sequence samples
approximately one in 64 asynchronous registrations. It avoids selecting every
64th request, which could align with group boundaries. A sampled request and
its original ticket share one trace object; it retains only timestamps and
the node's metrics, with no strong reference to the registry or driver.
Unsampled requests carry an empty optional trace. No per-request data is exported.

The trace partitions one successfully received read into these elapsed stages:

| Metric suffix | Start | End | Included work |
|---|---|---|---|
| queue | Driver read start | Start of successful ReadIndex admission invocation | Context minting, registration, owner scheduling, deferred admissions |
| quorum | Successful invocation start | First exact-group confirmation observed | Admission callback, peer/pump work, transport, quorum progress and owner observation |
| apply | Confirmation observed | Immediately before successful-result send | Remaining pump work, applied-index coverage, completion selection and delivery preparation |
| notification | Before send | Successful receiver branch observed | Channel delivery and receiver scheduling |
| total | Driver read start | Successful receiver branch observed | Sum of the four stages above |

The five exported names use prefix `raft_async_read_profile_`. They describe
elapsed pipeline intervals, not isolated network latency or pure apply CPU.
Each member publishes an admission timestamp only when its sealed invocation
returns `Ok(true)`; deferred attempts leave no marker. The sample need not be
the group's representative. Duplicate or member-only confirmations cannot
overwrite the first exact-group marker. Confirmation is stamped separately for
each sampled member while walking the group, so it includes preceding member
notification/trace work. Send time is captured before the
channel can wake its receiver. A failed or abandoned send does not emit a
successful sample.

Relative timestamps use release/acquire atomics and a separate missing-value
encoding, preserving legitimate zero-duration stages. The receiver captures
its timestamp before recording histograms. Missing, unordered or overflowed
timestamps increment only the total metric's error population; they cannot
change the read result or admission state. Complete successful samples record
all five histograms. Existing outer read-establishment/backend timers also
include these histogram updates; their difference from profile_total must not
be interpreted as uninstrumented backend overhead. The profile has integer
sums satisfying queue + quorum + apply +
notification = total. Recording uses existing leaf observer locks outside
owner locks. It adds five fixed histograms to the diagnostic export inventory;
the existing 512-KiB bound and worst-case export test remain required.

Cross-metric exports are independently captured, so conservation is checked
only between fresh, quiescent endpoint snapshots. Counts and sums from the
same successful sample population can be compared; quantiles cannot be added.
Whole-cohort snapshots can include warmup and verification. They are not an
exact decomposition of the benchmark's separately bounded measurement interval.
Failure/cancellation traffic requires separate coverage and is not inferred
from these successful-read histograms.

Focused controls cover deferral with unchanged group membership, an unsampled representative,
exact first confirmation, apply coverage, send-before-receive, cancellation
before and after send, owner close, and arithmetic conservation. Existing Raft
tests continue to enforce the original ordering and ownership contracts.

## Execution ownership and interpretation

| Boundary | Owner and code | Authority retained |
| --- | --- | --- |
| Receive/dispatch | Streaming receiver validates the frame, reserves a response slot and spawns one bounded handler (`point_stream.rs`) | Original deadline, monotonic frame identity and request/response capacity |
| Read preparation | RPC handler holds public admission and awaits `prepare_raw_get` (`grpc.rs`, `runtime.rs`) | Read cancellation can release its reservation; blocking engine work retains it |
| Registration | `read_barrier_async` mints an incarnation/sequence context and queues its ticket | Checked uniqueness, bounded request count, original monotonic deadline |
| Submission | Dedicated Raft owner drains ingress, seals a queued prefix and invokes `read_index` (`driver.rs`, `async_read.rs`) | Exact group membership and current-term readiness; credit additionally checks actual raft-rs pending occupancy |
| Confirmation | Peer transport wakes the owner; its pump publishes exact ReadStates | Fresh Safe ReadIndex quorum, first exact-group confirmation |
| Apply/completion | A successful complete pump covers the confirmed index before sending the original ticket | Failed/partially applied pumps cannot return a successful read |
| Backend/reply | Woken RPC handler obtains the fenced read view and completes the read, then queues its response | Snapshot after barrier, serving/epoch/keyspace fences and bounded response ownership |

The confirmation stage includes admission callback/peer-lock time, transport,
quorum progress and owner observation. It is not a measurement of irreducible
network RTT. The queue stage includes all pre-admission work. On credit, a
false admission may mean either current-term unready or pending protocol credit;
these histograms cannot separate those reasons. The notification stage stops
at ticket receipt, before backend execution and response enqueue/encoding.
No request/group correlation or exact per-request tail vectors are exported.

The source preservation argument is a projection: remove optional trace state,
its sampling/timestamp/metric operations and the extra metric inventory, and
the existing read-state transitions, callbacks and completion branches remain
identical to the parent. Samples neither feed a guard nor supply an index,
identity, deadline, admission decision or read result. The trace retains no
registry/driver ownership cycle. This argument is conditional on successful
execution of the observer operations; added allocation and scheduling cost can
change timeout frequency. It is not a new proof of Raft or whole Rust execution.
Existing algorithm proofs retain their original scopes; these diagnostics do
not fill the open proof-composition or actual Chaos fault-test requirements.

## Local port validation

Both current-source ports pass formatting and Raft/server all-target Clippy
with warnings denied. The CRC port passes 214 Raft tests/doctests and four
observability tests; the credit port passes 221 and four respectively. These
counts include the three inherited instrumentation checks and all existing
package tests. First-party debug artifacts were explicitly invalidated before
each source's checks, and first-observed test build units compiled afresh.
The six executable diagnostic files were unchanged throughout validation.
These are focused source gates, not fresh full-workspace, process-recovery,
Chaos Mesh or performance acceptance for either diagnostic revision.
