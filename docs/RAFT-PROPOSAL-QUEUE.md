# Bounded owner-consumed proposal queue

Tracking: #20 and #9. This candidate extends exact Ready
`892b2a178450309859113c942f1738c070130eb5`. It is not accepted on the main
runtime branch. The indexed-read experiment is not part of this lineage.

## Purpose and limits

Public Raw writes enqueue their encoded command without taking the Raft peer
mutex, which also protects persistence. The existing sole driver owner submits
an available FIFO prefix before requesting the next Ready. This allows a Ready
to contain concurrent writes without adding a batching timer. Metadata planning,
configuration changes and existing direct proposal APIs keep their current
paths; their interleaving with queued work is still decided by the Raft peer.
The Raw validation/read-barrier path and blocking RPC dispatch remain in place.
This change alone does not establish a throughput improvement.

Each owner permits at most 128 reserved requests and 64 MiB of encoded command
bytes. A turn inspects at most 64 requests, including cancelled tombstones, and
stops after reaching a 1 MiB encoded-byte target. One legal command can exceed
the turn target. Count/byte limits include popped requests until their actual
submission call returns or unwinds. A caller returning or dropping its ticket
does not release an in-progress reservation.

These limits cover queue/handoff encoded commands, not total RSS, Raft-log
retention, upstream decode/encoding temporaries, metadata work or public backend
occupancy. Existing public admission remains an independent outer bound.
Cancelled queue entries keep capacity until inspected or closed. A retained
suffix wakes the existing owner. Blocking storage can delay service; a bounded
number of inspections is not a bound on wall-clock turn duration.

## State and transitions

Each admitted request owns a non-cloneable ticket, one encoded command, an
absolute deadline, an optional expected term and one reservation. The queue
mutex orders stop against claim. The request mutex orders cancellation against
claim. The lock order is queue then request. Neither guard is held while calling
Raft. The driver's existing pump gate supplies the single-drainer premise.

| Event | Required state | Result |
|---|---|---|
| Admit | Queue open, deadline future, count/bytes available | Reserve and append `Queued` |
| Caller deadline | `Queued` | Atomically set `Cancelled`; definite `Expired` refusal |
| Ticket drop | `Queued` | Atomically set `Cancelled` |
| Owner inspection | Cancelled, expired or stopped before claim | No Raft call; discard reservation after inspection |
| Owner claim | Live `Queued` | Set `Claimed`; retain reservation through submission |
| Caller deadline | `Claimed` | Return unconfirmed; do not cancel or free the handoff |
| Submission return | `Claimed` | Publish its exact result once, then release reservation |
| Submission unwind | `Claimed` | Publish unconfirmed, then release reservation |
| Close | Queue open or stopped | Stop admission; refuse queued suffix; preserve claimed handoff |

Waiting checks an already published result before testing the deadline. Thus a
late observation can return a known result; it does not reset a deadline or
authorize submitting an expired queued request. Raw replacement retries retain
the original absolute deadline and occur only after the existing exact receipt
proves replacement. Unconfirmed submission/apply errors are not replayed.

Incoming transport messages are stepped before the queue is consumed. Every
claimed request then checks current leadership and its optional planning term
under the same peer lock as `RawNode::propose`. Its returned `(term, index)` is
only a local submission position. The existing exact apply-receipt path, fence
adjudication and durable Ready/engine ordering still determine write success.

## Safety argument and pending formal obligations

The following is a source-mapped inductive argument, not a completed
machine-checked proof or Rust refinement proof. For an arbitrary finite admitted
set, let `R` contain requests whose reservations have not been dropped. The
ledger invariant is `count = |R|` and `bytes = sum(encoded_length(r), r in R)`,
with count/bytes below their configured limits. Admission extends both ledger
and `R` under one queue lock after checking both limits. Claim, cancellation,
caller timeout and stop leave a claimed reservation in `R`. Exactly one owned
reservation drop removes a request and its bytes. Weak queue references avoid a
cycle; final queue destruction refuses its queued requests even if tickets live
longer than the owner.

Only `Queued -> Claimed` authorizes a Raft call, and the single owner performs
that transition once. Cancellation and claim share a request mutex, so they
cannot both win. Stop and claim share the queue mutex. A definite queue refusal
therefore excludes Raft submission by that request. If claim wins first, a
deadline or later stop cannot establish absence of an effect; the outcome stays
unknown until an exact submission result is known. Submission success still
requires the separate apply predicate before any successful public write reply.

With a live fairly scheduled owner, finite callback execution and a finite
admitted prefix, each owner turn removes a bounded nonempty prefix. New requests
append behind it; cancelled entries consume inspection budget. Thus an admitted
request is eventually inspected, refused or claimed. This conditional queue
progress statement does not prove consensus progress under partition, storage
stall, scheduler starvation or process loss.

Before runtime promotion, mechanize this arbitrary-request safety/progress
argument and its composition with exact submission, completion publication,
original Ready durability and Raw group application. Cover callback failure,
driver poisoning, recovery and publication ordering explicitly. Existing proofs
for predecessor candidates do not automatically discharge this new composition.

