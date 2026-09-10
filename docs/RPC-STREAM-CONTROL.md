# Bounded streaming-gRPC control

Historical experiment specification at `40e813f37672eb66b80b307c0e20f9bde0adf31c`.
The measured selection is recorded in the mainline performance report. The
subsequent normal-port streaming and native batch integration is described in
[Native RawKV batch client](RAW-BATCH-CLIENT.md); the experiment-only/default
statements below apply to the frozen historical boundary.

Tracking: #9, #13 and #20. This adds `tonic_stream` beside `tonic_unary` and
`tarpc_tcp` in the opt-in `rpc-experiment` build. It is a transport candidate,
not a production selection or an accepted throughput improvement. The measured
[b49 unary/tarpc comparison](https://github.com/c4pt0r/kv9/blob/aa56fac/scripts/redis-reference/results/b49a2f6-rpc-framework-c64-tmpfs-diagnostic.md)
motivates testing HTTP/2 stream setup overhead separately from tarpc framing.

## Runtime boundary

`KV9_GRPC_STREAM_EXPERIMENT_ADDR` enables a separate nonzero loopback listener.
Normal builds contain neither this listener nor its generated service. The
existing public and Raft protobuf service definitions are unchanged. The new
`PointStream.Exchange` bidirectional service carries authenticated GET, PUT and
DELETE envelopes with the original inner protobuf messages. Every point frame
authenticates through the same authenticator and invokes the same `Handler`,
`Kv9Grpc`, public admission ledger and backend used by the tarpc control.

No new write batching, durability mode, read lease, Raft transport or speculative
write acknowledgement is introduced. Quorum reads and exact committed/applied
receipts remain properties of the shared handler/backend. Multiple calls may
complete out of order; each response carries its own stream-local request ID.
All envelope Debug implementations redact credentials and application data.

## Client state and transitions

For one endpoint, the cache holds at most one current generation. A generation
contains `(closed, next_id, pending)`, a bounded FIFO sender, an owned reader task
and a cancellation token. A pending entry stores its operation and one response
sender. The one initialization lock serializes replacement. Calls retain an
`Arc` to their selected generation, so replacement cannot move an existing call
onto a new stream.

The transitions in `rpc_experiment/stream.rs` are:

1. **Register.** Under one state mutex, require an open generation and available
   pending capacity; use checked increment to allocate a fresh positive ID,
   insert exactly one pending entry and enqueue its immutable frame with
   `try_send`. There is no await between these steps. Queue failure or exhausted
   IDs closes the generation. No ID wraps or is reused. A cancellation guard is
   installed before this block.
2. **Receive.** Under the same mutex, require an open generation and remove the
   exact ID from `pending`. Unknown, duplicate or missing replies close the
   generation. Decode the reply using the registered operation, preserving all
   metadata entries. Reject invalid status/payload shapes, success control
   metadata, invalid GET presence/value bounds, zero write receipts and
   ambiguous error controls before delivering any result. The unchanged client
   classifier is reused for error controls. Even a legitimate DataLoss status
   conservatively closes the stream; it does not authorize replay.
3. **Close.** Mark the generation closed and drain remaining response senders
   with an unmarked unconfirmed status. Cancel outbound production and terminate
   the reader. Reader exit, stream failure and a dropped incomplete call all use
   this path. A guard belongs only to its generation and never clears or closes
   the endpoint's later replacement. Closing an already closed generation is
   harmless. A reader cleanup guard exists before spawning, including the
   abort-before-first-poll case.
4. **Complete.** Once the matching response is observed, disarm that call's
   cancellation guard. The ordinary point client still checks the response and
   applies its original retry policy: only an exclusive valid NotLeader refusal
   permits a new attempt. All collateral transport-failed writes remain unknown.

One timed-out or canceled call closes its entire generation. This deliberately
trades additional uncertain sibling results for bounded cancellation ownership:
the adapter never retains an unbounded collection of abandoned requests or
cancellation tombstones. Later logical calls may reconnect. The point client's
existing absolute monotonic deadline includes connect, enqueue and observation;
the internal adapter methods rely on that outer deadline for connection setup.

## Server state and transitions

For an open response stream, let `last_id` be the greatest accepted request ID.
Before decoding another request, the pump reserves one bounded response-queue
slot. On receipt it rejects IDs `<= last_id`, operations outside GET/PUT/DELETE,
and remaining deadlines outside 1 through 30,000,000 microseconds. It then
records `last_id` and the receipt-time monotonic deadline and creates one handler
future carrying that reserved slot. Authentication and point payload decoding
precede backend entry inside the shared handler.

Each handler future returns exactly one correlated response into its reserved
slot. Pending jobs stay in an owned `FuturesUnordered`; no per-point detached
transport task is spawned. Input EOF drains accepted jobs. Invalid input,
response-stream drop or server shutdown drops unfinished transport futures.
The existing handler's independent completion task continues an already prepared
write with its admission reservation until exact apply settles.

A transmitted remaining duration can acquire extra server-side time in transit;
it is not a synchronized-clock deadline proof. The client's independent absolute
deadline is authoritative for observation. A timeout never proves no effect.

## Safety argument and implementation mapping

Assume the original authenticated handler/backend satisfies its documented Raft
contract, the framework preserves a connection's message bytes/order, and
mutually exclusive state operations have their ordinary Rust synchronization
semantics. This proof is conditional on those premises; it is not a proof of
tonic/prost/HTTP2 or the complete later Rust/Raft runtime.

**Unique binding.** Initially `pending` is empty and `next_id = 1`. Register
advances `next_id` strictly, never wraps, and inserts/sends under one mutex.
Receive only removes an existing entry; Close inserts none. Induction therefore
gives at most one pending call per ID and at most one delivery for that ID.
The registered operation determines the reply decoder. An ID not in the map
cannot provide either a success or a retry-authorizing marker to a pending call.

**At most one handler invocation per accepted frame.** The server accepts only
strictly increasing positive IDs. Each accepted frame is moved into exactly one
future; its operation selects exactly one shared point method. Future polling
and response queuing introduce no retry edge. Invalid frames execute no point
handler. An earlier frame may already have executed when a later bad frame
closes the stream; those unfinished observations correctly remain uncertain.

**Success preservation.** A server success envelope is constructed only from
the shared handler's success. Receive checks its exact binding and valid shape
before delivery. Thus every delivered write success projects to that invocation's
committed/applied receipt and every delivered GET success projects to a shared
quorum read. Concurrency may reorder completions, but does not substitute another
request's payload or receipt. The linearization point stays in the shared backend.

**No uncertain replay.** Close only emits unmarked uncertainty and stops that
generation's outbound production. No transition copies a pending mutation into
another generation. Only the outer client's existing validated NotLeader edge
retries. A frame sent before cancellation may still execute once; the adapter
never claims its absence and never turns the uncertainty into a refusal.

**Reservation ownership.** For each server stream, partition its response queue
capacity into available slots, slots owned by running handler futures, and slots
containing buffered replies. Reserve, Send, Poll and Drop preserve their constant
sum `CHANNEL_LIMIT`. Consequently running point jobs plus buffered responses
never exceed that limit. Canceling transport observation does not release a
prepared write's separate public admission reservation; the existing completion
task owns it. These transport and public ledgers count different resources.

Conditional liveness additionally requires an available Raft quorum, eventual
backend/transport/executor service, response consumption and a sufficient
deadline. Under those premises an accepted job settles and its slot is released.
Without them the bounded deadline, cancellation or connection failure yields
uncertainty instead of an invented success. No unconditional availability claim
is made.

## Resource and availability limits

- Eight accepted TCP connections and eight live response streams per listener;
  the stream permit lives in the returned response stream. Excess connections
  close; excess streams receive an unmarked resource error.
- At most 256 running-or-buffered point responses per server stream. Each
  framed protobuf envelope is capped at 1 MiB plus 8 KiB; shared point payload
  and authorization limits still apply. Decoder and HTTP/2 allocations have
  separate framework bounds and are not charged to public admission.
- Per endpoint, pending entries and queued outbound frames are independently
  bounded by configured client concurrency, at most 256. The existing global
  point-client capacity remains unchanged. Empty/closed generations do not
  retain an active outbound producer or reader.
- Accepted sockets explicitly enable TCP_NODELAY. Shutdown cancels socket I/O
  and stream pumps; dropping a stream owns and aborts its transport task.
- Each voter hosts its own adapter; there is no new coordinator, proxy or
  service-critical singleton. This does not establish cross-host availability.

## Acceptance sequence

Run the adapter negative controls and the ordinary workspace checks locally,
then retain same-source standalone server/workload builds with
`scripts/build-rpc-experiment.py`. `scripts/rpc-experiment-e2e.py` now exercises
all three transports on three real voters with leader termination, original-
directory restart, independent full histories and serial-fresh drain evidence.
Keep failed attempts and exact source/executable/feature identities.

Only then compare all transports with repeated same-artifact paired Redis
measurements. Production promotion additionally requires exact-feature actual
Chaos Mesh histories and the applicable proof/refinement gates. Earlier default
runtime Chaos and b49 tarpc evidence do not establish streaming acceptance.
Redis-class performance remains open and precedes dynamic multi-Raft and
automatic range splits. Hosted CI remains manual-only.

## Local validation, 2026-09-10

The feature-enabled workspace run passed 670 tests/doctests, zero failed and
23 ignored. The default workspace separately passed 645, zero failed and
23 ignored. All-target experimental Clippy passed after correcting a test's
lexical lock scope; no lint was waived. The final focused streaming suite passed
all 15 tests after that test-only correction and an accurate idle-restart test
rename. No runtime source changed after the first successful focused suite.

The real three-voter, ordinary-WAL run exercised all three transports and passed
complete-history validation over 495 operations: unary 152 (139 OK, 13 unknown),
tarpc 143 (129 OK, 14 unknown), and streaming 200 (184 OK, 16 unknown). Leader
termination and original-directory restart produced successful GET/PUT operations
fully contained in each fault/recovery window. Unknown operations are retained
in the histories; these correctness totals are not performance measurements.

Independent acceptance verified all six voter and three client lifetimes exited,
12 owned listening-socket assignments, unchanged per-node store incarnations and
nine final voter captures with two serial post-client-exit export advances,
zero public/read/apply occupancy and stopped=false. Both build graphs explicitly
enable the experiment in root/server/client and leave engine/Raft features empty.
Audit SHA-256:
`d9d41f057dc90bae677fb80d2818f3362ebf08ff45348c541b03b43e3ba3db5a`.
The inventory has 104 references / 370,261,817 bytes, SHA-256
`3bdd8373bda76880e646c251ba8d53e249635cff714887a2094f0830aef52903`;
every reference was read back. All audit stages passed on their first attempt.

The separately retained debug server SHA-256 is
`bfa8e503348fdb00ef0ba2eb336449251e4588ead2becbde624e524109e09d58`;
the workload SHA-256 is
`cc994b44d17a0db9edc7bd95b5d0d267d3bb7fac8be86daf83076f96c0b75f9e`.
The build truthfully records a dirty b49-based tree with 425 inputs, source-tree
SHA-256 `7d4250b57404800166cb8aee581d375329adf48b318d81ee4ee90dd78a688f8d`.
All inputs were independently reopened before this validation append. The final
fixture success prose was subsequently corrected from "both" to "all three";
the executed fixture copy and original logs remain unchanged. Neither subsequent
edit changes executable behavior or the three machine-recorded transport cases.

The adapter tests use an instrumented backend. Their deterministic response
backpressure test holds 256 completed replies without polling the actual returned
stream, then delivers all 513 operations and checks admission drain. The live-
stream limit remains occupied after response headers. Held-write tests establish
unknown classification, no replay, and reservation ownership through cancellation
and deadlines. The restart unit test covers an idle generation; the actual
three-voter history run covers ongoing workload during leader termination.
Outer TCP connection saturation, client queue saturation and maximum frame-size
boundaries are source-reviewed here, not separately saturated by these 15 tests.

Retained evidence:

- `/tmp/kv9-stream-build-first`: same-source standalone build records/artifacts.
- `/tmp/kv9-stream-e2e-first`: all histories, fault windows, socket/endpoint
  ownership, status writer identities and serial-fresh drain observations.
- `/tmp/kv9-stream-tests-evidence`: private focused test attempts; the initial
  missing test import and dependent compile errors are preserved.
- `/tmp/kv9-stream-{default,feature}-workspace-first.log`,
  `/tmp/kv9-stream-tests-final.log` and
  `/tmp/kv9-stream-clippy-all-{first,second}.log`: exact local checks.
- `/tmp/kv9-stream-report-controls/result.json`: six invalid transport/schema/
  build-feature controls rejected by the unchanged report checks.
- `/tmp/kv9-stream-independent-audit`: independent source/history, lifecycle,
  listener, window, freshness and bounds audits plus verified inventory.

The initial locked check refused the intentional tokio-util runtime-feature
lockfile update; an offline resolution added its futures-util dependency edge
without package upgrades. That check failure and the test/lint failures above
are retained. No runtime failure has been observed in this local increment.
The conditional argument above is not a machine-checked complete runtime proof.
No streaming throughput or actual experimental Chaos result is claimed here.
