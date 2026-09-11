# Peer watchdog: bounded idle checks without producer notifications

This experiment follows rejected direct-body source `6707bcc`. It removes the
explicit producer-to-watchdog notification on empty-to-nonempty enqueue. The
producer still wakes the request body and sets the first backlog timestamp.
The accepted performance control remains `5ee897a`; no throughput or latency
benefit is claimed before a matched comparison. The underlying queue, per-RPC
ownership, batching, cooperative budget and Raft checks are unchanged from
`6707bcc`.

## Source contract

`grpc/direct_body.rs` retains two distinct notification mechanisms:

- The body waker is taken under the queue mutex and invoked after unlocking
  on enqueue, sender/receiver closure and active-session invalidation.
- `Shared::changed` wakes the independent watchdog on lifecycle changes and
  outage discard. Enqueue no longer notifies it.

At each watchdog iteration, lifecycle notification interest is enabled before
reading state. Under the queue mutex, the watchdog checks RPC ownership and
sender closure, captures monotonic time `t`, and chooses a deadline `D`:

| State | Deadline and action |
| --- | --- |
| Invalid token, or closed sender with empty queue | Return immediately |
| Nonempty queue with progress timestamp `P` | `D=P+B`; return if already due |
| Empty queue | `D=t+B`; this is an idle check, not a stall deadline |

`B=STREAM_PROGRESS_BUDGET` remains three seconds. The chosen sleep is retained
across ordinary future polls. A lifecycle notification restarts the locked
inspection. A timer completion rechecks current ownership and `P` before it
can report a stall. In particular, empty-queue timer expiry only rearms the
check. The watchdog is polled by the peer owner, independently of HTTP2 polling
the request body.

## Conditional deadline argument

Use the queue invariants and mutex linearization points proved in
[DIRECT-PEER-BODY.md](DIRECT-PEER-BODY.md). In particular, `P` is absent exactly
when the queue is empty. Repeated sends and stale inspection leave an existing
`P` unchanged. A valid dequeue with backlog remaining moves `P` to the current
monotonic time; draining/discarding clears it. Opening a replacement RPC gives
that RPC its own budget and fences the previous owner.

Consider a still-active session and an iteration observing an empty queue at
time `t`. The queue predicate and clock sample share one critical section. Any
subsequent first enqueue establishing backlog time `e` is ordered after that
section. Monotonicity gives `e>=t`, hence `D=t+B<=e+B`. The already-selected idle
check is due no later than the new backlog's deadline. If timer registration
is delayed until after `D`, its first eligible poll is ready; registration
does not move the deadline.

At that check, either the queue is empty again, ownership has ended, or the
watchdog observes the current `P`. In the last case it returns if `P+B` is due,
otherwise arms that actual deadline. Valid progress or a drain followed by
new enqueue can only make the relevant timestamp later than the timestamp
covered by the earlier observation. Thus an old selected check remains early
enough; it cannot require an additional full policy interval after observing
a continuously stalled backlog.

The same reasoning applies when a previous iteration observed a nonempty queue:
its timer was armed at that backlog's `P+B`. Progress, emptying and new enqueue
may move the current deadline later, but cannot make the old timer late for the
new deadline. The current-state recheck prevents that early timer from declaring
newly progressed work stalled. An already-expired backlog is checked before
`select`, so repeatedly ready lifecycle hints cannot defer the decision by
winning selection indefinitely.

Lifecycle changes retain prompt wakeup: registration precedes the locked
predicate, so a preceding change leaves a notification permit and a later one
wakes the registered owner. The actual peer-worker path has only one live
watchdog waiter per queue. Cancellation drops the session guard; stale body
polls and drops remain fenced by the unchanged per-RPC token under the mutex.

This is a policy-deadline argument under monotonic clocks, timer service,
cooperative/fair scheduling and finite local operations. It does not establish
a hard three-second wall-clock bound under scheduler starvation or mutex
contention. Progress still means valid local dequeue, not delivery, quorum
confirmation or application. A frame retained inside HTTP2 after the queue
empties is covered by the existing keepalive/subsequent-traffic behavior.

## Validation obligations and scope

The changed behavior requires replacing the old test that demanded an immediate
producer wake and then backdated `P` behind an already-armed idle observation.
That synthetic `e<t` history violates the premise above. Preserve actual
enqueue timestamps for the new empty-to-nonempty timing coverage. Existing
tests that start with a nonempty queue may still backdate `P` to exercise an
already-expired predicate.

Focused tests must distinguish body and owner wakes, cover real enqueue before
and after idle arming with an unpolled body, keep one future alive across
multiple idle intervals, preserve its timer across ordinary polls, and check
prompt closure/invalidation. Retain current-timestamp revalidation, stale-traffic
age, ownership, queue bounds and cooperative-yield tests. Compiled source
controls must reject producer notification, absent idle checks, false idle
expiry and lost lifecycle notification by their intended assertions.

This document is a source-mapped deductive argument, not a new machine-checked
model or whole-system refinement. No core consensus transition, fresh read
confirmation, applied-index fence, exact write result, authentication rule or
admission limit is modified. Local tests, source controls, process recovery,
matched performance and actual Chaos Mesh acceptance remain separate gates.
Results must bind the final source; inherited `6707bcc` validation is historical.

The expected tradeoff is one fewer explicit producer notification per empty-to-
nonempty burst, with notifications already subject to coalescing. Healthy idle
sessions now perform approximately one timer check per three seconds under
ordinary scheduling. Other RPC/body/lifecycle events can still wake the owner.
This is neither elimination of all owner wakes nor measured attribution of the
previous candidate's regression.

## Local source gates

The implementation passes **228 kv9-raft tests**: 197 library tests (including
20 direct-body component tests), 19 integration tests and 12 documentation
tests. Clippy passes for all kv9-raft targets with warnings denied. The first
logs and exact source hashes are retained in
`/tmp/kv9-peer-idle-watchdog-validation-first`.

The control runner retains the preceding nine route/body/TCP controls and adds:

| Injected source fault | Required rejection |
| --- | --- |
| Enqueue notifies the idle owner | Separate owner counter observes an unwanted wake |
| Empty queue disables the idle sleep | Real subsequent backlog misses its unchanged deadline plus observation allowance |
| An idle timer treats absent `P` as stalled | Persistent healthy idle future requests reconnect |
| Sender closure omits its lifecycle notification | Armed watchdog receives no prompt closure wake |
| Timer uses its old deadline instead of current `P` | Valid dequeue cannot protect newly progressed backlog |

Each control requires baseline success, one compiled mutation failing its named
assertion with the ordinary test-failure exit code, and restored-source success.
The runner hashes production and the split-out test source for all phases.
The first recording at `/tmp/kv9-peer-idle-watchdog-route-controls-first`
passes all **14 triples / 42 compiled executions**, with exact 0/101/0 exit
sequences and the intended assertion for every injected fault. No first-run
rejection or runtime retry was needed for these source gates.
