# Persistent point client

The first implementation increment of [#40](https://github.com/c4pt0r/kv9/issues/40)
provides kv9_server::client::PersistentRawClient for asynchronous RawPut,
RawGet and RawDelete. It uses the existing public protocol and preserves quorum
reads and exact positive (term,index) write receipts. The existing synchronous
RawClient and CLI remain available.

This increment is the client foundation. The workload executable, complete
history recorder, persistent-client Chaos Mesh matrix and measurement reports
are still required by #40. A connection-reuse test is not throughput evidence.

## Configuration and bounds

ClientConfig is version 1 and rejects unknown JSON fields. The caller supplies
the token separately; it is absent from config and reports and is marked sensitive
in transport metadata. Construct the client inside a Tokio runtime.

| Input/resource | Bound |
| --- | --- |
| Configured peer identities and socket addresses | 1–32, distinct positive node IDs, distinct addresses with nonzero ports |
| Channels | One lazy channel per configured peer, shared across client clones |
| Concurrent logical calls | 1–256, immediate local rejection when full; no semaphore waiter queue |
| RPC attempts per logical call | 1–16, including the first attempt |
| Logical deadline | 1–30,000 ms |
| Retry backoff | 0–logical deadline; each sleep is capped by the original deadline |
| Key/value bytes per operation | At most 4,096 / 65,536 |
| Encoded/decoded protobuf message limit | 1 MiB each |
| Authorization token | 1–4,096 bytes, valid ASCII metadata |
| Transport connect timeout | At most 1 second and at most the configured logical budget |
| Channel buffer and channel concurrency settings | Each at most the configured logical-call limit |
| Retained attempts in a report | At most the configured attempt limit |

Configuration is validated before creating channels; payload lengths are checked
before cloning a protobuf request or issuing an RPC. These are application and
channel settings, not a proof of the complete tonic/h2 allocator footprint.
Transport queue slots can retain cancelled requests until dispatch/cleanup; the
limit applies to each of the bounded channels. HTTP/2 internals, response
buffering and total transport memory remain part of #13's wider bounds.
The workload configuration loader must additionally bound its input file before
deserialization and account for its own history and dataset allocations.

The initial client accepts literal socket addresses and a caller-provided
keyspace ID and region epoch. For fresh, unsplit benchmark keyspaces,
conf_ver=1 and version=1 is the declared initial epoch. It performs no implicit
split discovery, epoch repair or dynamic membership change. Configure all
candidate voters; a hint can select a configured node ID but cannot add an
address or force a request through a proxy.

## Logical operations and outcomes

The caller transfers one immutable operation into call. Its monotonic deadline
is calculated once before the attempt loop. Each RPC gets the remaining timeout;
the outer timeout_at and routing backoff use the same absolute deadline.
Attempt durations cover one dispatch and response decode. Logical duration
includes routing, all attempts and backoff. Neither uses server metrics.
Executor starvation can delay observing a timeout; these budgets are not a
real-time scheduling guarantee.

| Observed response | Report and routing action |
| --- | --- |
| Valid point-write response with positive term and index | Success, retain the exact receipt |
| Valid get response with a present optional-value wrapper | Success, retain the value or absence |
| Exclusive FailedPrecondition + kv9-not-leader=true | Proven refusal; another attempt is allowed within the original hop/time budget |
| Exclusive admission marker on ResourceExhausted | Terminal proven refusal: count, bytes or oversized request remain distinct |
| Exclusive read-unconfirmed marker on Unavailable | Terminal read failure, quorum/apply phases remain distinct |
| Timeout, unmarked RPC error, disconnect, malformed response | Terminal unknown write or read failure; no automatic replay |
| Invalid local input or exhausted client capacity | Local rejection with zero RPC attempts |

The NotLeader marker must appear exactly once. An optional hint must also appear
once and contain a canonical positive decimal u64. Mixed, duplicated, unknown or
malformed kv9-* metadata fails closed, including such metadata attached to an
otherwise successful response. Unknown status text is never parsed into a
refusal. An unmarked gRPC status code is retained as evidence; it does not prove
whether its origin was the server or transport, or whether a write executed.

If every dispatched attempt proved refusal but the next attempt's budget expires,
the report remains a refusal and stop=deadline explains why it stopped.
If the last dispatched attempt times out, the write remains unknown even when
all earlier attempts were refused. Admission refusal is terminal so the
generator cannot hide overload behind internal retries.

After an uncertain operation, the shared preferred endpoint rotates for the
next logical operation. That is a new invocation with its own ID and result,
not a replay of the unknown write. A failed initial seed therefore does not
prevent later requests from reaching configured survivors. Concurrent successful
calls may update this routing hint; it is not a correctness authority.

Dropping call or aborting its Tokio task discards client observation and releases
client capacity. A server blocking job may still execute and hold server admission.
The workload must stop issuance, drain every issued call and record its terminal
outcome. This client API alone cannot certify a complete workload history.

## Protocol proof and source mapping

The ClientRetry TLA+ model describes one logical point write. crSent counts
dispatches; crClosed counts the contiguous prefix of attempts whose trusted
refusals prove no effect; crApplied records effectful attempt IDs. A timeout
enters unknown and can still be followed by a late server effect. Both successful
and unknown terminal outcomes prevent additional dispatches.

The inductive invariant establishes:

1. crClosed <= crSent <= crClosed + 1: at most one attempt lacks proof of refusal.
2. Effects belong to (crClosed, crSent]: a trusted refusal excludes past and
   future effects from that attempt.
3. A ready client has crClosed = crSent; another dispatch can only add one
   potentially effectful attempt.
4. The absolute deadline remains equal to the original budget; any dispatch
   increment occurs strictly before it.

Thus any two effectful attempt IDs are equal. This is an at-most-one-effect
result under the server contract that a single dispatched attempt has at most
one effect. It supplies no deduplication mechanism and makes no retry safe by
assuming Put is idempotent: replaying v0 after an intervening v1 is observable.

The parameterized TLAPS inventory contains **8 declarations and 66 obligations**,
checked with strict mode, no fingerprint reuse, a fresh cache and SANY's semantic
module audit. Parameters are arbitrary positive attempt and clock budgets.
TLC separately explores two small configurations with two fingerprints each;
its bounded clock horizon and state counts are not an unbounded proof.
Terminal quiescence models allowed stuttering, not a liveness assertion.

| Model boundary | Implementation/contract |
| --- | --- |
| CRDispatch / original budget | PersistentRawClient::call, immutable borrowed operation, one deadline, remaining RPC timeout |
| CRRefuse | Exclusive wire classifier; the retry edge accepts only validated NotLeader |
| Trusted refusal excludes effects | Runtime point writes use commit_batch / propose_and_wait_loop; a prior internal attempt can be retried only after a typed Replaced verdict; unknown apply results return errors |
| Single effect within an attempt | Existing exact receipt, replaced-position, Raft application and durability contracts; not reproved as a network refinement here |
| CRUnknown and late CREffect | Terminal transport/deadline/protocol outcome; cancellation does not undo execution |
| CRSuccess | Positive exact (term,index) response retained once |
| Terminal freeze | All non-NotLeader outcomes break the attempt loop; route rotation affects only later calls |

The abstraction does not verify Rust, tonic, h2, the network, scheduler or storage
by refinement. It assumes truthful authenticated server refusal metadata and
the existing server execution contract; it does not tolerate Byzantine servers.
The source boundary review covers the pinned tonic 0.14.6, hyper 1.11.0,
hyper-util 0.1.20, h2 0.4.19 and tower 0.5.3 in Cargo.lock: tonic reconnects
readiness, while its dispatch path forwards one HTTP/2 send_request result.
No application retry is delegated to that transport path. Dependency changes
must re-audit this assumption.

## Reproducible verification

    cargo test --locked -p kv9-server client::tests -- --test-threads=1
    python3 scripts/check-client-controls.py --output /tmp/kv9-client-controls-new
    python3 scripts/check-client-protocol.py \
      --jar /path/to/tla2tools-v1.7.4.jar \
      --tlapm /path/to/pinned/tlapm \
      --output /tmp/kv9-client-protocol-new

The protocol runner uses the same tool pins and semantic auditor as the existing
metadata/Ready gates. It retains 15 TLC runs: four full checks, nine runs for
three baseline/mutant/restored protocol controls, and two reachability witnesses.
It also retains 16 fresh proof/audit runs, rejects omitted proofs and custom
axioms, and rejects six corrupt output controls. The protocol mutations are
retrying an unknown write, resetting the deadline and falsely refusing an
already effectful attempt.

The real gRPC tests count accepted TCP connections at the server. They cover
66 mixed RPCs on one connection, reuse after server restart, bounded routing,
malformed receipts/refusals, admission and capacity outcomes, and a shared logical
deadline. The response-loss fixture makes v0 visible while withholding its reply,
then a separate client reads v0 and writes v1; the original reports unknown
without replaying v0. This stateful test uses a scripted server and is not a claim
of Raft/MinIO durability. The independent complete-history witness and real
Chaos Mesh workload remain outstanding acceptance work in #40.

Three isolated Rust mutations must compile, select exactly one test, fail the
specified behavioral assertion, and pass again after exact source restoration.
Constructor counting, zero tests, compilation failures and unrelated failures
cannot satisfy the gate. Complete workload terminal accounting needs its own
additional control when the recorder is implemented.
