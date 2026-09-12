# Bounded observation of the Raft request-body handoff

This diagnostic branches from selected CRC behavior. It does not include the
held Append-payload, vector-reuse, receipt or scheduling candidates. Its purpose
is to resolve an interval not covered by the earlier confirmation-queue study:
batch offer to the existing request-body receiver producing that batch.

## Clock and population contract

`batch_offer_to_body_poll` starts in `BodyProfile::offer`, immediately before the
existing `batch_tx.send` select. It stops when the original `ReceiverStream`
returns `Ready(Some(batch))`, before recording observer counters/histograms and
passing the unchanged batch to tonic. It includes admission waiting. It is not
pure queue residence, HTTP/2 flushing, TCP delivery or a replica acknowledgement.
The observer's own recording may delay subsequent work; these measurements are
diagnostic observations, not an uninstrumented performance comparison.

Every offered batch belongs to one of eight fixed classes: heartbeat,
heartbeat_response, append, append_response, read_index, read_index_response,
other, or mixed. Message kinds are captured from the original typed Raft message;
no protobuf payload is reparsed. Coalescing counts only envelopes admitted to
that exact route generation. A homogeneous batch belongs to its message kind;
a batch containing more than one kind belongs to mixed.

Each class selects offers 1, 65, 129, ... independently. Monotonic clocks are read
only for selected samples. Every offered batch contributes its seven message
counts; selected yielded and abandoned batches have separate composition totals.
A selected batch yielded by the receiver records `Outcome::Success`, meaning
this local boundary only. Every other latency outcome remains present and empty.
A dropped selected sample increments `abandoned_unknown_duration` and its
composition counts, with no clock read or histogram duration. This includes
pending-send cancellation, receiver teardown and a closed-channel send error
being dropped. Unsampled batch drops do not invent selected samples.

Fixed session-event counters identify the original selected branches:
route_changed, route_watch_closed, connect_failed_or_timed_out,
outbound_queue_closed, batch_channel_closed, rpc_resolved_before_batch,
rpc_resolved_during_offer and progress_budget_expired. RPC resolution is not
assumed to be a transport error or an acknowledgement. Session counters and
abandoned samples are different populations; their totals cannot assign a
particular cause to each dropped batch. Task abort may have no selected event
branch, but owned samples are still counted on drop.

Counters use checked atomic updates. Exhaustion leaves the prior value and
sets an invalidity flag; it does not wrap or stop database work. Counter loads
and individually coherent histograms are sequential observations, not one
atomic conservation snapshot. Do not equate deltas of rounded cumulative
selection counts to a fresh per-interval sampling sequence. Use start/end
cadence offsets and preserve any in-flight endpoint difference.

The extension is `metrics.json.raft_request_body_handoff`: null for transports
without this observer, otherwise version 1, the stage, cadence, seven message
kind names, eight class rows, eight session event rows and event exhaustion.
Each class has offered_batches, selected_from_offers, offered_message_counts,
yielded_sample_message_counts, abandoned_sample_message_counts,
abandoned_unknown_duration, counter_exhausted and the existing seven-outcome,
65-bucket latency snapshot. Original 26 metrics, schema 2 and the 512-KiB export cap
remain unchanged. The export stress test preserves all original 26 worst-case
outcomes plus all reachable extension outcomes and maximum-width counters.

## Projection to the selected implementation

Erase the added message-kind field, fixed composition arrays, samples, event
updates, snapshot getter and export extension. Project an `ObservedBatch` to its
original protobuf batch. The outbound 4096 and batch 16 channel capacities, FIFO
operations, route identity comparisons, stale-work inspection budget, 128-message
and 1-MiB coalescing targets, and legal oversized-message behavior are unchanged.
Composition updates occur only where the existing loop includes an envelope.
The first accepted envelope contributes one count; induction over the original
bounded loop gives the exact composition of the unchanged ordered batch.

`ObservedStream::poll_next` polls the original receiver exactly once with the
same context, returns Pending and EOF unchanged, and returns the identical
batch on Ready. It creates no task, timer, custom waker or delivery decision.
Its sample owns only fixed observer state and composition, never a batch, route,
queue, RPC or driver. Dropping the receiver drops the same queued payloads; drop
observation takes no lock and reads no clock. Observer locks never call database
code. The original send future, select arms, cancellation points, connect budget,
keepalive, stream-progress watchdog and reconnect backoff remain intact.

Observation adds fixed data per existing queued message/batch and fixed metrics
per transport; it introduces no growing sample history or destination map. Total
transport memory still depends on the existing peer count and payload limits.
No claim of identical timing or process RSS follows from projection. This is a
source-level observation-erasure argument, not a whole-Rust machine proof.
No ReadIndex authority, persistence, Ready processing, pump/apply/view fence,
deadline or write acknowledgement changes.

## Acceptance before interpretation

Run focused cadence, composition, Pending/Ready/EOF, wakeup, full-channel,
cancellation, closed-channel ownership and exhaustion regressions. Preserve the
existing route-generation, coalescing bounds, endpoint failure and recovery tests.
Run Raft/server checks, explicit testing-feature partition checks, formatting
and Clippy using the root-owned retained-build lock. Bind a clean retained release
and check ordinary leader-loss/original-directory restart histories before the
two fixed c1 GET/c64 mixed diagnostic cells. Keep all outcomes, endpoint identities
and original timing boundaries. No actual Chaos or new performance selection is
established merely by adding this observer.

Read the complete interval by node/role and batch class. Do not add means from
different populations into GET latency. If body polling accounts for meaningful
waiting, design a separate uninstrumented handoff reduction that preserves the
stalled-reader watchdog and route cancellation, then compare complete GET/mixed
cohorts. If it is small, identify the next specific owner/HTTP2/socket boundary.
Full proof composition, actual Chaos and host-failure gates remain open. Hosted
CI stays manual; the selected runtime remains CRC.
