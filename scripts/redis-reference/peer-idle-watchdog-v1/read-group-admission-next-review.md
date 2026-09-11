# Read-group admission: retained arithmetic and next design constraints

The 16 reported endpoint deltas are correct. All 32 named input files matched their SHA-256 values; each three-voter delta and maximum matched the derived JSON. Before/after writer identity (PID/start/boot plus root/store), monotonic unsaturated admission counters and the retained serial-fresh drained boundaries also agree. This is a bounded readback, not a rerun of the full matched audit. Source review uses accepted 5ee; rejected f62 is not promoted.

| Envelope scope | Repeat 0 members/group | Repeat 1 members/group | Pooled members/groups |
|---|---:|---:|---:|
| c64 old-point | 2.033251623 | 2.033739649 | 2.033498311 |
| c64 old-batch1 | 2.010704918 | 2.011546669 | 2.011126278 |
| c64 new-point | 2.062466134 | 2.058242646 | 2.060353410 |
| c64 new-batch1 | 2.034208491 | 2.036001667 | 2.035104667 |

All eight c1 rows are exactly 1.0. Only voter 2 contributes positive group/member deltas in these endpoints; recorded maximum group size is 64 at c64 and 1 at c1. Each envelope's member count exceeds its recorded measured-call count by 8,322. These are before-setup/client to fresh post-client counters, including initialization, warmup and verification; no timed-only group ratio, group-size distribution, packet count or causal latency attribution follows. A member is a barrier request; BatchGet is one key in this workload. No new runtime/instrumentation was used.

## Current source semantics and concrete hazards

AsyncReads reserves at most 128 requests. One owner drains at most 64 queued entries, excludes canceled/expired requests, seals the remaining prefix before calling ReadIndex once, and uses the first member's unique 24-byte context as the immutable group key. Deferred admission may regroup requests while retaining identities and absolute deadlines. Confirmed groups retain live members until unified apply covers the exact first confirmation, or individual members cancel/expire. Their success is selected only after the entire pump iteration succeeds. The new idea may postpone a future seal; it must never append a later caller to an already-issued quorum request.

A cap on simultaneously unconfirmed groups needs an explicit definition: active_groups includes confirmed groups waiting for apply, so using that count would implement a different policy. Count in-progress admission consistently; refund deferred/error/stop paths, and release confirmation credit exactly once. A confirmed group can release unconfirmed credit while still retaining its own request reservations and apply fence. Immediate c1 admission means no added dwell timer when credit is available, not bypassing an already-full cap.

There are TWO current self-notification paths to fix: AsyncReads::submit notifies for retained uninspected suffixes, and NodeDriver::step_observed notifies whenever queued!=0 and peer.read_admission_can_progress(). A committed leader makes the latter true even if new group credit is full. Leaving it unchanged would cause a self-sustaining busy loop. An actionable-work predicate must distinguish admissible live work and bounded cancellation/expiry cleanup from live queued work blocked solely on credit. Do not short-circuit the whole pump: inbound confirmations, ticks, persistence/apply, shutdown and role errors must continue. Confirm/release during a turn must make waiting work actionable before parking, preserving WorkSignal's publish-before-notify and begin-turn-before-drain ordering.

Cancellation and role handling are substantive design decisions. complete currently cleans only active members; canceled queued entries are reclaimed during submit. If credit gating skips all submit work, queued reservations can remain stranded despite ticket Drop's wake. Preserve bounded cleanup even while admission is blocked. Removing one representative cannot free a still-live group's credit or change its confirmation key. Removing the last member removes a registry group, but does not retract its already-submitted ReadIndex from Raft. Therefore “bounded unconfirmed groups” must explicitly mean live registry groups, or separately account for retired yet outstanding contexts; repeated cancellation must not silently defeat the promised bound. This review does not infer an unbounded Raft-internal population without inspecting that implementation.

Role changes also cannot strand queued callers behind old-term credits. peer.read_admission_can_progress deliberately returns true on a follower so read_index can produce typed NotLeader. A new credit gate must preserve that terminal path, current-term commit deferral and original deadlines; lifecycle invalidation/credit reset needs explicit term/owner scope and must not make an old acknowledgement authorize a new group. Request destruction still must occur outside the registry lock, since Drop reacquires it to release reservations.

## Minimum focused evidence for an implementation

1. With a withheld real first-group confirmation, admit only the declared credit count; accumulate later readers without joining the sealed group. On exact confirmation, seal their bounded fresh prefix and emit a distinct ReadIndex. Old/foreign/duplicate/member-only contexts cannot authorize the later group or double-release credit. Preserve immediate singleton admission when credit is free.
2. Use the existing WorkSignal park observer to prove the owner actually parks while only credit-blocked live work remains. Incoming acknowledgement/cancellation/role change must wake it; ticks and transport delivery must continue. Mutating either self-notification gate should reveal unwanted busy work, while removing credit-release notification should reveal a parked queued request. Pending futures alone do not prove absence of a spin.
3. Cover representative cancellation, last-member cancellation, independent queued/member deadlines, full 128 request capacity, deferred ReadIndex, callback error and owner stop. Verify bounded cleanup, no context reuse, no leaked request/credit and the explicitly chosen retired-context policy. Do not reset caller deadlines to buy batching time.
4. Hold apply after confirmation: a successor may use released unconfirmed credit, but the first group cannot succeed before its own unified watermark. Failed Ready/application must still close reads without publishing success. Retain the exact barrier and later-snapshot tests.
5. Exercise leader-to-follower and new-term/uncommitted-leader transitions with credit already occupied. Retain typed refusal and prompt ready-queue progress after current-term commitment; no stale group may block the new owner indefinitely. Adapt the existing two-group heartbeat/late-reader test to controlled credit release, preserving its fresh-confirmation assertions.

Safety can be argued as postponing permitted admission transitions and forming only fresh sealed memberships, with unchanged exact-context/quorum/apply success guards. That argument requires a separate credit conservation/lifecycle lemma and an eventual eligible-queue-service obligation; liveness is not inherited merely by calling the change scheduling-only. Cap size may trade away useful pipelining, increase queue delay/refusals, or worsen tails. The small observed average is a reason to measure this hypothesis, not approval of an unimplemented policy or evidence of its benefit.

## Input bindings

- Derived endpoint record: `/tmp/kv9-peer-idle-watchdog-comparison-preparation/read-group-amortization.json`, SHA-256 `fb3f6c86e48be1433394a33c5bf365b7a583a29a8b97a494b1e117d27f1aaab1`.
- Its referenced prior matched audit hash is `b23a9ae4ac1e639f834d3e7fb999e55ff9c0bdb6673763d141c6c30ebb84f343`; full audit was not repeated here.
- Accepted 5ee crates/raft/src/async_read.rs: `7cae8d4bd986ff60d20a22d3f04dc4d39721f80f68d489a5ea98eefa32eb020f`
- Accepted 5ee crates/raft/src/driver.rs: `f2832061ad65c28b73f25757dcfe87fb4f990630ccd04043326ce5d3d4d01f63`
- Accepted 5ee crates/raft/src/work.rs: `766fe563ceba1f2e398ce14704b9fc232be3b4fa52b0301783036f95f089164a`
- Accepted 5ee crates/raft/src/rawnode.rs: `a2f19979dacc1942a760e2fe46fd82183fe4fbfbcde2d3fe6de6fe087efcdee3`
