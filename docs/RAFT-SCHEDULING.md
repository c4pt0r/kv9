# Event-driven Raft scheduling

Tracking: #41, #13, #20 and #9. This candidate replaces timer-bound work
delivery; it does not change durable acknowledgements, ReadIndex confirmation,
application receipts or the Ready persistence/publication protocol.

## User-visible objective and current status

In-memory Raw KV should approach Redis-class latency and throughput where the
durability and replication contract permits. The current 20-ms pump sleep is an
implementation bottleneck, not an accepted performance baseline. Compare fixed
key/value sizes, logical concurrency, operation mixes, hardware placement,
throughput, latency distributions, CPU and failed/unknown outcomes. Label a
standalone non-persistent Redis run as a reference, not equivalent durability.

This first candidate removes the pump's unconditional sleep. The existing
2-ms outbound batching window and 1-ms completion polling remain unchanged so
their separate effects can be measured. Group commit remains #20 work. No
near-Redis result or production acceptance is claimed by this document.

## Notification and ownership contract

Each peer owns one `WorkSignal` for its process incarnation. Its mutex protects
two booleans: `pending` and `stopped`. Notifications coalesce into one pending
bit; they carry neither request data nor consensus authority.

1. A producer publishes a local proposal, configuration change, campaign or
   read request under the peer lock, releases it, then notifies. Transport
   reception similarly publishes to its inbox before notifying.
2. Before draining, the owner clears `pending` under the signal mutex. It then
   releases that mutex before acquiring any peer, inbox or application lock.
3. Work admitted during a drain either joins that drain or leaves `pending`
   set for another turn. A bounded inbox drain re-notifies if a suffix remains.
   A Ready successor exposed by application also causes another turn.
4. After a successful turn, the owner checks `pending` and `stopped` under the
   same mutex used by notification. `Condvar::wait_timeout` atomically releases
   that mutex and parks. Spurious wakeups repeat the predicate/deadline check.
   No predicate-to-park gap is permitted.
5. Stop or terminal failure sets the terminal signal and wakes a parked owner.
   Later notifications cannot restart it. Existing fatal checks still prevent
   Ready publication or application after an observed failure.

`NodeDriver::spawn` claims a one-way owner flag and returns a typed error on a
second spawn. A separate mutex serializes complete driver pump iterations,
including manual test iterations. The production runtime starts one background
owner and does not manually deliver additional ticks. `DrainToken` ownership,
peer persistence ordering and exact receipt semantics remain in force.

The signal mutex is a leaf lock. Notification acquires it only after publishing
and releasing the work lock. Waiting never holds a work lock. Installing a
transport signal also notifies, covering inbox messages admitted before driver
assembly. A fresh driver/transport lifetime is required on restart; addresses
and node IDs alone do not transfer a notification or owner capability.

## Time and work bounds

Tick deadlines use a monotonic clock independently of traffic. A turn before
the deadline delivers no tick; a due turn delivers one and sets the next
deadline one period after that observation. A delayed owner coalesces missed
ticks instead of issuing catch-up bursts. Thus traffic cannot accelerate ticks
or postpone an already due tick. A blocked OS thread or synchronous I/O can
still delay an election; conditional progress requires eventual scheduling and
bounded service times. This is not a real-time election deadline guarantee.

Built-in transports share a bounded inbox: at most 4,096 messages and 64 MiB of
protobuf encoded weights. A turn drains at most 256 messages with a 1-MiB byte
target; one legal message may exceed that target. These are message/encoded-byte
bounds, not claims about heap allocation, transport decoding or process RSS.
Saturation refuses reception without granting any consensus acknowledgement;
Raft retransmission handles loss. gRPC ends a saturated stream explicitly.

The Raft configuration limits committed entries per Ready to a 1-MiB target;
one legal large entry may exceed it. Ready and LightReady remain distinct
phases, and this target is not a total per-process memory bound. Persistent
storage and application ordering are unchanged. Custom harness transports may
retain periodic delivery through the trait's default no-op signal binding;
every built-in transport binds the real signal.

## Verification still required

The candidate adds tests for coalescing, retained inbox work, time independent
of traffic, a local read submitted at the check/park boundary, second-owner
refusal, and three voters electing/confirming reads before their next tick.
The workspace run passed 582 tests with 23 explicitly ignored tests. The first
run retained a now-obsolete fixed-sleep observation assertion; the second full
run uses the actual returned-wait contract. A separate intermediate build found
two server fixtures still supplying unbounded channel senders; those now use
bounded production inboxes. Failed logs remain retained.

