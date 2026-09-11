# Two-context local ReadIndex admission window

This experimental change raises local pending ReadIndex admission from one
context to two on the CRC runtime. It follows `de37c710` plus the retained-build
cache correction `7fccd8ad`. It excludes diagnostic instrumentation and does not
change the selected mainline. No throughput or latency gain is established for
this candidate yet.

The current-source four-cell lifecycle observation explains the hypothesis.
In c64 point GET/PUT traffic, one-context admission changes sampled queue mean
from 19.893 to 82.557 us, while invocation-to-confirmation falls from 168.753 to
142.385 us. Apply and notification also shrink, but the sampled total grows
from 273.454 to 299.022 us. Admitted members per group increase from 3.144 to
10.241. These are successful-sample, whole-client-envelope intervals on
instrumented CRC/credit sources, not pure network RTT, isolated capacity wait,
measurement-only client latency or performance acceptance. Two contexts test
whether useful overlap can reduce waiting while retaining bounded admission.

## Unchanged read authority

Both synchronous and asynchronous local callers use `RaftPeer::read_index`.
The peer lock covers leadership, current-term commitment, actual raft-rs
`pending_read_count()` and submission. A count of two or more defers local
admission. The owner wake predicate uses the same threshold, then admission
rechecks it under the peer lock. No fixed batching delay, lease or stale read is
introduced.

Groups are still sealed before their invocation. A later group has a distinct
context and cannot reuse a previous group's already-observed confirmation.
Actual raft-rs acknowledgement/configuration reevaluation can confirm a prefix
of pending groups; each returned context still goes through the existing exact
group and applied-index checks. A successfully completed pump must cover the
confirmed index before the original ticket can return. The caller then obtains
and validates its read view. No acknowledgement or apply fence is removed.

Cancellation and deadline expiration stop caller observation but do not remove
an upstream context. They therefore do not refund protocol admission. Conversely,
confirmation can release protocol space while the original read still waits
for local apply. Election reset and singleton immediate confirmation retain
their original upstream behavior.

## Counted bound and its premises

`ReadWindow.tla` models a positive capacity C and pending count p. The initial
count is zero. A guarded admission requires p < C and changes the count by zero
or one, covering singleton immediate confirmation as well as a retained
context. Quorum/configuration release is nonincreasing; reset clears the count;
caller cancellation preserves it. Inductively, 0 <= p <= C: admission produces
p' <= p + 1 <= C, release cannot increase p, reset produces zero, and
cancellation/stuttering preserve the bound. `ReadWindowProof.tla` mechanically
checks this argument for arbitrary positive C. The source gate binds C=2 to the
Rust threshold and verifies both guard sites. TLC checks capacities two and
three; these numbers are capacities, unlike the older admission model's
budget/term configurations named `Credit2` and `Credit3`.

This is a counter abstraction under explicit source-mapping premises. It assumes
all queue-increasing transitions use the guarded at-most-one admission rule.
Authenticated remote `MsgReadIndex` currently enters through the separate
`step_message` upstream path and can bypass the local gate. Therefore this
theorem is not an unconditional bound on every possible upstream pending queue.
For any accepted local invocation, its locked pre-count is below two and its
at-most-one increment respects that threshold; arbitrary remote increases are
outside the always-bounded abstraction. No remote ingress change is included
in this experiment. A universal ingress bound remains separate work.

The original `ReadCredit` single-invocation projection remains applicable with
`rcOccupied` interpreted as the full-window predicate. Changes that remain on
one side of the threshold can stutter. Its safety and conditional admission
progress proofs do not prove per-caller fairness under competing traffic,
wall-clock latency, grouped-read/Rust/Ready composition or upstream Raft itself.
The counted model adds no such claims and provides no new quorum authorization.

## Verification and promotion

Run both `scripts/check-read-window-protocol.py` and the existing
`scripts/check-read-credit-protocol.py` with the pinned TLC/SANY and TLAPS tools.
The counted gate audits the dependency/theorem inventory, uses fresh strict
proof checks, and retains normal/defective/restored triples. Controls reject
admission at full capacity, double increments, cancellation refunds, omitted
proofs and an unapproved false axiom. A cancellation-refund defect can preserve
the numeric bound, so it has a separate action-property and theorem check.

Real Raft tests must exercise two outstanding contexts, third-group deferral,
selective confirmation, shared sync/async admission, cancellation/deadlines,
apply-held reads, quorum reevaluation, reset and owner wake behavior. Existing
compiled async-read controls remain required. Process recovery, exact-source
Chaos Mesh and matched uninstrumented c1/c64 pure/mixed read measurements are
separate gates; none can be inferred from the proof or diagnostic parents.

The first raw proof draft left the invariant opaque in the stuttering branch
and failed one of 23 obligations. Expanding its unchanged invariant/legal-input
definitions closes that branch; the initial failure and corrected fresh proof
remain retained. No semantic assumption or invariant was weakened.

## Local checkpoint, 2026-09-11

The uninstrumented candidate passes 718 workspace tests/doctests (23 ignored),
formatting and warnings-denied all-target workspace Clippy. Nine real Raft
admission tests include asynchronous expiration and committed configuration
quorum reevaluation. All 14 compiled semantic-control triples pass: each
baseline succeeds, the intended production mutation fails its exact assertion,
and restored source succeeds. All 42 selected Raft test units are freshly
compiled under exclusive ownership of the shared Cargo target.

The counted formal gate passes all 29 cases, including nine distinct theorems
and 23 baseline obligations. The existing admission gate passes all 37 cases,
with 28 distinct theorems and 265 baseline obligations. The two gates have
different abstractions; their counts do not establish a composed system proof.
Retained local roots are `/tmp/kv9-read-window-workspace-first`,
`/tmp/kv9-read-window-controls-first`, `/tmp/kv9-read-window-formal-first` and
`/tmp/kv9-read-window-admission-formal-first`.

Exact-source process recovery, actual Chaos Mesh and uninstrumented performance
remain pending at this checkpoint. No hosted CI was dispatched. The selected
runtime remains CRC; this commit publishes a reviewable experiment.
