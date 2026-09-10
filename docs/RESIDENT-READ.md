# Resident point-read execution

This document records the original point-read candidate. The later native batch
extension, optional resident borrowing contract and captured-view byte-budget
fallback are specified in [ASYNC-BATCH-READ.md](ASYNC-BATCH-READ.md).

Tracking: #20 and #9. This candidate extends sealed groups `2cbbe26`.
Its purpose is to remove the remaining blocking-pool handoff from uncontended
memory-index GETs. It changes neither quorum confirmation nor acknowledgement
or persistence ordering. Performance selection requires a fresh comparison;
the earlier grouping result is not a measurement of this implementation.

## Execution contract

The concrete `RuntimeBackend` owns a `WalEngine`, whose live index is a
`MemEngine`. The new inherent `try_resident_snapshot` methods expose that
specific implementation's memory-only view. The generic `Engine` and `ReadView`
contracts are unchanged: arbitrary engines do not gain permission to execute
blocking operations on an async worker. The default `RawApi` preparation still
returns a blocking job.

A public GET retains its existing admission reservation and first awaits the
same bounded, owner-serviced quorum/apply barrier. The post-barrier continuation
then tries the metadata lifecycle mutex. Contention transfers the original
barrier result, context and key to the blocking job. A successful try copies
only the bootstrap state, releases the guard, and applies the same Serving plus
endpoint-ready predicate. Lifecycle errors retain their precedence over a
failed barrier. There is no lifecycle guard held over a view or callback.

After a successful barrier and lifecycle check, the continuation tries the
memory index read lock. It clones the three persistent-map roots and releases
the guard. A contended try captures no view and returns the unconsumed barrier
to the blocking path. The WAL mutex, Raft peer mutex, driver state-machine mutex
and device I/O are absent from this resident path. The resulting owned snapshot
uses the existing memory-map read implementation and stays fixed across later
atomic batches, including batches spanning column families.

Both paths pass the same view to the same `check_context_in` gate and then to
`RawExecutor::get`. Metadata keyspace, range-index and region-row lookups all
use the caller-owned `MetaTxn`; they do not open another snapshot. The successful
`LeaderRead::new(view, true, hint)` implementation never uses `hint`. Passing
`None` here removes a full `driver.status()` query whose return value was
unused; it does not replace a leadership check. Typed follower/refusal hints
still originate from the unchanged quorum barrier.

`RawReadJob` is now an enum. `Completed(value)` means the actual context-checked
read has already finished. The public handler returns the value without a
blocking dispatch and finishes the same reservation. `Blocking(job)` transfers
that reservation into the existing worker, including when the RPC is canceled
while the job is queued or running. A canceled quorum wait still destroys its
preparation without starting an engine job. No await occurs between the final
resident view selection and its read/result construction.

A Completed value can be returned after a later epoch change because its read
already finished on the earlier authorized view. A queued Blocking job checks
the lifecycle and the current post-barrier view when it actually executes; an
old request epoch is then refused. Neither representation can turn a completed
old read into a new read of post-change data. Batch reads, scans, writes and
administrative operations retain their existing execution boundaries.

## Conditional refinement argument

Fix an arbitrary finite execution. Assume the existing quorum/apply barrier
contract, atomic persistent-map publication, and the same-view context gate.
These premises include the sealed-group freshness and Ready durability
contracts; this argument does not independently establish them.

For each request, let its ownership state be `Waiting`, `Barrier`, `Blocking`,
`View`, `Completed`, or `Terminal`. An unsuccessful barrier is an error value,
not a `ReadBarrier`. A successfully minted barrier is non-Clone/non-Copy and
remains owned by this request until exchanged for one view. The transitions are:

| Transition | Credential/view effect | Admission ownership |
|---|---|---|
| Invoke -> Waiting | No view or successful credential | Public preparation owns reservation |
| Waiting -> Barrier | Unchanged quorum/apply algorithm produces credential or error | Same preparation |
| Lifecycle contention -> Blocking | Transfer original credential/error; capture no view | Same reservation transfers with worker submission |
| Index contention -> Blocking | Return original successful credential; capture no view | Same reservation transfers with worker submission |
| Successful index try -> View | Consume credential for one owned post-barrier snapshot | Same preparation |
| Blocking execution -> View | Recheck lifecycle, consume credential for one post-barrier snapshot | Worker owns reservation |
| View -> Completed | Gate context and read data from exactly this view | Preparation or worker retains reservation until work finishes |
| Error/cancellation -> Terminal | Create no successful read; destroy owned work | Release only when the actual owning work ends |

