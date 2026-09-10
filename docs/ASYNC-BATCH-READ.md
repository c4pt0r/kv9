# Asynchronous native batch reads

Tracking: #9, #20 and #50. This candidate extends native client/benchmark
revision `3bd1751bddf9a85b07447b64a12adf28ed0df8a9`. It addresses a measured
baseline gap: native `BatchGet`, including batch size one, still used a blocking
read barrier and worker handoff, while point GET already supported asynchronous
barriers and resident completion. A new comparison is required to quantify the
effect. The earlier point and native-batch results are not a matched regression
experiment.

## Execution and bounds

`RawApi::prepare_raw_batch_get` defaults to the existing blocking implementation.
The concrete WAL runtime awaits the existing `read_barrier_async` once for the
whole batch. It shares lifecycle checking and resident snapshot acquisition with
point GET. Raft quorum confirmation, apply catch-up, write acknowledgement and
WAL persistence rules are unchanged. There is no lease or follower-read bypass.

The async continuation is eligible only for at most 256 keys with a combined
user-key length of at most 1 MiB. Larger low-level requests keep their existing
blocking execution; these are scheduling limits, not new wire rejections. After
capturing a resident view, the continuation validates the request context and
every key's region in that view. It then borrows the raw values without copying
them. `RawExecutor::try_batch_get_resident` counts each result position, including
duplicate keys, against a 1 MiB value-copy budget using checked subtraction.
Only a fully successful budget pass may clone any value bytes.

`ReadView::get_resident` is an optional contract: no I/O, no waiting on locks,
no value copying, and the exact immutable value that `get` would return. Its
default declines the optimization. `MemSnapshot` implements it by borrowing from
its owned persistent map. Absent and present-empty values remain distinct.
The borrowed `ReadView` forwarding implementation lets the existing consuming
metadata gate inspect a temporary wrapper without relinquishing the owned view.

There are two different blocking transfers:

| Reason | What the job owns | Observation when it executes |
|---|---|---|
| Lifecycle/index contention before capture | Original unconsumed barrier result, context, keys | Check lifecycle, capture one post-barrier view, validate and read that view |
| Value budget exceeded or borrowing declined after capture | The already authorized owned view, context, keys | Check lifecycle, read the same captured view without another barrier or snapshot |

In the second case a later epoch/value change cannot change the result. In the
first case an epoch change before the eventual snapshot is checked against that
later snapshot and may reject the old request. Both results follow the existing
same-view read contract. Dropping an unsubmitted job drops its view or credential.

The public handler uses the same reservation lifecycle as point GET: preparation
owns admission while awaiting the barrier; a completed result needs no blocking
worker; a submitted blocking job retains admission through cancellation until
actual work ends. Separate `public_raw_batch_get_completed_inline` and
`public_raw_batch_get_blocking_submitted` counters preserve the existing point
counter meaning. Saturating counters never control capacity or correctness.

These limits bound resident key lookup/copy work, not allocator latency, metadata
size, total blocking-result allocation, whole-process RSS, or latency under
unbounded offered load. Existing stream response limits apply after execution;
they do not replace the before-copy scheduling budget. There is no new response
memory quota in this change.

## Conditional safety refinement

Assume the existing Raft quorum/apply credential contract, atomic persistent-map
publication, correct same-view context gate, and immutable resident/get
equivalence. These are explicit premises, not conclusions of this scheduling
change. Fix an arbitrary request, ordered key vector `K`, and finite execution.
Let `V` be its captured view and let `G(V, K)` be the ordered vector obtained by
reading each encoded key in `V`, retaining missing values and duplicates.

Maintain the following invariants:

1. Before capture the request owns at most one successful, non-cloneable barrier
   credential. A failed try transfers it; it does not consume it or create a view.
2. Capture consumes that credential once and yields exactly one post-barrier
   view. No transition out of the captured state invokes another constructor.
3. Authorization, budget probing and all returned values use this same `V`.
   Borrowing a wrapper changes ownership of a reference, not snapshot identity.
4. A successful result is exactly `G(V, K)`, and the request retains one admission
   owner until its actual preparation or submitted work ends.
5. If resident materialization begins, the sum of returned value lengths,
   counting every position, is at most 1 MiB. A declined probe copies no values.

Initially no credential, view or result exists. Barrier success establishes only
credential ownership; a barrier error cannot become a credential. Lifecycle
errors precede barrier errors as before. Contended lifecycle/index acquisition
preserves invariant 1 by transfer. Successful resident or blocking capture
establishes invariant 2 under the barrier premise. Context validation borrows or
consumes only that captured view, establishing invariant 3.

For the budget loop, after the first `i` positions, its remaining budget equals
the original budget minus the sum of those positions' borrowed value lengths.
The empty prefix establishes this equation. Checked subtraction preserves it
for each next position or declines before copying. Duplicate positions take
the same transition again; missing/empty values subtract zero. Thus a completed
probe establishes invariant 5. Borrow/get equivalence makes the cloned vector
equal `G(V, K)`. A declined probe moves owned `V` to the worker, which computes
the same vector directly. Neither branch captures a later snapshot. This
establishes invariant 4 for both paths.

Cancellation before submission destroys only owned preparation. Cancellation
after submission leaves the reservation with the actual worker. Errors and
destruction create no successful result and no new credential or view. These
cases preserve the invariants, completing the induction.

Erase borrowing, failed tries and worker scheduling from a successful execution.
The remaining sequence is invocation, a quorum/apply barrier, one post-barrier
snapshot, authorization in that snapshot, ordered reads from that snapshot, and
response. It is an allowed existing atomic batch-read execution. Deferring
materialization cannot alter the linearizable observation because `V` is owned
and immutable. Epoch changes overlapping the request may occur after its
authorized read view; new requests still establish and validate their own views.

This is a source-level conditional refinement argument. It is not a
machine-checked refinement of all Rust, protobuf, stream ownership, or allocation
behavior. The native-batch TLA+/TLAPS work on proof revision
`d31fc44` retains its original abstract scope and explicit Raft/engine premises;
it does not by itself verify this new adapter. No new consensus algorithm or
cluster-wide singleton is introduced. Progress still depends on a live quorum,
eventual apply, fair scheduling and finite engine work.

## Acceptance scope

Focused tests cover one barrier per ordered batch, duplicate/missing/empty
values, commit-before-apply waiting, both epoch components, completed old-view
results, uncaptured contention, and oversized captured-view fallback. Public
tests cover an occupied blocking pool, preparation cancellation, and admission
retention after blocking submission. Engine/raw tests cover immutable borrowing,
unsupported views and exact duplicate-inclusive budget boundaries.

Candidate-specific process histories, actual Chaos Mesh acceptance, and matched
performance results must retain their exact source and artifact identities.
Earlier native-batch fault results remain evidence for their recorded revisions.
This document alone does not promote the candidate or claim a QPS improvement.