The three-process Raw KV regression also passed: leader failure, continued
writes, deletes/range deletion and original-node restart/catch-up through
(term 4, index 15). Warning-denying workspace/all-target Clippy passed.
These are local candidate checks; no hosted workflow was dispatched.

Acceptance remains open for the TLA+/TLAPS scheduling proof and implementation
map; deterministic publication-before-notification, check/park, terminal and
clock fault controls; exact-revision process/MinIO/Chaos histories; and paired
release performance, idle CPU and sustained-ingress tick observations. Existing
observation source controls must be updated for the changed wait contract before
being counted as acceptance. A passing workspace run proves none of these
remaining obligations by itself.

## Queued-only outbound coalescing candidate

The second candidate removes the fixed 2-ms outbound window. After obtaining
one envelope for the current connection generation, the worker coalesces only
already queued envelopes and immediately offers the batch to the existing
stream. It inspects at most 127 additional envelopes, counting stale generations
toward that bound. Accepted envelopes preserve FIFO order and exact destination
identity, including address reuse. Byte/count targets, reconnect budgets, stream
progress deadlines and the outer route-change cancellation remain in force.

The batch is the first valid envelope followed by the destination-filtered
prefix consumed from the queue. Coalescing cannot create or reorder entries or
grant a Raft acknowledgement. Its notification-free synchronous loop is bounded
and never waits for another envelope. Each nonblocking receive observes the
queue at that call, rather than an atomic snapshot at function entry: a
concurrently arriving envelope can join this batch if already queued before a
later receive. This describes the implementation's membership rule.

Local checks pass 133 Raft tests and warning-denying all-target Raft Clippy. New
regressions cover stale-generation work bounds, address reuse, idle first-message
flush, FIFO/count limits and a legal message crossing the byte target. Existing
actual gRPC failover, blackhole/reconnect and endpoint-migration tests also pass.
Completion polling still uses its original 1-ms interval. Paired measurements,
applicable source controls and full exact-candidate failure acceptance remain
pending; this change does not claim a measured performance result.

## Receipt and read completion notifications

The third candidate removes the 1-ms completion polls. A driver-local
`CompletionSignal` carries a monotonically increasing generation and a condition
variable. Each waiter captures the generation before inspecting its receipt,
read state or applied watermark. It parks only while that generation remains
unchanged, under the same mutex used by publication. A publication between the
state lookup and parking therefore prevents parking; publication after parking
wakes all registered waiters. Repeated notifications coalesce without allocating
per-request wait records.

The driver publishes after a complete pump turn, after releasing observable
state locks. Fatal application/persistence paths and stop also notify. All five
wait sites retain their original invocation deadline: configuration application,
exact write application, read admission, exact read-context confirmation and
read application catch-up. Notifications are hints, never acknowledgements or
quorum certificates. An unrelated notification, stop, a missing or evicted
receipt, and an unknown write do not manufacture a successful result. A manual
stop alone leaves an unresolved caller subject to its original deadline.

The generation uses checked arithmetic. Exhaustion becomes a permanent error,
wakes parked waiters and fences a subsequent otherwise successful driver turn;
it cannot wrap and make a later publication indistinguishable from an earlier
observation. The signal mutex is a leaf lock: waiters release it before acquiring
application locks, and publishers release application locks before notifying.
The existing receipt retention limits and exact term/index/context checks remain
unchanged. Wake-all can increase condition-check work at high concurrency; its
CPU and tail-latency effects require measurement.

The workspace passes 588 tests with 23 explicitly ignored tests, including new
regressions for publication before parking, generation exhaustion, waking all
parked waiters, a hint without an exact receipt, and fatal application while a
caller is parked. The first workspace build found a missing test-only import;
the corrected full run passed. Warning-denying workspace/all-target Clippy also
passed. The three-process Raw KV regression passed leader failure, continued
writes, deletes/range deletion and original-node restart through (term 2,
index 14). Its initial invocation omitted the explicitly configured artifact
directory and exited before compilation or node startup; the corrected
invocation and both logs are retained. These checks do not close the scheduling
proof, implementation fault controls, exact-candidate Chaos acceptance or paired
performance measurements.
