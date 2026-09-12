# Bounded quorum-message trace

This diagnostic feature implements the next observation boundary in
[the quorum-latency plan](QUORUM-LATENCY-NEXT.md). It is not a runtime
optimization, a new performance result, a core Raft proof, or a completed
industrial acceptance gate. The selected runtime remains `11113f6`. Implementation lives on the isolated
`codex/quorum-trace-slots` branch; it is not promoted as a runtime optimization.
The prior `1875e74` capture lost 1,083 selected observations to its shared
observer mutex across six process prefixes. The frozen reader correctly
refused every complete-context RTT attribution. This revision changes only
observer storage to remove that identified source of loss; its actual capture
and overhead qualification are pending.

## Scope and activation

`--features quorum-trace` enables local message/group events and the existing
six read-stage metrics. The default build contains neither this recorder nor
its shadow queue, exporter, or trace fields. No wire field, Raft setting,
transport framework, admission limit, retry, deadline, apply watermark, or
completion rule changes. The current production transport covers metadata
region 0; this is not dynamic multi-Raft instrumentation.

The recorder samples exactly the 24-byte ReadIndex contexts whose final
big-endian u64 sequence is divisible by 256, including sequence zero. A process
preallocates at most 65,536 events. Every eligible observer invocation counts
as offered; selected invocations either record once or increment exactly one
of contended, poisoned, full, or exhausted. Counter overflow invalidates the
whole counter snapshot. Queue continuation stages exist only for selected
starts that obtained a retained local ticket. Offered does not mean a client
request count, and group member observations must not be summed as unique
requests across stages.

Recording reserves an immutable preallocated slot with a strong atomic CAS.
The cursor only advances up to capacity, never resets or wraps, and each failed
CAS implies another successful reservation. There are at most 65,536 such
advances during one recorder lifetime. Each unique reservation initializes its
own `OnceLock`, so no other writer can hold that cell. No mutex or protocol
lock is acquired. The existing contended/poisoned fields remain zero for schema
compatibility; capacity and clock exhaustion still return no observation. The
observer does add CPU, atomics, clock reads, memory, and scheduling overhead;
that overhead must be measured against an uninstrumented control.

## Boundaries

The JSON `stage_names` explicitly names every position in `stage_counts`.
Events retain the original context, message term/from/to/kind, and process-local
monotonic timestamp. Group events leave term unobserved; a reader must not
fabricate it by locking the Raft peer or by joining an ambiguous message term.

| Observation | Source boundary and meaning |
| --- | --- |
| Group submit/admit/defer/refuse | Existing ReadIndex invocation and its unchanged result. A deferred context may be submitted again. |
| Quorum confirmed | First exact ReadState confirmation for the active sealed group. |
| Completion eligible | Selection with confirmation covered by the unified apply watermark, called only after successful whole-pump processing and completion publication. This is not successful client delivery. |
| Members uncovered / group closed | Active members removed without apply coverage, or queued/active requests closed. Cancellation and timeout are combined when no separate cause is observed. Requests removed before group submission have no complete group trace. |
| Outbound offered/accepted/rejected | Existing bounded peer queue and unchanged try_send result. Accepted is observed after try_send and can race dequeue. |
| Outbound dequeued / route discarded | Exact existing Arc destination comparison, in first receive and queued coalescing. Dequeue does not prove network transmission. |
| Queue abandoned | A retained ticket dropped before a recorded terminal queue boundary, such as connection-failure drain or task/channel destruction. A post-capture drop is outside the prefix. |
| Unknown peer / encoding rejected / masked outbound | Existing early send exits; the test-only partition mask remains test-only. |
| Inbound validated | Existing decoded message after root, receive-authority, envelope and sender validation, before inbox admission. |
| Inbox offered/admitted/rejected/drained | Existing bounded inbox and unchanged admission/drain order. Test-only inbound masking occurs after drain and can leave an unmatched driver step. |
| Driver step | Immediately before the existing Raft step call, not its successful return. |

Each queued observation uses its unique originating slot index plus one as
a nonzero ticket. Non-ticket events leave intentional gaps in ticket IDs. Each outbound ticket
retains the actual destination Arc allocation, preventing pointer reuse while
the trace names it. Ticket and route identifiers have meaning only inside one
process capture. The wire has no equivalent route token.

Periodic Raft heartbeats may repeat the same context. Count every emission and
response separately. Duplicate or missing matches make exact attribution
unknown. Even a unique local outbound-dequeued to inbound-validated pair is a
candidate interval including transport and remote work, not proof that the
particular transmission caused that response. Never subtract process clocks
or add means from different populations. A reversed accepted-to-dequeued span
is unavailable, not zero; an independently ordered offered-to-dequeued span
can remain a clearly labeled partial observation. Any observation loss disables
complete-context and message attribution for that capture; exact local ticket
spans may remain explicitly partial.

## Capture and process identity

The experiment owner creates an empty regular file named
`data-dir/quorum-trace.capture` only after timed clients have exited. The status
loop polls at most once per second and attempts capture once. It closes only
the observer, waits at most 100 ms for active observation callbacks, then takes
a finite snapshot. A timeout, incomplete slot, invalid marker or I/O failure is retained
as a terminal observation failure, without retrying a different prefix.
Default OS signal termination need not run destructors, so capture is explicit
while the node remains alive rather than relying on shutdown.

