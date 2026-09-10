# Asynchronous point-read preparation

Tracking: #20 and #9. This candidate starts from Ready group synchronization
`892b2a178450309859113c942f1738c070130eb5`. The separate proposal-queue
experiment is excluded: its measured tmpfs write throughput regressed. The
target is client-visible memory Raw KV throughput approaching the local Redis
reference, while preserving the existing quorum, apply, and snapshot contract.

## Execution path

Public `RawGet` first reserves the existing request count and encoded-byte
budget. It asynchronously registers a unique ReadIndex context with the
existing Raft owner. The owner performs admission, processes Ready, matches the
exact confirmation, and waits for unified local apply coverage. An individual
oneshot returns the resulting index to the request. Only the driver can turn
that index into the existing private `ReadBarrier` capability.

After preparation, one `spawn_blocking` job consumes the capability, rechecks
serving state, and acquires the existing established engine view. The region
and epoch gate and value lookup consume that same view. Engine, peer, and
metadata mutex operations remain outside Tokio preparation. Cold endpoints
retain the existing synchronous lifecycle refusal path.

This removes a blocking worker's quorum wait and the async caller's receipt
ring search. It retains one blocking engine job per GET, normal RPC encoding,
the existing Raft owner, and the existing synchronization required by Ready.
Batch reads, scans, metadata operations, and writes retain their existing path.
It introduces no lease, new node, coordination service, or single-node fast path.
No throughput gain is established until an unchanged-protocol comparison runs.

## State, bounds, and cancellation

For one driver, contexts are `(process incarnation, checked sequence)` encoded
in 24 bytes. Synchronous and asynchronous callers share the checked counter;
exhaustion refuses new contexts without wrapping. Process-incarnation
uniqueness is an existing protocol premise.

Each request moves through `queued -> claimed -> active -> selected -> released`.
Deferred admission returns `claimed -> queued`; error, cancellation cleanup,
and stop can move an owned request to release without success. There is one
owner invoking peer admission. A registry mutex protects the queue and active
map, but never surrounds a peer callback or a completion sender's destruction.

The reservation count includes queued, claimed, active, and selected requests.
It is at most 128, including requests whose receiver has disappeared. A turn
inspects at most 64 queued requests, including canceled and deferred requests.
The queue bound is not an RSS bound, an engine-work bound, or a bound on the
upstream Raft library's internal read-state storage. The existing public
admission ledger separately bounds live public request count and encoded bytes.

Cancellation before owner inspection prevents admission. Cancellation racing
an already inspected request can still permit its read-only Raft admission;
cleanup retains ownership until the callback or owner finishes. Closing the
receiver precedes its cancellation wake, preventing a missed cleanup signal.
Stop detaches queued and active requests under the registry lock and fails them
outside it. An already claimed callback retains its reservation until it returns;
the unclaimed suffix remains reachable by stop while that callback is blocked.

The absolute deadline is set once at registration. Timeout records whether the
request has observed quorum confirmation or is still awaiting it. A published
result wins over a simultaneously observed timer, consistent with the existing
known-result-first convention. Selecting a completion under the registry lock
is its eligibility point: a later stop may preserve that already selected
success. It cannot create a new success for a request still in the registry.

The original public reservation moves continuously from preparation into the
blocking job, without an intervening await. Handler cancellation while waiting
for preparation drops its future and starts no engine job. Once the job is
spawned, the job owns the reservation until actual completion or unwind, even
if the client disconnects. `public_raw_read_backend` running time now includes
logical preparation and blocking-job queue/execution time; it does not measure
OS thread occupancy. `public_raw_read_prepare_queue` no longer includes the
blocking pool's queue delay.

## Safety argument and remaining proof work

Assume the underlying Raft ReadIndex implementation confirms only a valid quorum
read for the supplied context; successful whole-Ready completion preserves the
existing unified applied watermark; and the established-view implementation
enforces its snapshot contract. Those are composition premises, not facts
proved by this new registry.

For an arbitrary finite execution, maintain these invariants:

1. Every reserved request has one owner in the queue, callback, active map, or
   completion selection. Registration is the only increment and refuses at the
   limit; ownership transfers do not decrement. Request destruction performs
   the sole decrement. Thus the counted population never exceeds 128.
