# Coalesce physical Raft-owner notifications

This candidate changes `WorkSignal::notify` to call `Condvar::notify_one` only
on the pending bit's false-to-true transition. The original code already
coalesces work logically into that bit, but still calls the condition variable
for every producer while it is true. The new guard retains the same mutex and
publication order; it introduces no polling loop, batching delay or queue.

The direct parent is `57ff6851e40ed63c837189d6eb0a11190704725a`. The separate
inbox-vector candidate is not included. This isolates notification coalescing
from message representation changes. Performance benefit remains unmeasured.

## Local proof and assumptions

Let P be `pending` and S be `stopped`, both protected by the same mutex.
The owner may park only while not P and not S. `begin_turn` clears P before
the owner reads any work queue, and returns false if S is set. Producers
publish their actual work before acquiring the signal mutex. `stop` sets S
and always broadcasts. None of these transitions changes.

There is at most one production waiter on a WorkSignal: `NodeDriver::spawn`
claims the background pump once, and the other production signal operations
do not call `wait_until`. Concurrent manual `step` calls serialize through
the pump gate and do not become additional condition-variable waiters. The
change is not a general replacement for a condition variable shared by several
independent consumers. `CompletionSignal`, which wakes multiple receipt
waiters, is unchanged.

For every producer notification, the protected logical state is identical:

| State on acquisition | Original transition | Candidate transition |
| --- | --- | --- |
| S is true | No change, no wake | No change, no wake |
| S is false, P is false | Set P, wake one | Set P, wake one |
| S is false, P is true | Keep P, wake one | Keep P, no additional wake |

Only the last case needs justification. If the sole owner is processing work,
P remains set until its next `begin_turn`; its intervening parking predicate
therefore prevents sleeping. If it has not started waiting, the same predicate
prevents parking. If it was already parked when P became true, the earlier
false-to-true transition has already notified that sole waiter. Later skipped
wakes add no runnable owner. The standard mutex/Condvar atomic unlock-and-wait
contract prevents a notification from falling between the predicate check and
waiter registration.

Induction over protected signal transitions gives the same P/S observations
and stop decisions. Queue publication before notification, combined with
clear-before-drain, ensures an accepted producer's work is either consumed
by the current turn or covered by a retained pending turn. A producer delayed
between publication and notification can create a redundant later turn after
its work is already consumed; that remains harmless in both versions. Multiple
producers do not overwrite P, and a notification during drain survives it.
Spurious wakeups recheck the unchanged predicate. Stop still broadcasts and
cannot be reversed by later producers.

The argument assumes eventual owner scheduling and mutex acquisition, the
existing one-owner contract, and correct producer publication. It adds no
stronger liveness guarantee, deadline bound or scheduler-fairness claim. The
signal remains a hint, not evidence of Raft quorum, persistence or application.
Peer stepping, read-group membership, ReadIndex admission, successful-pump and
applied-index fences, cancellation, tick deadlines and write acknowledgment
authority remain unchanged. This is a local source-level refinement argument;
it does not close the outstanding whole grouped-read/Ready/Rust proof work.

## Validation and measurement boundary

Existing tests cover pending work across drain/park, stopped owners, bounded
inbox prefixes, delayed ticks, and real parked-owner read/proposal/cancellation
wakeup paths. The additional test holds an actual inbox owner after its empty
drain, publishes four messages from independent producers, then lets it check
the parking predicate. Complete message delivery must occur before a distant
tick; cleanup stops and joins a faulty parked owner before asserting failure.
It checks delivered work rather than the implementation's notification count.

Local validation passes **219 Raft tests/doctests**, formatting and
warnings-denied all-target Raft Clippy. The new test also rejects a deliberately
lost pending turn: a control clears P on a repeated producer notification and
the actual-delivery test fails with its bounded receive timeout (exit 101).
The exact original source is restored and that targeted test passes (exit 0).
The baseline package gate already includes its successful execution; it is
not repeated to manufacture another baseline. Original control output and
both source versions are retained at
`/tmp/kv9-work-signal-coalescing-control-first`.

The independent source/test review finds no local blocker under these stated
premises. Fewer condition-variable calls do not eliminate the mutex acquisition
per producer or prove an end-to-end gain. Historical scheduling/futex CPU
samples motivate the experiment but are neither current latency attribution
nor a predicted speedup. Default release, process E2E and a separately declared
matched screen remain required before selecting a performance increment.
Candidate-specific Chaos acceptance is a separate subsequent gate. All checks
remain local; broader formal composition is not claimed by this local proof.
