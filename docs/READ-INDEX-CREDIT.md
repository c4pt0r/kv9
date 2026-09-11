# ReadIndex admission against the actual pending queue

This worktree reapplies the isolated read-credit change from `57ff6851` to the
selected byte-table CRC source `ca0002c7`. The earlier implementation and its
historical validation below used parent `5ee897a`. Those earlier recordings
retain that source scope; they do not establish fresh performance, recovery or
Chaos acceptance for this combined source. The candidate remains unselected.
Tracking: #9 and #20.

## Current CRC-baseline reapplication

The fresh Raft package run passes 218 tests/doctests. The current full workspace
passes 716 tests/doctests with 23 explicitly ignored tests; formatting and
warnings-denied all-target Clippy pass. All 14 compiled implementation-control
triples pass with exact baseline/mutant/restored exits 0/101/0 and the intended
semantic assertion. Copied source inputs and final restoration are checked.

The control runner now accepts an explicit `--cargo-target` for a compiler cache
owned exclusively by that run. Its default remains a private output cache;
source/cache overlap and executable paths outside the selected cache are
rejected. This local run uses `/home/dongxu/kv9/target` sequentially, preserving
the same tests, source mutations and acceptance checks while avoiding another
private dependency build.

The unchanged formal gate freshly passes all 37 cases, including 28 distinct
theorems and 265 baseline obligations. All 13 formal/helper/config inputs and
the relevant read implementation are byte-identical to the previously accepted
`57ff6851` inputs. The current observations are retained in
`/tmp/kv9-read-credit-crc-workspace-first`,
`/tmp/kv9-read-credit-crc-controls-first` and
`/tmp/kv9-read-credit-crc-formal-first`, with their original invocation and
terminal records. The single-invocation proof scope below remains unchanged.

No source-specific release, process/Chaos recovery or performance result is
inherited from the earlier candidate. Those gates remain pending for this
reapplication; the historical results below retain their original source scope.

The accepted control's retained c64 endpoint spans show about 2.03 GET members
per admitted group, including setup, warmup and verification. This motivates
testing whether one outstanding quorum request lets later queued readers form
larger groups without a fixed batching timer. It does not establish that
ReadIndex grouping causes the full performance gap with standalone Redis.

## Admission and ownership

`RaftPeer::read_index` checks leadership, current-term commit and the pinned
raft-rs 0.7.0 `pending_read_count()` under the same peer lock as submission.
Local admission defers when the actual pending count is at least one. Both
synchronous and asynchronous callers use this entry point. A deferred call
submits nothing and retains its original context and absolute deadline.

The count belongs to Raft's pending read queue, independently of caller or
registry lifetime. Canceling the last member cannot refund an outstanding
protocol request. An acknowledgment or configuration-quorum reevaluation that
advances the queue, or Raft's reset on a role/term transition, releases that
protocol occupancy. A singleton may immediately produce a ReadState without
entering the pending queue. A confirmed group
waiting for local apply no longer consumes this capacity; its members still
need exact confirmation, apply coverage and a successful whole pump.

This is an atomic local-admission policy against actual upstream occupancy.
It is not a universal bound on messages injected by authenticated remote peers:
upstream handling of a remote `MsgReadIndex` does not call this wrapper. Such
occupancy also prevents further local admission. There is no new independent
credit registry, cancellation refund or term counter to synchronize.

## Freshness and progress argument

The existing [sealed-group contract](READ-GROUPS.md) remains in force. Every
member invokes before the actual admitted quorum request starts. Deferral
allows regrouping only before that start. Later invocations cannot join an
already admitted group or consume its quorum confirmation, even if the
eventual read indexes are equal.

The new condition removes some otherwise admissible local submission
transitions; it does not create confirmation or completion transitions. Under
the existing uniqueness, Raft ReadIndex, apply and view-gate premises, this
preserves the sealed-group safety argument. Those premises are not proved by
the capacity check. Machine-checked group/ReadIndex/Ready composition and
source refinement remain open promotion requirements.

The owner still performs bounded queued cancellation/deadline cleanup before
trying admission. A full protocol slot must not cause an early return before
that cleanup. Deferred submission already suppresses its own retained-prefix
notification. The driver's final queued-work notification now also consults
the capacity-aware peer predicate, so a full slot alone cannot drive an idle
notification loop. Inbound acknowledgments, cancellation, ticks and shutdown
remain independent work sources.

The driver processes inbound messages before trying queued admission. A real
acknowledgment can therefore release the slot and admit a fresh group in the
same turn. Followers remain eligible for an admission attempt so queued
callers receive `NotLeader` promptly. Current-term readiness and capacity are
rechecked inside the actual submission lock, independently of the wake hint.
An idle c1 reader starts immediately when the slot is free.

Progress requires fair owner service and eventual quorum communication; finite
success latency is not promised during a partition. When all callers for a
pending context cancel, ordinary Raft heartbeat retransmission and reset remain
responsible for clearing it. A lost acknowledgment does not justify credit
release based only on caller lifetime or elapsed time.

Nor does weakly fair polling guarantee each caller a slot when other callers
can refill it indefinitely. A conditional success claim needs persistent
availability or an explicit allocation-fairness premise, in addition to enough
caller budget. The synchronous path retains its original deadline check after
failed admission; this change does not prove a new absolute prohibition on
admission after a wall-clock deadline.

## Machine-checked scope

