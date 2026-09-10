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
