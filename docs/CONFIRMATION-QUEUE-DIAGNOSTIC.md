# Local confirmation-queue diagnostic

This branch instruments the selected CRC runtime at `9694df2` (whose selected
runtime is `ca0002c7`). It is a diagnostic, not a selected performance change.
Its rates include observer overhead. No new consensus algorithm, wire field,
queue, batching delay, scheduling policy or configuration is introduced.

## Observation contract

`metrics.json` retains schema 2 and its original 26 metrics. The additional
`raft_transport_wait` array has exactly 15 ordered rows on a gRPC transport:

| Stage | Kinds | Population and completed duration |
| --- | --- | --- |
| `sender_queue_residence` | Seven message kinds below | Every serialized, unmasked outbound enqueue attempt. For admission, start immediately before the original `try_send`, after taking the peer lock; stop after removal and destination classification. Successful timing includes the insertion operation and excludes peer-lock acquisition. |
| `batch_channel_admission` | `batch` | Every existing batch-channel send select reached after coalescing. Start before that select and stop in its selected arm. This measures local channel admission, not wire flush, stream consumption, remote delivery or a quorum ACK. |
| `receiver_inbox_residence` | Seven message kinds below | Every inbox admission attempt after computing encoded weight. For admission, start immediately before insertion under the inbox lock; stop immediately after removal in the bounded drain. Timing includes insertion and excludes producer lock acquisition. |

The seven kinds, in order, are `heartbeat`, `heartbeat_response`, `append`,
`append_response`, `read_index`, `read_index_response`, and `other`. Batch
coalescing can contain several kinds, so a batch has no fabricated message kind.
Each node owns its counters. Endpoint status supplies node/process identity;
no message is assigned a leader identity or read-context identity by this probe.
Non-gRPC/custom transports may return an empty extension; they are not eligible
for this diagnostic recording's exact 15-row inventory.

Each row contains `stage`, `kind`, `sample_every` (64), `attempts`,
`selected_from_attempts`, `abandoned_unknown_duration`, `counter_exhausted`, and
`latency` (the existing `LatencySnapshot`). Attempts 1, 65, 129, ... are selected
independently per stage/kind. `selected_from_attempts = ceil(attempts / 64)` is
the selection schedule implied by that counter load, not completed samples.
An exhausted counter stops further sampling; it never wraps or rejects work.
Observation memory is bounded by the existing queue capacities and drain prefix
plus a fixed number of histogram cells. Queued records carry at most one sample.

Histograms use local `Instant` durations in nanoseconds and the original 65
exponential buckets and seven outcomes. Reset is component construction. Each
histogram snapshot is coherent on its own. Counters, different rows and separate
histograms are read sequentially. No global atomic conservation equality is
promised. Reject exhausted, invalid or saturated observations for analysis.

| Boundary | success | rejected | replaced | error | unconfirmed |
| --- | --- | --- | --- | --- | --- |
| Sender queue | Removed for the same destination allocation | Unknown route, full or closed queue | Removed for a stale destination allocation | Discarded in the existing bounded connect-failure drain | Unused |
| Batch admission | Local send admitted | Local sender closed | Unused | RPC resolution wins the select, regardless of its RPC result | Progress budget wins the select |
| Receiver inbox | Removed by bounded drain | Original count/byte admission limit refused | Unused | Unused | Unused |

Sender unknown-route and receiver refusal durations start at selection before
the lock and end after refusal; they are not residence times. Sender full/closed
queue durations start before `try_send`. The common `aborted` and `released`
histogram outcomes remain unused. Selected work dropped during cancellation,
queue teardown, or unwinding increments `abandoned_unknown_duration` exactly
once, without recording a duration. Drop reads no clock and takes no observer
lock. A batch canceled by a route change therefore cannot look like a successful
send. Remaining in-flight work has no completed duration.

Inbox dequeue clocks are stopped inside the original queue lock, but records
are committed after releasing it and issuing the existing retained-work
notification. This preserves dequeue timestamps while avoiding a new lock
nesting. Outbound admission refusal likewise releases the peer lock before
recording. Observer locks never call into queue, route, driver or Raft code.

These rows include background heartbeats and all phases between metric
endpoints. Public/read/apply drain is not a transport quiescence fence. Without
joining exact read contexts, do not sum stage means into request latency,
subtract endpoint quantiles, infer network RTT, or correlate timestamps across
processes. Delta quantiles must be computed from delta buckets. Keep each voter,
message kind, outcome and completed-sample denominator visible.

## Erasure argument and scope

Define projection E to remove the new module, metric fields/calls, sample
ownership and test-only observations. Project `(message, weight, sample)` to
`(message, weight)` and `OutboundMessage` to its original destination/envelope.
All observer-only instructions are stuttering steps under E. The following
correspondence is the review obligation for this diagnostic:

1. `send` retains partition checks, serialization, envelope bytes and the same
   enqueue order. `enqueue` holds the original route lock through the same
   `try_send`, retaining one worker, the same immutable destination allocation
   and a 4,096-message queue. Inspecting a returned send error adds only a metric;
   the original operation also discarded that returned message.
2. `receive_for_destination` and `coalesce_queued` retain pointer-identity
   filtering, FIFO order, inspected-message count, byte targets and the same
   cooperative yield. Observer outcomes cannot return, admit, select or reroute
   a message. The connect-error `match` erases to the previous bounded discard
   loop: consume a queued message, or break on the same `try_recv` error.
3. `peer_worker` retains its biased route-change select and owned cancellation.
   `peer_session` retains the exact endpoint, reconnect backoff, channel capacity
   16, original futures/select arms and 3-second progress budget. Observations
   inside selected arms do not change their return/break decisions. Drop only
   changes counters. No observation is a new admission, retry or ACK authority.
4. Inbox encoded weights, admission conditions, FIFO prefix, byte/count drain
   bounds and all producer/owner notification sites retain their original
   effects. E removes stopped samples and the bounded deferred-record list.
5. The driver adds only a transport snapshot accessor. ReadIndex, sealed read
   groups, full successful pump, commit/apply/view fences, writes and recovery
   remain unchanged. Export still runs outside its state lock and returns no
   business error; its original size limit and publication protocol remain.

This is a source-level safety refinement argument under the existing model of
asynchronous scheduling and best-effort message loss. Observer CPU time, clock
reads, atomics, allocation and lock contention can alter timing and select
schedules; no timing/performance equivalence or machine-checked proof of Rust
is claimed. Resource exhaustion and poisoned observers retain the common
metrics module's existing failure assumptions. Existing core proofs apply to
the unchanged algorithm; this document does not replace them or establish
distributed fault qualification. Focused tests must retain queue bounds,
generation filtering, cancellation and the sampled completion/drop contracts
before recording. Actual Chaos Mesh qualification remains a separate gate for
promoting a runtime optimization.
