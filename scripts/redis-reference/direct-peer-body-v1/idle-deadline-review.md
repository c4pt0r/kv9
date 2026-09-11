# Idle-deadline watchdog design review

Conditional design approval; no concrete cancellation or lost-wakeup blocker was found in the stated transformation. This reviews a proposed follow-on to frozen 6707bcccf15ea788ff231f4263f43a9f73fa63dd, not an implemented candidate. No production edits, Cargo, runtime, timing or tests were run.

When an empty queue is inspected at time t under the queue mutex, retain an idle check deadline t+B (B remains three seconds). Any subsequent enqueue establishing a still-relevant backlog timestamp e is ordered after that observation, so t+B <= e+B. Registering/polling the timer after unlocking does not lose an earlier deadline: an already-due sleep becomes ready when scheduled. At the check, inspect current token, closure and P under the mutex. If backlog exists, use its actual P+B and either expire or wait to that deadline. Valid progress may move P later; queue emptying makes this a recheck, not a stall. Thus removing producer Notify does not add a policy interval to the deadline of a continuously stalled backlog. Scheduler delay, timer granularity and lock contention remain the existing conditional limits, not a three-second real-time theorem.

Necessary implementation details:

- Sample idle now under the same queue mutex as the empty predicate. Preserve the initially selected idle Sleep across ordinary future polls; a poll alone must not reset it to now+B.
- An idle timer firing only causes a locked recheck/rearm. Returning Session::stalled merely because that timer fired would reconnect healthy idle peers. Keep the direct already-expired nonempty predicate and current-P recheck after timer readiness, so stale timers cannot expire newly progressed work.
- Keep lifecycle notifications and Notify registration before inspecting ownership/closure. Sender EOF, receiver drop and token invalidation must remain promptly observable without waiting for the idle timer. Preserve independent watchdog polling, per-RPC token fencing, one live waiter and Body waker ownership. The producer still sets P only on empty-to-nonempty admission and still wakes Body outside the mutex.

The performance claim must be narrower than “remove per-message supervisor wakes.” In 6707, producer Notify occurs only on empty-to-nonempty transitions, and Notify may coalesce events. The proposal removes that explicit producer-to-watchdog wake edge. Body/HTTP2/RPC completion, lifecycle events and periodic idle checks can still wake the peer owner. Healthy idle queues add approximately one watchdog timer wake per three seconds per active RPC under ordinary scheduling. Whether the removed edge matters to measured GET latency or throughput is unmeasured; the rejected 6707 result does not establish its cause.

Five focused production-backed test groups and controls:

1. Arm separate counting wakers for idle Body and Session::stalled, enqueue normally, and require Body wake but no direct watchdog wake. Inspect before timer expiry or any extra scheduling await. A source control restoring producer changed.notify_one must fail this specific assertion. Retain the original waker and lifecycle tests.
2. Keep Body unpolled, arm watchdog while empty, enqueue at a real later timestamp, then require eventual expiry near actual P+B within a declared scheduler allowance. Cover enqueue before first watchdog poll and just after an idle observation. The no-idle-timer/changed.await mutant must fail. Tokio test-util is absent from the pinned workspace: use a bounded real three-second wait or a narrowly justified test seam, not an invented production clock relationship.
3. Keep one watchdog future alive across more than two idle intervals; require no completion or token churn. At an idle deadline, drain/progress racing the check must revalidate current P or remain idle. Controls that treat an idle check as expiry or blindly use an old timestamp must fail. Retain the existing valid-dequeue budget-reset test.
4. With an idle timer armed and Body retained, sender/receiver closure and token invalidation must wake/terminate the correct waiter promptly; stale Body Drop must not affect a replacement. Abort-before-first-poll, same-route and A/B/A tests remain unchanged in intent. Removing a lifecycle notification should fail a short closure test, not only a long timeout.
5. Repeated producer sends/stale-only inspection/irrelevant notifications must preserve a continuously nonempty P. Preserve the existing semantic P comparison and non-poisoning assertion marker; retain its enqueue-resets-deadline mutant. Expiry-before-select must still terminate an overdue backlog despite repeated ready notifications.

The old idle_session_does_not_expire_and_first_queued_work_wakes_watchdog test explicitly requires an immediate owner wake and then artificially backdates P before the already-armed idle observation. Both parts need an honest scope update: the immediate wake is intentionally removed, and demanding expiry from that impossible e<t history would not test the new argument. Do not silently retain the backdated post-enqueue fixture while weakening its timeout. Backdated timestamps remain useful in tests that start the watchdog with an already-nonempty queue, where the real predicate reads that timestamp directly.

No core Raft authorization/confirmation/apply transition is intentionally changed. The argument is local conditional source reasoning, not a machine-checked scheduler/transport proof or performance acceptance. Existing real frozen-reader/blackhole and route/cancellation gates remain relevant after implementation; this review runs none of them.

Reviewed control source SHA-256 values:

- crates/raft/src/grpc/direct_body.rs: `8de391879f192afbeca9321b147bf1058301cd45b2b8d07e58791dc4bf229ef6`
- crates/raft/src/grpc/direct_body/tests.rs: `a23cf0eae30b4415273527d2fbd9b84c0fb606fb72196be1d6f63562176df47b`
- crates/raft/src/grpc.rs: `94678473f675a3e2f352f280511986bec4b6c370551718f7ae1523d3b498cb37`