[ReadCredit](../proofs/tla/read-credit/ReadCredit.tla) extends the unchanged
single-invocation ReadAdmission model with an occupied predicate and caller
cancellation. The occupied predicate abstracts the real upstream queue, with
both possible post-admission states to cover immediate singleton confirmation.
It is an overapproximation, not a counter-conservation proof or an implementation
refinement of the entire pending queue.

The [parameterized proof](../proofs/tlaps/read-credit/ReadCreditProof.tla)
projects every transition to an original admission transition or stuttering.
It derives the original safe-return theorem, checks the free-credit admission
precondition, and proves that cancellation and deadline expiration do not
release occupancy. The separate eventual-admission theorem requires a ready,
stable leader, positive remaining budget, no cancellation, no competing refill,
and fair upstream release and polling. It proves admission in that window,
not complete read success or a wall-clock latency bound.

The bounded model includes current-term commit before its stable window. Its
fair-commit premise prevents an initial uncommitted stutter from standing in
for the missing-credit-release counterexample. All complete model recordings
require exhausted state queues, nontrivial exploration and the exact expected
counterexample or completion verdict. Semantic controls remove the credit
guard, refund credit on cancellation, omit release fairness, or admit competing
refills. The latter two must refute unconditional progress. Property definitions
remain unchanged across each action-only mutation.

The runner reuses the existing SANY dependency/assumption/hole audit and freshly
checks both the original admission proof and the new credit proof under the
pinned TLAPS toolchain. It also rejects an omitted proof and an added axiom.
No new quorum-certification axiom is added. Upstream certification remains an
explicit model boundary, and grouped-read/Ready/Rust composition remains open.

```sh
python3 scripts/check-read-credit-protocol.py \
  --jar /path/to/tla2tools-v1.7.4.jar \
  --tlapm /path/to/pinned/tlapm \
  --output /new/read-credit-proof-output
```

## Required validation

The initial local library run passed 185 Raft tests with five new credit tests
(`/tmp/kv9-read-group-credit-focused-first`). Two additional synchronous tests
now cover the shared capacity gate, fresh confirmation and original deadline.
The current full Raft package run passes 218 tests/doctests: 187 library, one
driver-lock, 13 bad-frame, five membership and 12 documentation tests. All-target
Clippy passes with warnings denied. All 14 implementation-control triples pass
with exact 0/101/0 exits and intended semantic failure markers. Frozen bindings
and complete logs are retained in
`/tmp/kv9-read-group-credit-source-validation-first` and
`/tmp/kv9-read-group-credit-async-controls-first`.

These are local correctness checks. They supply no new performance measurement,
whole-system formal composition or Chaos Mesh acceptance.

The default workspace subsequently passes 714 tests/doctests with 23 explicitly
ignored tests, plus warnings-denied workspace/all-target Clippy. The first
workspace recording is `/tmp/kv9-read-group-credit-workspace-first`.

The complete formal gate passes 37 retained cases at
`/tmp/kv9-read-group-credit-formal-third`: 28 parameterized theorems and 265
fresh obligations (15/196 inherited admission, 13/69 new credit), four semantic
model/proof control triples and two semantic-audit triples. Two fingerprints
agree at each bound: 832 and 2472 distinct safety states, and six conditional
progress states. All positive queues are exhausted. The counterexamples retain
the violated property, state trace and fair-commit prefix; an error, empty run
or unrelated failed obligation cannot count as the expected defect.

Earlier attempts remain preserved. The first proof draft left six obligations
unproved because its proof steps did not unfold the required state definitions;
the next draft added those definitions without weakening theorem statements.
The first complete gate then rejected the TLC wrapper's tuple-based initial
assignment, fixed by explicit assignments. The second gate rejected a one-state
initial stutter under the existing two-state counterexample requirement. The
final wrapper includes the actual current-term commit transition and fair
commit before the stable window; it does not lower that requirement. No failed
attempt is presented as acceptance of its original source.

| Retained record | SHA-256 |
| --- | --- |
| Raft source validation result | `f5a785c1d987ed6eb96b88371964047879051873c5da63e2a6c8bb083c9178d3` |
| Compiled control manifest | `ef0ed0ea5c1b3f3394018375b928a6c8a8d66d04ded9288e47104615ac063876` |
| Workspace result | `4337d458d8c394c7d63d57d581982e149911ba40dc120464b0c4d52969565a61` |
| Complete formal summary | `81345fcb3d7da8a4ea0694dda18b2f5149a3af3a5fb1a247af8f014d51e522c2` |

Real three-voter tests must withhold acknowledgments, cancel the last submitted
member, clean queued cancellations at full occupancy, park the actual owner,
wake it with an acknowledgment, hold apply after confirmation, and change
leadership. Late reads must use a distinct context and remain pending on an
earlier group's acknowledgments. Synchronous admission must share the gate.

The existing per-member-broadcast mutation now reaches the cap on its second
callback, before it can emit an extra heartbeat. The real grouped-read test
therefore additionally checks that its single admitted group owns all three
members. The control requires that earlier semantic failure; the original
heartbeat-count and late-context assertions remain. Compilation failure or a
different failure marker is not acceptable evidence.

Before selection, complete source controls, formal checks, process histories,
matched c1/c64 throughput/mean/p99 screening, and source-scoped Chaos Mesh
acceptance. Increasing group size can introduce queueing and hurt latency;
the mechanism remains an experiment until those results support selection.
