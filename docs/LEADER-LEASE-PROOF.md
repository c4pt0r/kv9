# Leader lease reads without a per-read quorum round trip

Tracking: [#9](https://github.com/c4pt0r/kv9/issues/9), #20.
This is a conditional algorithm proof and an implementation contract. The
selected runtime `11113f6` still uses Safe ReadIndex. This document does not
enable lease reads, qualify a clock platform, or establish a performance result.
The subsequent [transition model and inductive proof](LEASE-AUTHORITY-MODEL.md)
make acquisition, renewal, revocation, recovery and local read steps executable.

**Claim.** A leader with a correctly acquired, unexpired lease can serve
linearizable reads using only local operations after request arrival. Acquiring
and renewing the lease still requires a quorum; each individual read does not.
The clock assumptions below strengthen Raft's asynchronous safety model. The
write protocol, durable acknowledgements and state-machine ordering remain Raft.

With fully asynchronous, unbounded clocks this claim is impossible: an isolated
old leader cannot distinguish a run with no replacement from one where the
other quorum elected, wrote, and then received a new read at the old leader.
If its clock can stop, identical local observations cannot authorize a correct
local answer in both runs. The proof below therefore does not promise Raft's
timing-independent read safety after the clock assumptions are violated.

## Assumptions and the proposed protocol

1. Nodes are crash/recovery, non-Byzantine participants. Raft's election safety,
   monotonic durable terms/votes, leader completeness, committed-log agreement
   and ordered application hold. Successful writes acknowledge only after the
   existing durability/application requirements. Network messages may be lost,
   reordered or delayed without a latency bound; they cannot forge identity.
2. During one usable lease, its granting quorum `Q` intersects **every** quorum
   allowed to elect a competing leader. A fixed voting configuration with
   strict majorities satisfies this. Learners do not grant voting authority.
   Changing to a non-intersecting configuration is outside the fixed-configuration
   proof; joint configurations and transitions require an additional protocol.
3. For real times `u <= v`, each relevant clock satisfies
   `a(v-u) <= C_i(v)-C_i(u) <= b(v-u)`, where `0 < a <= b`.
   A common notation is `a=1-rho`, `b=1+rho`, `0 <= rho < 1`.
   Clock offsets need not agree. These are rate bounds over elapsed real time,
   including pauses, rather than an assumption that NTP timestamps are equal.
   Quantization, sampling error and arithmetic rounding must be conservatively
   accounted for before a concrete implementation instantiates the inequalities.
4. A grant for `(group, configuration, leader incarnation, term, round)` is
   accepted only in that exact term and configuration. The grantor has not
   already voted in a higher term. From the grant at real time `g_q`, it refuses
   every competing higher-term vote and self-vote until its clock advances by
   at least `E` units. This is a **voting promise**, not just postponing its own
   campaign. Higher-term traffic, timer reset, recovery, transfer and membership
   handling cannot silently erase the promise while the lease remains usable.
5. The promise survives a grantor's crash/restart through durable state plus a
   suitable clock, or a conservative recovery quarantine. A recovering leader
   starts with no usable lease. Reused addresses, delayed replies or restarted
   processes cannot reuse an old round/incarnation certificate.
6. Before lease reads are admitted in term `e`, the leader has committed an
   entry in term `e`. Reads retain the existing successful publication, apply,
   immutable-view and same-view metadata/epoch checks. A local revocation
   generation serializes term/configuration/transfer invalidation with final
   read validation; it cannot wrap or exhibit an ABA reset.

The leader samples its clock at real time `s` **before sending** a renewal
round. A voter establishes the promise before acknowledging that round, at
`g_q >= s`. Only exact, successful acknowledgements from a valid quorum activate
the certificate. The leader stores deadline `C_L(s)+D`, choosing

```text
0 < D <= E*a/b
```

Production must round the bound downward and reserve its proven clock/sampling
error margin. An acknowledgement received after this deadline activates no
usable lease. Out-of-order renewal replies cannot rebind or extend another
round's start time. Each certificate is proved against its own granting quorum;
votes or acknowledgements from unrelated rounds cannot be added together.

## Lemma 1: the leader stops using the lease before any grant expires

Let a final lease check succeed at real time `t`, after all acknowledgements
used in the certificate. For every `q` in `Q`, `s <= g_q <= t`. The successful
check and slowest permitted leader clock imply

```text
a(t-s) <= C_L(t)-C_L(s) < D <= E*a/b
therefore t-s < E/b.
```

The fastest permitted grantor clock then gives

```text
C_q(t)-C_q(g_q) <= b(t-g_q) <= b(t-s) < E.
```

Thus every grantor's voting promise is still active at `t`. This proof needs
no bound on network RTT. Delayed grants and responses consume lease usability;
they cannot extend it. Equality at the leader deadline is rejected. For example,
with an ideal one-second voter promise and `rho=100 ppm`, the leader duration is
at most approximately `999.800 ms`, before implementation margins. This is an
illustration, not a selected production timeout or a measured clock guarantee.

## Lemma 2: no higher-term leader can already have been elected

Suppose, for contradiction, another leader in term `e' > e` has been elected
by time `t`. Let its election quorum be `V`. Pick `q` in `Q intersect V` and
let `v_q <= t` be when `q` cast the actual higher-term vote used by that election.
An election certificate is about when votes were cast, not when their replies
were delivered.

- If `v_q < g_q`, the durable monotonic term/vote at `q` prevents its later
  successful grant for the lower term `e`. Such a reply is not a valid grant.
- If `g_q <= v_q <= t`, Lemma 1 says the promise is still active at `v_q`.
  Its voting rule forbids the higher-term vote.

Both cases contradict the assumed election. Raft's one-leader-per-term property
excludes a different leader in term `e`. Earlier-term leaders cannot commit a
new conflicting prefix against the higher-term quorum, by Raft's ordinary log
and term rules. Applying this argument to each usable certificate also excludes
overlapping usable leases for distinct leaders. Merely observing `role=Leader`
or `check_quorum=true` establishes none of these lease premises.

## The read algorithm and linearizability argument

After a read invocation, the leader performs the following local steps:

1. Under the authority gate, capture an active certificate, its term/configuration/
   revocation generation, and the **current** commit index `c`. Verify the
   current-term commit fence. Do not cache `c` at renewal for later reads.
2. Wait until successful local application/publication covers `c`. Acquire one
   immutable state-machine view of an exact committed prefix through index `j`,
   with `j >= c`. Validate the request's metadata, range and epoch on that same
   view. An applied watermark without the corresponding view is insufficient.
3. After view acquisition, validate under the authority gate that the same
   incarnation/term/configuration/revocation generation still authorizes the
   read and that the clock is strictly before the certificate's deadline.
   The time sample follows acquisition of that gate. On failure, discard this
   fast-path authority and use the existing fresh Safe ReadIndex path or return
   its typed deadline/error. Preserve the original invocation budget.
4. Read only from that retained immutable view and return. The view can be
   queried after validation because its contents cannot change. An unrelated
   fresh view cannot be substituted after the lease expires.

**Fail-closed service rule:** once the lease has expired, no new successful
local read is authorized by it. A read may proceed only after acquiring a new
valid lease or completing a fresh Safe ReadIndex and its apply/view fence.
If the node cannot obtain the required quorum confirmation, it returns typed
unavailability or the original request's timeout. It must not return cached
data, an empty success, or extend the deadline locally. Writes still require
Raft quorum commitment. Contact with one peer is useful only if the resulting
voting set is a valid quorum; merely reaching a peer establishes no authority.

This rule is necessary but does not replace the clock, vote and restart
premises: without them another leader could already exist while the old leader
incorrectly believes its lease is live. A response delayed after the successful
final validation can still be linearizable because it uses the retained view
and the operation overlaps the later change; network delivery time is not a
new authorization event.

Every write that completed before the read invocation belongs to a committed
prefix known through `c`: leader completeness and the current-term commit fence
cover earlier terms, and the active leader's commit frontier covers its own
completed writes. Lemma 2 excludes an unseen newer leader committing completed
writes before this read's valid check. Therefore each such write has index
`i <= c <= j`, and the returned view includes it with all later effects in that
prefix. Conversely, a write invoked after the read response cannot already be
in this view. The snapshot contains only a committed prefix.

For a precise history construction, first include the pending writes whose
committed entries appear in returned views, completing those pending operations
in the history extension permitted by linearizability. Other pending operations
may be omitted. Order writes by their agreed log positions and place each read
immediately after its view's prefix `j`. Reads at the same prefix are ordered
by their real-time precedence. The following four facts establish every possible
kind of real-time edge:

- A write completed before another write began has an earlier log position,
  by Raft's ordered proposal/commit and leader-completeness guarantees.
- A write completed before a read began has position at most `j`, by the fresh
  frontier argument above.
- A read completed before a write began has `j` smaller than that write's
  position: a not-yet-invoked write cannot already be in the retained view.
- If one read completed before another began, the latter's prefix cannot be
  smaller. On the same leader its fresh frontier covers the previously observed
  prefix; across leaders, leader completeness and the new term's commit fence
  preserve that already committed prefix.

Thus the resulting total order extends the real-time order and each operation
returns the state-machine result at its position. This is linearizability.
The argument does **not** fix each write's linearization point at the instant
its bytes first reach a physical majority. An entry might already be on a
majority while its leader has not processed the acknowledgements and the write
is still pending. A concurrent read can legally precede that pending write.
Consequently, the leader's locally known `commit_index` must cover earlier
completed operations; it need not equal an omniscient observer's physical
replication frontier at every instant. The physical snapshot time also need not
be the read's linearization point.

The final check conservatively fences arbitrary **process scheduling pauses**
when the clock continues to meet its rate bound. A pause between an initial
lease check and view acquisition may cross expiration; this protocol rejects it.
This is a sufficient protocol rule, not a claim that every such overlapping read
would otherwise violate linearizability. An alternative protocol may bind a fresh
per-invocation authority and commit frontier before the pause and prove an earlier
linearization point. In particular, a write completed during a paused read overlaps
that read and does not alone prove a stale-read violation. A pause
after final validation can delay the response, but the retained snapshot and
its valid linearization point remain inside the invocation/response interval.
A paused or rolled-back clock violating Assumption 3 is a different failure and
is not repaired by a second check of that same faulty clock.

## What is removed from the critical path

An already valid certificate supplies Lemma 2 for any number of later reads
whose final checks meet Lemma 1. Those reads perform local authority, application,
snapshot and lookup operations. None of the four read steps sends or awaits
a quorum message. Background heartbeat/append acknowledgements can renew the
certificate when they implement the grant contract. Initial acquisition, an
expired lease and a leader change can still require quorum communication.
An application backlog can still make a read wait locally. This proof gives
zero **per-read consensus RTT** on the valid fast path; client/server RTT,
queueing, lookup cost and write quorum/durability work remain.

Lease length trades renewal overhead against failure recovery latency. A lost
leader can serve local reads only until its proved deadline; a new leader must
respect the corresponding promises. After finite expiration a surviving quorum
can elect a leader under Raft's ordinary progress assumptions. No distinguished
machine or centralized clock service is introduced. This is conditional
availability, not a bounded recovery-time theorem for an indefinitely delayed OS
or network.

## Necessary boundaries and explicit counterexamples

| Omitted condition | Counterexample |
| --- | --- |
| Send-time anchor | With unit-rate clocks and `E=D=10`, grant at time 0 expires at 10. Its ACK arrives at 9. Starting the leader deadline at ACK time permits a read at 12, after a new leader elected at 10 has completed a write at 11. |
| Bounded clock rate | A leader clock stops while it is partitioned; other nodes' promises expire and they elect and write. The old clock still says the lease is valid. A new read there is stale. |
| Voting promise, including self-votes | A node that suppresses its own timeout but grants a higher-term candidate's vote can help elect a replacement during the old lease. |
| Historical term check | Higher-term votes can already be in flight before a lower-term grant. Blocking only future votes does not invalidate those votes or their later election result. |
| Promise preservation on reboot | A grantor restarts, forgets its promise and votes with a third node while the isolated old leader's lease is still usable. |
| Fresh commit/apply/view fence | A write completes at index 10 after renewal at index 8. Reading the renewal-era prefix 8 returns stale data even with an exclusive leader. |
| Per-invocation authority | A worker caches a successful lease Boolean, pauses past expiry, then uses it for a newly arriving read after a replacement leader has completed a write. This new read cannot linearize before that write. |
| Transfer and configuration discipline | Forced election or a non-intersecting replacement quorum bypasses the promises; the intersection proof no longer applies. |

For one possible recovery rule, suppose every outstanding voter promise uses
duration at most `Emax`. Its remaining real lifetime is at most `Emax/a`.
A recovering voter with no trustworthy retained deadline can refuse grants,
votes and self-votes for at least `Emax*b/a` units on its fresh bounded-rate
clock; even the fastest such clock then waits at least `Emax/a` real time.
Restarting again cannot shorten this quarantine. This conservative rule needs
its own persistence/incarnation refinement and is not present in production.

Before a forced transfer, invalidate lease authority under the read gate and
prevent delayed renewals from restoring it; otherwise wait out the promise.
Fixed-membership proof does not itself justify membership changes, split ranges,
dynamic groups, remote follower reads or revoking a lease on a partitioned node.
Those extensions need explicit intersection and activation/revocation proofs.

## Current source gaps and implementation gates

Source inspected at `c2fc693`, using pinned raft-rs `0.7.0`:

- `crates/raft/src/rawnode.rs` selects `ReadOnlyOption::Safe` and
  `check_quorum=true`. raft-rs's `LeaseBased` read branch simply returns its
  committed index once its current-term fence passes. That configuration toggle
  is not this protocol's deadline or grant certificate.
- `TickDeadline` coalesces a long scheduling delay into one tick. The existing
  tick proof expressly supplies no real-time election bound. A paused owner can
  retain its logical leader role beyond an elapsed-time lease, so role/tick
  checks cannot substitute for an independently checked clock deadline.
- raft-rs's normal vote suppression is based on leader identity and elapsed
  ticks. A forced campaign has an exception. Current product surfaces do not
  expose the test-only transfer API, but a lease implementation must fence the
  underlying transition too. Durable term/vote storage does not by itself retain
  the additional time promise across restart.
- `NodeDriver::read_barrier_async` and the resident read path already establish
  separate quorum, successful pump, apply and same-view authority. A lease fast
  path needs a distinct, non-forgeable authority type and a source refinement;
  it must not fabricate a successful Safe ReadIndex confirmation.

The [fixed-configuration transition model](LEASE-AUTHORITY-MODEL.md) now specifies
acquisition, renewal, revocation/expiration, generation exhaustion, restart
quarantine and local read-view validation, with a parameterized inductive proof
and fault controls. Its [Rust component](LEASE-CONTROLLER.md) now has local source
tests and integer timing proofs. The next gate is unique peer installation and
actual voting/publication/read-view bindings with source refinement. Local Rust tests
must exercise delayed ACKs, stale rounds, application stalls, cancellation and
the check/snapshot/revoke races. Actual Chaos Mesh E2E must include bidirectional
and asymmetric partitions, delayed/reordered renewal traffic, process pause,
grantor restart, leader replacement and post-recovery linearizability histories.
Clock-bound violation tests must demonstrate disabling/refusal where detection
is guaranteed; no monitor can turn an arbitrary undetected stopped clock into
the bounded-clock premise. Host suspend/VM migration support requires a proved
platform contract or disabling this fast path on that platform.

Only after these correctness gates should matched local GET/mixed/batch
throughput, mean, p95 and p99 be compared with selected `11113f6` and Redis.
This stage does not dispatch hosted CI or close an original #9 checklist item.

## Mechanical proof scope and primary references

[`LeaderLeaseProof.tla`](../proofs/tlaps/leader_lease/LeaderLeaseProof.tla) supplies
parameterized lemmas for clock-horizon containment, delayed grants, historical
and future vote exclusion, intersecting election quorums, completed-write prefix
coverage and final expiry rejection. Its arithmetic specialization uses unbounded
integers with arbitrary scaling; the real-time inequalities and the full
history/linearization argument are proved above. The separate
[transition proof](LEASE-AUTHORITY-MODEL.md) adds acquisition/recovery/read actions.
Neither proof is a machine-checked Rust translation, clock qualification, or E2E
acceptance. Retained commands, audit, negative controls and
initial proof-development failures are linked in the validation record.

[`clock-containment.smt2`](../proofs/tlaps/leader_lease/clock-containment.smt2)
additionally expresses Lemma 1 over arbitrary **real** constants. Unsatisfiability
of its premises plus the negated conclusion proves real-time containment, beyond
the integer specialization. Removing the send-time order, permitting the leader
clock to stop, or omitting the drift margin each produces a retained satisfying
countermodel. This arithmetic result does not establish any physical clock's
compliance with the premises.

The premises are satisfiable: `a=b=1`, `E=D=10`, `s=g=0`, `t=1`, and both
elapsed-clock values equal to `1` satisfy every premise with an unexpired
promise. The real proof is not obtained by assuming an impossible lease.

The [retained validation record](leader-lease-proof-v1/README.md) contains the
fresh semantic audit, eight TLAPS lemmas / 23 obligations, six rejected faulty
lemma variants, two rejected proof-integrity controls, three output controls,
real arithmetic proof and three countermodels. It also retains the initial
parse failure, unsupported real-arithmetic draft, incomplete automatic proof
drafts and the successful explicit order-lemma decomposition. No assumptions
were replaced with the desired conclusion. The final real theorem is checked
separately with the pinned Z3 binary; the TLAPS arithmetic scope remains explicit.

Raft's original read protocol requires a committed entry from the current term
and fresh leadership authority. Its dissertation also describes amortizing that
authority with clocks and expiring a lease before leadership transfer.
See [Raft, Section 8](https://raft.github.io/raft.pdf) and
[Ongaro's dissertation, client interaction](https://github.com/ongardie/dissertation/blob/master/clients/clients.tex).
[TiKV's lease-read explanation](https://tikv.org/blog/lease-read/) gives a concrete
send-time-based renewal design. The pinned library documents the clock-drift
limitation of [raft-rs LeaseBased](https://docs.rs/raft/0.7.0/raft/enum.ReadOnlyOption.html).
These sources motivate the protocol; the explicit grant/restart assumptions and
proof above define this proposal's own boundary.
