# Bounded Append entry payload experiment

Selected CRC leaves `max_size_per_msg` at the pinned raft-rs 0.7.0 default of
zero. This means at most one log entry per Append message. Set the encoded-entry
payload target to 64 KiB so an already available suffix can share a message.
The upstream API contract is documented at
[Config::max_size_per_msg](https://docs.rs/raft/0.7.0/raft/struct.Config.html#structfield.max_size_per_msg).
There is no timer, new queue, lease, eager acknowledgement or delay waiting for
future entries. `batch_append` remains false; that separate upstream option
mutates already queued messages and requires its own size-bound review.

## Hypothesis and acceptance

The preceding local transport diagnostic records mixed-load leader sender
heartbeat residence around 31 us and receiver heartbeat-response residence
around 21 us, while batch-channel admission is below 1 us on average. These are
different message populations, not an additive request-latency partition. The
configuration experiment tests whether fewer replication messages relieve
shared transport/owner work. It does not promise an isolated GET improvement.

Compare the uninstrumented candidate to selected CRC with fixed v3 c1/c64 GET
and 50% mixed workloads. Retain each repetition, GET-only latency and all
outcomes. Keep the candidate separate until applicable source/proof mapping,
ordinary recovery and actual Chaos Mesh gates pass. A short screen can reject
this hypothesis; it cannot establish production readiness or Redis parity.

## Safety mapping

Raft AppendEntries already carries an ordered contiguous suffix. The target
changes only the length chosen by the existing log-slice operation:

1. Every emitted entry still names the same index, term and command bytes from
   the leader log. The preceding index/term and receiver's prefix check remain
   unchanged. Partitioning a suffix into longer contiguous slices cannot
   introduce a log entry absent from that suffix or reorder two entries.
2. Followers persist the original Ready entries and HardState before transport
   publication. Match progress still comes from the original follower response;
   commitment still requires a majority and the original current-term rule.
   No in-flight byte or message is treated as a committed acknowledgement.
3. The same Ready/LightReady persistence and complete pump, apply, exact receipt
   and read-view fences remain. Safe ReadIndex, quorum checking, membership,
   failure poisoning, deadlines and cancellation are byte-identical to selected
   CRC. The 256-message upstream in-flight bound is unchanged.
4. Upstream log slicing always permits the first available entry so a legal
   entry larger than 64 KiB can progress. Thus 64 KiB is an entry-payload target,
   not a hard encoded RPC cap. Existing transport byte/count/admission bounds
   remain in force, and envelope/protobuf headers are additional bytes.

This maps the change to Raft's existing arbitrary-length AppendEntries rule;
it is not a new whole-implementation proof. Rigorous composition and fault gates
remain open as documented by the project. The focused lag/rejoin test observes
actual adapter messages, verifies contiguous indexes and the payload target
with its single-entry exception, and checks all replicas' final values,
including an oversized entry. Existing overwrite, minority read and persistence
failure tests remain necessary. No performance or Chaos result is asserted by
this source-only experiment document.
