# Isolate existing outbound Raft work from public RPC scheduling

This uninstrumented prototype branches from selected CRC behavior. The public
runtime retains two workers, event interval eight and adaptive global scheduling.
A separate per-node runtime with one worker, all drivers enabled and event
interval eight owns the existing GrpcTransport peer workers and their outbound
tonic connections. Its thread name is `kv9-raft-tx`. No extra network service,
port, proxy or cluster-wide singleton is introduced.

Inbound Raft service/HTTP2 work still uses the existing public listener/runtime.
This is outbound executor isolation, not complete Raft networking isolation or
CPU affinity isolation. All workers still share the configured process CPU set.
The Raft owner and blocking pool remain unchanged. Total async worker count
increases from two to three per replica; any measured change includes that
resource tradeoff. A shared three-worker control is needed to separate placement
from worker-count effects before claiming an isolation-specific improvement.

## Why this experiment

The completed request-body observation records sub-microsecond c1 heartbeat
handoff means, while separate sender/inbox observations expose longer waits
under mixed public traffic. Making the entire public runtime poll I/O every
task instead of every eight tasks regresses c1 GET, c64 GET and c64 mixed
throughput, means and tails. Both complete repetitions reject that setting.

This candidate changes execution placement rather than adding another global
maintenance-frequency sweep or replacing the batch queue. It preserves the
original peer state machine. Public task competition may fall, but extra worker
scheduling, cache misses and cross-runtime wakes may outweigh the benefit. No
throughput, latency or CPU improvement is assumed.

## Source correspondence and resource ownership

The only executable changes are construction/ownership of the additional runtime
and passing its handle to GrpcTransport instead of the public runtime's handle.
The peer worker, tonic endpoint configuration, best-effort outbound queue,
route-generation checks, coalescing, body channel, progress/connect watchdogs,
keepalive and backoff code are unchanged. Existing peer tasks retain their
original task count and queue bounds. No new per-message task or buffer is added.

Project a candidate execution by erasing executor-internal maintenance and the
new executor's identity. Each application transition remains a base transition
with identical guards, values and effects. Induction over the projected trace
preserves any base invariant valid for arbitrary scheduling interleavings: the
initial application state agrees; each unchanged transition preserves the
relation; erased maintenance transitions leave application state unchanged.
This is a conditional exact-delta correspondence, not a machine-checked proof
of Tokio, Rust memory ordering or complete implementation refinement.

In particular, moving a runnable task cannot authorize fresh Safe ReadIndex,
seal/extend a read group, complete a failed pump, cover unapplied data or bypass
epoch/view checks. Durable WAL sync, Raft commit/apply and write acknowledgements
retain every original guard. Deadline/cancellation outcomes can differ with
scheduling; no timing equivalence or guaranteed latency bound follows.

Both runtimes are constructed before the original owner-creation boundary. A
construction failure returns a configuration error before the owner starts.
NodeRuntime owns the peer runtime after the public runtime in field drop order
and before the existing store guard. The original Drop first cancels public
work, stops/joins the Raft owner and aborts the public acceptor. Existing fields
then drop in order; the public runtime releases task-held transport references
before the peer runtime is shut down. The store guard remains last. The transport
still aborts peer tasks when it drops; no detached owner outlives store ownership.

Progress remains conditional on OS/executor fairness and eventual network and
storage availability. A node-local executor is not a new cluster-wide authority;
the existing quorum/election protocol handles loss of a replica. This does not
establish the complete independent-host failure matrix or close admission bounds
for inbound streams.

## Acceptance and next comparison

Run the server regressions, experimental RPC configuration, formatting and
all-target Clippy locally. Then bind a clean original release and check actual
stream/unary leader-loss/original-directory restart histories, outcomes, drain
and process lifetimes. Unit tests using their own runtime alone cannot verify
the production constructor. Startup failure and teardown coverage are relevant
to the additional owned runtime, even though consensus source is unchanged.

After source/recovery acceptance, perform a bounded uninstrumented c1/c64 GET
and mixed screen against selected CRC and same-run Redis with complete forward
and reverse repetitions. Keep read means/tails and both operation populations.
Do not change storage guards or remove original evidence for capacity. If the
candidate is promising, compare a shared three-worker control using the same
resource budget; only then expand to full point/batch and actual exact-source
Chaos Mesh gates. A regression ends this candidate's expansion. Current selected
runtime remains CRC. Full core proof composition and industrial availability
remain open; hosted CI stays manual.
