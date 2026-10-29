# Leader transfer for the metadata group and data groups

Design packet, 2026-10-28. Parent [#9](https://github.com/c4pt0r/kv9/issues/9). The first of the four
increments agreed after [REPLICA-ADDITION](REPLICA-ADDITION.md) §7 (leader transfer → automatic
data-group repair → drain a node → split/merge). Builds on the lease model
([LEASE-AUTHORITY-MODEL](LEASE-AUTHORITY-MODEL.md), [LEASE-CONTROLLER](LEASE-CONTROLLER.md)) and on
[KV9-ADMIN](KV9-ADMIN.md).

> **Status: implemented (2026-10-28).** `RaftPeer::transfer_leader` (guards: leader, voter,
> `matched >= committed`, not self, none in flight; lease revoked in the same critical section),
> `NodeDriver::transfer_leader` (bounded wait for an observed new leader), `AdminApi::transfer_leader`
> for the metadata group and data groups, gRPC `TransferLeader`, `kv9-admin node transfer-leader
> --to [--wait]` and `group transfer-leader <creation-task> --to`. `scripts/leader-transfer-e2e.sh`
> (CI matrix) passes: metadata leadership to a follower and back in one bounded step with writes
> continuing, a data group's leadership to a replica, and the learner / non-member / self refusals.
> Lean model `proofs/lean/leader-transfer` (11 theorems, 6 semantic controls) via
> `scripts/prove-leader-transfer.py`. No scheduler calls it yet (§4).

## The problem

Planned maintenance — restarting a node for an upgrade, draining it before retirement, moving load
off a hot node — today waits for an election: the operator stops the leader, `check_quorum` deposes
it after the election timeout, the remaining voters elect a successor, and writes stall for the
whole window. raft-rs already implements leadership transfer (`MsgTransferLeader` → the transferee
receives `MsgTimeoutNow` and campaigns at once, exempt from the pre-vote stickiness that protects a
live leader), and the raw node already revokes this leader's lease when it sees such a message. kv9
deliberately exposes none of it: the only entry point is a test seam
(`RaftPeer::transfer_leader_for_tests`), kept out of every product surface until a design said when
it is safe.

## 1. The verb

`TransferLeader(root, region, transferee)` on the public service (admin-gated), where `region` is the
metadata group or a data group's region, and `kv9-admin node transfer-leader --to <id>` /
`kv9-admin group transfer-leader <creation-task> --to <node>`. It is **leader-only** (a follower
answers `NotLeader` with the hint, as every membership verb does) and refuses unless, at the leader:

- the transferee is a **voter** of the group (never a learner, never a non-member);
- the transferee is **caught up**: its `matched` index is at least the leader's commit index at the
  moment of the decision, so it holds every committed entry (raft-rs ships the uncommitted tail and
  transfers once it is matched; a node behind the commit index is refused, so an operator never
  hands leadership to a lagging node under load). The first implementation demanded an exact match
  with the leader's last index; the 2026-10-29 operations drill showed that a live metadata log
  advances a few entries a second by itself, which made the verb fail several times in a row;
- the group has no **fatal** driver state and no configuration change pending apply;
- it is not already transferring.

The verb then (1) revokes this leader's lease certificates, exactly as the raw node does on an
inbound `MsgTransferLeader`, so no lease read is served after the decision; (2) asks raft to
transfer; (3) waits, bounded by one election timeout, until this node observes a leader that is not
itself — on success the response names the new leader and its term; on timeout the response says
`expired` and the caller may retry (raft-rs abandons an unfinished transfer after one election
timeout and this node leads again, lease and all — a re-election is the only way its old
certificates come back, and that needs a new term).

Nothing is committed for a transfer: it changes no membership, no catalog row. Leadership is not a
durable fact, so the receipt is an observation, as `ClusterInfo` is, and `kv9-admin --wait` polls
`NodeStatus` until the group reports the expected leader.

## 2. Invariants

- **T1 — single leader per term** is raft's; the transfer adds no term in which two nodes lead.
- **T2 — no lease read after the decision.** Revocation precedes the transfer message in the same
  critical section that holds the peer; a lease read in flight validates its certificate generation
  on finish and fails closed.
- **T3 — only a caught-up voter is nominated.** The verb's guard is the model's guard; the
  transferee may still lose an election to a third node (raft permits it), and the receipt reports
  whoever leads.
- **T4 — a transfer never leaves the group without a leader for longer than one election
  timeout.** Either the transferee campaigns on `MsgTimeoutNow` (its own term, immediate) or the
  old leader resumes.

## 3. Where it lives

| Piece | Location |
| --- | --- |
| production transfer with guards | `RaftPeer::transfer_leader` (beside the test seam, which stays test-only), `NodeDriver::transfer_leader` |
| the verb | `AdminApi::transfer_leader`, `RuntimeBackend` for `META_REGION_0` (`self.driver`) and for a data group (`raw_directory.by_region`), gRPC `TransferLeader` |
| the tool | `kv9-admin node transfer-leader --to <id> [--wait]`, `group transfer-leader <creation-task> --to <node> [--wait]` |
| evidence | Lean model `proofs/lean/leader-transfer` (T2–T4 as transitions over leader, lease generation and transferee progress; controls: nominate a learner, nominate a lagging voter, transfer before revocation), `scripts/kv9-admin-e2e.sh` extended (metadata transfer to a follower with writes continuing; data-group transfer observed through `group describe`; refusals for a learner and a non-member) |

## 4. What this enables next

Automatic data-group repair and node drain both begin by moving leadership off the node that is
about to be removed or retired; with this verb they do it in one bounded step instead of a timeout.

## 5. Out of scope

Lease handoff that keeps the fast read path warm across the transfer (the new leader earns its own
lease); transfer of a learner-attached group mid-migration (the driver simply waits: a migration
step that lands at a node that no longer leads is confirmed by the new leader on its next pass);
any placement decision about *who* should lead.

## 6. The increment

Implement §1 and §3 in one step, with the Lean model and the e2e extension; no server-side scheduler
uses it yet — drain and repair (the next two increments) are its first callers.