Induct on these transitions with four invariants:

1. Each live operation has one reservation owner. Preparation has no detached
   engine work. Worker submission transfers the sole reservation into the job;
   cancellation cannot release that job's reservation. Completed has no remaining
   engine work. Existing reservation destruction performs the sole release.
2. A request has at most one successfully captured view. It starts with none;
   only the two constructors can capture one, both consume its unique credential,
   and neither is reachable after View/Completed. Failed tries capture no view
   and transfer, rather than duplicate, the credential.
3. Every captured view follows that request's successful quorum/apply barrier.
   The constructors require its credential; error values cannot inhabit this
   argument. Deferring the constructor cannot move it before the barrier.
4. Every successful value is read under the same view used for its context gate.
   The common helper transfers the owned view through `begin_at`, the gate and
   `into_view`. It opens no new view and performs no engine read after completion.

The empty state establishes these invariants. Invocation changes only ownership;
barrier completion adds a credential but no view. Both contention transitions
preserve all four by transferring without observing the engine. Constructor
success establishes 2–3. The common gate/read helper establishes 4. Errors,
cancellation, completion and destruction create no additional credential/view
or engine work, preserving the invariants.

Erase failed try operations and worker scheduling transitions. Each successful
resident execution then has the same semantic sequence as an allowed existing
read execution: invocation, quorum/apply barrier, lifecycle check, post-barrier
snapshot, context gate, data read and response. Moving this finite sequence into
the continuation changes scheduling, not its observation contract. A value
computed before an overlapping epoch change remains an allowed old-view result;
a later invocation must establish and gate its own view. Thus, under the stated
premises, successful resident and fallback executions preserve the existing
point-read safety contract. Removing the unused status hint is observationally
irrelevant because the constructor's `true` branch returns the view for every
hint. This proof does not claim equality of wall-clock error timing, panic
containment or performance between the two schedules.

Progress remains conditional on a live quorum, eventual apply, fair executor
scheduling and finite storage/engine work. A contended try takes no wait/spin
loop; the blocking fallback has the existing progress assumptions. A request
gets one resident attempt in production, not repeated polling for an unlocked
index. No new cluster-wide service or service-critical singleton is introduced.

Machine-checked composition and exact-candidate fault/history acceptance remain
promotion gates. Earlier group/async fault evidence retains its original source
scope; it is not acceptance for this later execution path.

## Bounds and observations

The existing public request/encoded-byte limits and 128-request async registry
remain unchanged. The resident path takes an owned O(1) persistent-root clone,
then finite point/routing lookups and value copying; it does not claim constant
CPU time, zero allocation, bounded allocator latency or whole-process RSS.
Large values still cost CPU/memory to materialize. This change adds no response
byte quota and does not establish a latency SLO under unbounded offered load.

`public_raw_get_completed_inline` counts successful values returned as Completed;
`public_raw_get_blocking_submitted` counts blocking jobs handed to dispatch.
Preparation failures count in the existing backend-error metrics, not the
inline-success counter. New counters saturate and never control capacity,
identity, eligibility or correctness. They are cumulative observations, not
atomic measurement-window populations across replicas.

Tests exercise actual resident completion with an occupied sole blocking worker,
non-waiting writer contention, owned cross-CF snapshot stability, committed but
unapplied writes, lifecycle-contention fallback, both epoch components, old-view
completed results and fresh stale-epoch refusals. Existing public cancellation
and reservation tests continue through the blocking variant. The compiled
control runner rejects waiting index acquisition, dispatching a completed read,
discarding a contended request and bypassing the resident context gate; each
requires a passing baseline, the named semantic assertion failure and a passing
byte-restored source. A build error or timeout is not accepted as a control.

Full local validation, exact default three-process execution, fresh performance
measurement and candidate-specific protocol/fault acceptance are tracked
separately. Routine checks run locally; no hosted CI is dispatched.

Initial local gate: 630 workspace tests/doctests passed (23 ignored), and
warnings-denied workspace/all-target Clippy passed. All four compiled
baseline/mutant/restored triples passed. The first workspace run exposed an
obsolete assertion expecting 13 status lines; the two new counters make 15.
That failed run is retained, and the complete corrected run passed. The controls
preceded only that status-line test assertion update; production source is
unchanged between the control run and the corrected workspace run.
