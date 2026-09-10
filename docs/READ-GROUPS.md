# Sealed quorum-read groups

Tracking: #20 and #9. This candidate extends asynchronous preparation
`1ad259e78c148b0b6b8d837b142a2e9521b60b3f`, whose separate tmpfs comparison
improved GET throughput by 30.3%. That result is not a measurement of this change.
The current increment replaces per-request ReadIndex submission with one
submission for a bounded, sealed group. It retains each request's own deadline,
cancellation, admission reservation, and established engine view.

## Why group before Raft

In pinned raft-rs 0.7.0, the leader's `MsgReadIndex`/`ReadOnlyOption::Safe` arm
records a read context and invokes `bcast_heartbeat_with_ctx`. The read-only
queue tracks those contexts and may advance an acknowledged prefix. KV9's
existing transport coalescing combines already emitted envelopes into RPCs;
it does not eliminate those logical heartbeat broadcasts.

The new owner detaches at most 64 queued requests under the registry lock.
Canceled and expired requests count toward that inspection limit and are
released outside the lock. The remaining membership is sealed before the
single `read_index` callback. There is no timer or wait to fill a group.
An actual three-voter test checks the protocol effect directly: three reads
emit two contextual heartbeats, one per follower, instead of six. A later read
emits a new context and cannot complete on acknowledgments for the first group.

## Ownership and execution

One `ReadGroup` owns the sealed member vector and its first confirmed index.
The active map is keyed by the first member's checked invocation context. That
key lives independently of the member; cancellation or timeout of the first
member cannot destroy the group's confirmation routing while another member
is live. Only the exact group key is accepted as confirmation. Other members'
individual contexts are not interchangeable confirmation keys.

Context allocation remains shared with synchronous readers and refuses counter
exhaustion. A reserved-context set covers queued, claimed, active, and selected
requests, including callback ownership and unpublished selected completions.
Registration rejects a duplicate reserved context or a still-active group key.
Global process-incarnation/context uniqueness remains an explicit protocol
premise; the set is not a substitute for it across completed requests/restarts.

All sealed members become claimed before the callback. Stop immediately reaches
the unclaimed queued suffix and active groups; an already claimed group retains
its reservations until the callback returns or unwinds. This differs from the
earlier per-request callback shape: up to 64 members, rather than one member,
can belong to that one claimed call. The stop test blocks the first group and
requires the 65th request to fail without waiting for its callback. No registry
guard is held across Raft admission or request/sender destruction.

When admission returns `false`, the leader has not committed its current-term
entry and no quorum read has been initiated. Members return to the queue with
their original identities and absolute deadlines; a later retry may regroup
them before initiating a fresh read. A known follower refusal is propagated to
every member. Other peer admission failures remain terminal typed read failures.
An admitted group's members never grow. Requests registered during its callback
remain queued for a later group, even when both groups' eventual indexes match.

The first exact group confirmation sets every remaining member's quorum phase.
Completion selects members only after a successful whole pump and unified apply
coverage. A canceled/expired member can be removed while live siblings remain.
Selection is the existing completion eligibility point; a later stop may
preserve an already selected result. Every successful request still consumes
its own private barrier through the existing same-view region/epoch/data gate.
Sharing an index does not promise a shared snapshot or multi-key transaction.

## Conditional safety proof

Consider any finite execution. For request `r` and admitted group `g`, let
`Invoke(r)`, `Seal(g)`, and `StartReadIndex(g)` denote their event positions.
Membership is fixed before calling Raft, so every member satisfies

```text
Invoke(r) < Seal(g) < StartReadIndex(g).
```

The group key may have been minted before the final member arrived. Freshness
comes from starting the quorum read after sealing, not from the key's mint time.
An earlier refused/deferred attempt created no admitted read using that key.
Checked unique contexts prevent any earlier accepted quorum confirmation from
being confused with this group's confirmation.

Induct over the registry transitions with these invariants:

1. Every reserved request has exactly one storage owner: queue, claimed callback,
   active group, or selected completion. Its context stays in the reserved set
   until destruction. The reservation count equals that set's cardinality and
   never exceeds 128. Registration is the only increment and refuses at 128;
   moves preserve ownership; destruction performs the sole decrement/removal.