`quorum-trace.json` is atomically renamed from a create-new temporary file,
limited to 64 MiB, with no fsync or durable receipt. Its exact envelope keys are
`node_id`, `process_id`, `exporter_created_unix_ns`, `captured_unix_ns`,
`process_start_ticks`, `boot_id`, and `trace`. Unix nanoseconds are decimal
strings; event timestamps use the local recorder Instant. Linux start ticks
and boot identity bind process lifetimes; missing identity cannot authorize
actual attribution. An offline reader must bind these values to independent
expected process identity, never trust a PID or old file alone.

Capture is a finite prefix, not queue shutdown or client-drain evidence. Open
tickets and groups remain unmatched. Explicit post-measurement capture, source
and executable binding, lifecycle outcomes, overhead control and trace loss
accounting are required before publishing an actual latency diagnosis.

## Safety correspondence

The following argument concerns observational additions, not a proof of the
underlying Raft implementation, liveness under resource exhaustion, or
performance neutrality.

1. Erase cfg-only fields/calls and their local events. Every wire message,
   original queue entry, routing Arc comparison, admission result, consensus
   callback and completion condition is unchanged. Recording returns only
   local tickets or no observation; these values never select protocol work.
   Retaining PeerDestination extends only a SocketAddr allocation's lifetime.
2. The inbox shadow queue starts empty. Under the original inbox mutex, every
   accepted message appends exactly one optional ticket and every original
   pop removes one shadow entry. Rejected messages append neither. Induction
   on these operations preserves positional correspondence and all original
   message weights/order/limits. Dropping tickets observes abandonment only.
3. For each open observer invocation, offered increments once. An unsampled
   key ends there; a selected invocation increments selected then records one
   event or exactly one loss class. With counters valid and callbacks quiescent,
   selected = recorded + contended + poisoned + full + exhausted, and recorded
   equals the corresponding event count. The reservation cursor starts at zero; a successful strong CAS from n to
   n+1 uniquely owns slot n. By induction the cursor equals the number of
   successful reservations and cannot exceed capacity. Therefore slots cannot
   alias, and originating-slot-plus-one tickets are unique and bounded by
   capacity. Only the owner calls `OnceLock::set`; this call has no competing
   initializer. The retained route remains alive inside the immutable cell.
4. Entry checks closed, increments active, then rechecks closed, all SeqCst.
   Capture sets closed and waits for active zero. An entrant that passed its
   second check before closure is visible to that wait until its final decrement;
   one that enters after the cut cannot update counts/events. Late active-only
   increments cannot alter the snapshot. A successful capture therefore reads
   quiescent observation state. The acquire/release ordering of the SeqCst
   active counter and each OnceLock publishes completed events. Every reserved
   cell must be initialized; timeout or a missing cell yields no snapshot.
   A writer unwinding before publishing its reserved event cannot silently
   remove that hole from an otherwise accepted prefix.
5. Group confirmation is still the exact first matching ReadState. Eligibility
   is observed at the existing selection point after successful whole-pump
   completion and only with covered confirmation. No trace record substitutes
   for this fence, an apply/view receipt, or a durable write acknowledgement.

Focused tests cover sampling/repeated contexts, concurrent complete ticket
chains, exact concurrent saturation, counter overflow, incomplete-slot rejection,
concurrent capture, retained route
lifetime, abandonment, bounded inbox behavior, refusal/cancellation/close,
first confirmation and apply coverage, route replacement/coalescing, and
one-shot process-bound export. Local build/test results and actual experiments
must be recorded separately; no measured improvement is claimed here.

## Previous revision source validation

These results belong to `1875e74`; the slot revision needs its own diagnostic
source checks and a source-bound production build. The default and standalone
read-stage compilation paths contain no changed code.

The first source qualification passed without a failed command or rerun:
709 default workspace tests/doctests (23 existing ignored), 453 diagnostic
raft/server tests (1 existing ignored), and 438 standalone read-stage tests
(1 existing ignored). The 15 new focused diagnostic tests also passed as a
separate filtered gate. These populations overlap; do not add them as unique
tests. Formatting and all three explicit Clippy feature configurations passed.
The root-owned release-profile build-cache transaction invalidated first-party
artifacts and verified source stability and fresh compiler artifacts. The
80 GiB filesystem floor and 16 GiB additional build reservation were retained.

This is source acceptance on a captured working draft, not a new production
build, performance result, recovery run or Chaos Mesh acceptance. Exact logs,
commands, source hashes and the terminal record are retained under
`docs/quorum-trace-source-v1/`. Later documentation and offline-reader additions
are separately identified; compiled Rust/Cargo inputs must match the source
snapshot before publication. Session 23090 ended with exit 0 (`d0261f`).

The offline reader is `scripts/quorum_trace/read_trace.py`. Its 17 finite
synthetic controls passed after source review tightened missing-intermediate
and repeated-group attribution. Prior helper versions and successful control
runs remain in the frozen reader evidence. The final reader requires all local
queue intermediates for complete-context timing; an available endpoint span
alone does not make a complete chain. Source/control evidence is available on
the diagnostic branch, with no actual trace result yet.