## Public certainty and observability

Only an exclusive `kv9-proposal-refused` marker with gRPC `RESOURCE_EXHAUSTED`
proves this request did not reach submission. Its five exact values are
`request_count`, `encoded_bytes`, `request_too_large`, `expired` and `stopped`.
Duplicate, mixed, unknown or binary `kv9-*` control metadata fails closed.
Unmarked status codes or prose prove nothing about a write's effect. A read
cannot carry this proposal marker. Persistent clients return a terminal refusal
without automatic replay; claimed submission timeout remains an unknown write.
Range deletion retains its existing partial receipt if an earlier chunk applied.

The CLI prints `proposal_refused=true reason=<value>` for a validated refusal.
The history parser requires exit 1, empty stdout and an exclusive complete
marker line (plus the known optional Kubernetes exit trailer). The persistent
workload reports a separate `proposal_refused` population. The independent
validator accepts exactly the legacy 12-population vocabulary or the new
13-population vocabulary, requires matching widths and rejected latency counts,
and rejects proposal refusals in read histories. Legacy artifacts remain checked
under their declared vocabulary.

Status exposes `raft_proposal_queue_*` limits, current queued/in-flight/encoded
occupancy, peaks and stopped state. These are queue ledger observations, not a
replacement for process memory, latency or public admission measurements.

## Local evidence and remaining acceptance

The initial workspace run passed 626 Rust tests/doctests with 23 ignored tests.
The candidate adds 12 queue behavioral tests, four actual-driver/peer integration
tests and two wire/client tests. Queue tests include controlled callback unwind,
stop after claim, caller drop, and 256 cancellation/claim plus 256 stop/claim
races. Integration checks cover admission while the actual peer mutex is held,
dequeue-time leadership/term checks, exact receipts with paused apply, and a
real malformed committed command that poisons the owner and refuses its suffix.
Holding the peer mutex is a contention test, not a physical device-failure test.

The independent history suite passed 45 tests and the new report controls passed
three tests. `scripts/check-proposal-queue-controls.py` retains baseline, compiled
mutant and restored runs for eight intentional violations: cancellation bypass,
false definite claimed timeout, early reservation release, unbounded cancelled
inspection, ignored turn-byte limit, ignored request/byte capacity and admission
after stop. Exactly one selected test must pass, fail at its intended assertion,
then pass again; compilation failures do not count as a detected violation.

The frozen implementation `95fb5cd968411009a41ba4f7ed287cd6597115fa` also passes
all-target warnings-denied Clippy and fresh default-build process acceptance.
The default executable SHA-256 is
`e132277a03212e18ec65cdf927390d788f67aa4ad5ad0dbde932b2230463e70c`;
Cargo reports no enabled features. Four executing server lifetimes were checked
through `/proc/PID/exe` and PID start ticks, and all exited. The three-node
fixture writes after leader failover, replicates deletes/range deletes and
restarts the original node to term 3/index 14. Retained root:
`/tmp/kv9-proposal-queue-process-first`.

The same default server and a retained exact-revision persistent client passed
`scripts/workload-e2e.py` with actual owned MinIO: correctness (180 measured
operations), graceful performance stop (9), a dead initial seed (179), rejection
of an invalid phase report, and database progress after killing the generator.
The correctness/dead-seed full histories were independently checked. Every
replica produced a remote checkpoint. Final queue occupancy was zero; the
observed leader peak was four requests/603 encoded bytes, so this small fixture
is not an overload acceptance test. Twelve intentionally corrupted report
artifacts were rejected at their intended reasons and all three complete
original reports were revalidated. Workload/container cleanup was checked.
Retained roots are `/tmp/kv9-proposal-queue-workload-e2e-first` and
`/tmp/kv9-proposal-queue-workload-report-controls-first`.

The local evidence archive is
`target/correctness-evidence/2026-09-09-95fb5cd-proposal-queue-local.tar.gz`:
267 entries, 85,962,607 bytes, SHA-256
`978236da98ca46ad0bb59fa11b88004076b733e9a078e44b23e11df12be50791`.
Its manifest SHA-256 is
`155d85db761aacfb3830b7d606161a0fc326b1be786354b653aef68e929e42fe`.
Every entry was read back and byte/hash verified. The archive includes frozen
source, exact executables, original failed/successful local attempts, process
stores and identities, functional workload evidence and all eight compiled
control triples. Originals remain available.

Remaining gates include actual exact candidate Chaos Mesh injection with
independently checked histories, adversarial MinIO/recovery acceptance,
parameterized proof composition, and repeated disk/tmpfs/Redis comparisons with
the existing complete outcome populations. No local unit test or short shared
host benchmark closes those gates. The queue is in memory; loss before
acknowledgement may leave an unknown invocation. A replacement leader recovers
through the unchanged consensus/storage path. No new coordinator or
service-critical singleton is added, but this does not establish cross-host
availability for the candidate.