2. Active group membership is a subset of the membership sealed before its
   admitted callback. No transition appends to an admitted group. Cancellation,
   expiry, completion, and stop only remove members or transfer their ownership.
   Failed/deferred callbacks cannot authorize success; regrouping happens only
   before a later callback that initiates the actual read.
3. A group's confirmation changes only from absent to the first index matched
   by its exact key. Removing a representative changes neither key nor index.
   Thus every member receives evidence for its own admitted group, and every
   later invocation requires a distinct fresh confirmation.
4. A successful selected result requires a group confirmation `i` and a unified
   applied observation `a >= i` after the whole pump succeeds. Failure closes
   pending groups. A per-member timeout/cancellation never manufactures success
   for another member, changes its deadline, or releases its reservation.

The empty state establishes all four. Registration and prefix claim establish
the event ordering and preserve ownership. Filtering only removes requests.
Admission publishes exactly the sealed vector; its refused/deferred branches
publish no confirmation. Exact first confirmation establishes invariant 3.
Completion checks coverage before moving ownership out of the group; stop and
destruction add no success evidence. These cover every registry transition.

Assume the existing Raft ReadIndex protocol is safe for a request whose quorum
read begins after invocation, the unified applied watermark correctly covers
the established index, and the existing view/gate implementation preserves its
read contract. The event ordering above and invariants 3–4 supply those same
premises for each member individually. Consequently grouping preserves that
existing point-read safety contract. It does not re-prove Raft, storage
durability, group-to-Rust refinement, or the underlying Ready publication model.

Progress remains conditional on fair owner execution, a live quorum, finite
callbacks, and eventual apply. A member's cancellation cannot remove a live
sibling's group key. Registration/cancellation wake the owner; retained work is
bounded; deferred admission does not self-spin, and post-election readiness
wakes a retry without waiting for another tick. There is no finite completion
bound under partitions or failed storage.

Machine-checked composition of this group lifecycle with the existing ReadIndex
and Ready contracts remains a promotion gate. Actual Chaos Mesh and independent
public histories for this exact candidate are also required; earlier `1ad259e`
fault evidence must retain its original source scope.

## Observable bounds and validation

The original async status counters still count requests. Additional counters
report active groups, inspected requests, attempted/admitted groups, admitted
members, and maximum admitted group size. Cumulative diagnostic counters
saturate at their integer maximum and never participate in identity or safety
decisions. The limits are 128 reserved requests and 64 inspections/members per
group. Active completion scans at most that bounded request population. These
are not whole-process RSS or upstream Raft internal-memory bounds.

Local tests cover late registration during the actual admission callback,
representative cancellation, independent deadlines, full queue grouping,
bounded cancellation cleanup, blocked callback/stop, failed Ready, exact-context
and first-confirmation rules, and actual three-voter heartbeat/ack ordering.
The public-handler reservation and committed-unapplied/epoch product tests from
asynchronous preparation continue to run through this implementation.

The implementation-control runner adds compiled late-member, representative,
group-bound and per-member-broadcast defects to the previous seven controls.
Every control requires an exact named test, intended assertion failure, and
passing baseline/restoration; compilation failure is not a successful control.
Initial full-workspace validation passed 627 tests/doctests with 23 ignored,
and warnings-denied workspace/all-target Clippy passed. The first control run
caught the late-member mutant at an earlier unlabelled count assertion; that
attempt remains rejected. Reordering the same assertions makes the intended
membership failure explicit without changing a production predicate.

The complete second control run passed all 11 baseline/mutant/restored triples.
The default-feature three-process fixture also passed context/follower refusals,
leader kill, new-leader read/write, deletes, and original-directory restart to
term 2/index 14. Four actual executing process lifetimes were identified and
all exited. This is a process fixture, not a replacement for Chaos Mesh.
Raw records are `/tmp/kv9-read-groups-controls-second` and
`/tmp/kv9-read-groups-process-first`; the rejected first control run is retained.

Performance measurement and exact-source proof/Chaos acceptance remain open.
Routine validation stays local; hosted CI remains manual-only.