2. A request's confirmation changes only from absent to the first index returned
   for its exact full context. Checked sequence allocation prevents another
   invocation in the same process from reusing the context. Unmatched states
   cannot authorize this request.
3. A success is selected only with confirmation index `i` and a unified applied
   observation `a >= i`, after the whole pump and completion publication succeed.
   Failed Ready processing closes pending requests before success selection.
4. Only a successfully received index constructs a `ReadBarrier`; only the
   existing established-view seam consumes it for a data snapshot. The request's
   context gate and data execution use the resulting same view.

The base state is empty. Register preserves uniqueness and the capacity bound.
Claim and defer transfer the same request without changing its reservation.
Confirm preserves exact identity and the first index. Selection checks coverage
before removing ownership from the map, establishing invariant 3. Error, stop,
timeout, and destruction cannot construct a successful index. The driver and
runtime wiring then establish invariant 4. A later stop can neither erase an
already eligible result nor authorize an unselected request. These cases cover
the registry transitions and give a conditional preservation argument for the
existing read protocol.

Progress additionally requires fair owner scheduling, finite callbacks, a live
quorum, and eventual apply. Registration and cancellation wake the owner. A full
admitted prefix wakes retained work; deferred admission does not self-spin.
The post-pump readiness check wakes a read deferred before an election no-op
committed, without requiring a new timer tick. This is conditional progress,
not a latency bound under partitions or failed storage.

This argument and executable tests do not constitute a mechanically checked
refinement of the Rust implementation. Parameterized model composition for the
new waiter lifecycle, the underlying Ready candidate's remaining proof gates,
and actual Chaos Mesh E2E on the candidate remain open. Runtime promotion to
master requires that acceptance work; earlier exact-revision proofs or fault
runs must not be relabeled as evidence for this candidate.

## Local verification

Core registry and actual-driver tests cover exact context matching, first
confirmation, apply coverage, absolute timeout phases, cancellation and bounded
inspection, deferred wake behavior, callback unwind, stop ownership, selected
completion ordering, counter exhaustion, follower refusal, and failed Ready.
Public-handler tests cover cancellation before engine work and reservation
retention through actual wire cancellation during a blocked engine job.

Three-runtime product tests hold a committed write unapplied while asynchronous
preparation is pending, then require the resulting read to observe that write.
A stale direct-view bypass is checked in the same scene. A second fixture
completes preparation, commits either region epoch-half change, then executes
the held job: the obsolete context must be refused with the exact region ID;
a current context must return the post-change value. These fixtures explicitly
check that they exercised asynchronous preparation rather than cold fallback.

The initial local gate passed 622 workspace tests/doctests with 23 explicitly
ignored tests and warnings-denied workspace/all-target Clippy. Seven compiled
implementation controls passed their baseline/mutant/restored triples: ignored
context sequence, overwritten first confirmation, omitted apply coverage,
canceled admission, early claimed-reservation release, omitted election wake,
and deferred-admission spin. The runner requires one exact named test and its
intended semantic assertion; compile failures and empty selection cannot pass.

The default-feature binary also passed the real three-process Raw KV fixture:
typed context and follower refusals, write replication, leader kill and new-leader
read/write, delete/range-delete, and original-directory restart catch-up to
term 2/index 14. Four executing process lifetimes were independently identified
by executable hash and start time, and all exited. The final statuses show zero
queued, active, and in-flight asynchronous reads. Executable SHA-256:
`ef5d73feb062614b52ddd0d8eebba7ea74ab2971ea7cbcbb9ccc7028cdd6d184`.

Rerun from this candidate with the appropriate local target directory:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
python3 scripts/check-async-read-controls.py --output /tmp/kv9-async-read-controls-new
bash scripts/raw-kv-e2e.sh
```

Retained initial evidence is under `/tmp/kv9-async-read-controls-first` and
`/tmp/kv9-async-read-process-first`, with workspace/Clippy logs prefixed
`/tmp/kv9-async-read-`. Early compile failures (a missing method brace and a test
import path) are preserved separately from the later successful runs. These
checks do not establish full history/Chaos Mesh acceptance or a performance gain.
All routine execution is local; hosted CI remains manual-only for releases or
key milestones.
