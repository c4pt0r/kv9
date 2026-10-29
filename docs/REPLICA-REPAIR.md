# Automatic data-group repair

Design packet, 2026-10-29. Parent [#9](https://github.com/c4pt0r/kv9/issues/9). Second of the four
increments agreed after [REPLICA-ADDITION](REPLICA-ADDITION.md) §7. The data-group analogue of
[VOTER-REPLACEMENT](VOTER-REPLACEMENT.md): the same verdict, taken by each data group's leader
about its own voters, and the repair is a committed **move plan** that the replica-plan driver
carries out.

> **Status: implemented** (2026-10-29). `runtime/replica_repair.rs` takes the verdict on every
> group this node leads and commits the move plan; the Lean model is accepted
> (`scripts/prove-replica-repair.py`); `scripts/replica-repair-e2e.sh` passes on real MinIO and runs
> in CI. §7 records what the implementation settled beyond this packet.

## The problem

A data group with three voters survives one loss. Nothing today notices that loss for the group:
the metadata leader's voter verdict covers `META_REGION_0` only, and a data group's dead replica
stays a voter until an operator commits a move. The operator's move is now one verb and the rest
is automatic (REPLICA-ADDITION), so what is missing is exactly the verdict and the choice of a
destination.

## 1. The verdict, at the group leader

Only the group's leader has an authoritative, local view of each voter's progress, so the verdict
is the leader's, per group, from `peer_progress` of that group's driver — the same rule as the
metadata verdict:

- a voter is **suspected** after `lost_after / 4` without a response or a `matched` advance, and
  **lost** after `KV9_REPLICA_LOST_SECS` (default 300 s) of continuous silence;
- `KV9_AUTO_REPAIR_REPLICA=1` enables decisions (default off, like `KV9_AUTO_REPLACE_VOTER`); the
  verdicts and the status line `replica_repair=` ship either way;
- a leader exists only while the group has a quorum, so a decision is never taken for a group that
  already lost it (REPLICA-ADDITION §2 gives the one safe order from there: promote first, remove
  second — the move plan is exactly that).

## 2. The decision: a move plan

For a lost voter `X` of group `G`, the leader commits a **move plan** (REPLICA-ADDITION §1) with
`source = X` (its exact committed incarnation) and `destination = D`, where `D` is chosen from the
catalog alone, deterministically:

- an **active registered store** (node directory row with an active endpoint, no pending or
  revoked admission) that is **not a committed member** of `G` and **not** the lost node;
- preferring the store that hosts the **fewest data groups** (counted from committed membership
  across all groups), ties broken by the lowest node id;
- the operation id is derived — `sha16("kv9-replica-repair/v1" ++ root ++ region ++ X.incarnation)`
  — so a second leader deciding the same loss confirms the same plan instead of committing a
  second one, and a store that returns and is lost again later gets a new identity (new
  incarnation, new plan).

The plan is committed at the metadata leader through the same planner `AddReplica` uses; when the
group leader is not the metadata leader it sends the decision over the node-authenticated internal
service (`PlanReplica`, beside `RecordInstallEvidence`). One live plan per group (the planner's
rule) means a repair never races an operator's own plan: whichever committed first drives, the
other is refused and the verdict is retried later. A plan that cannot find a destination is
recorded in the status line and retried on the next pass.

## 3. Invariants

- **R1 — a repair names a lost voter and a non-member.** The source is a committed member that the
  leader has observed silent for the full window; the destination is an active store outside the
  group.
- **R2 — one plan per loss.** The derived operation makes concurrent or repeated decisions for the
  same lost incarnation confirmations of one row.
- **R3 — the group is never shrunk first.** The repair is a move plan, so the driver promotes the
  replacement before it removes the lost voter (REPLICA-ADDITION A2); a group of three never passes
  through two voters.
- **R4 — a false verdict is bounded.** A partitioned voter that returns is removed anyway once the
  plan is committed (its log is never truncated on the strength of the verdict; it is simply no
  longer a member) and can be added again by a later plan; `KV9_REPLICA_REPAIR_COOLDOWN_SECS`
  (default 600) spaces one group's decisions.

## 4. What it does not do

Repair the metadata group (VOTER-REPLACEMENT does), create stores (the pool is what is
registered), choose placement by load or locality (D06), or act without a group leader.

## 5. Evidence

- Lean model `proofs/lean/replica-repair`: verdict and decision over the group leader's
  observations, R1–R3 as theorems, controls (decide on an active voter, pick a member as
  destination, pick the lost node, decide without the window, decide twice with different ids).
- `scripts/replica-repair-e2e.sh` on real MinIO: a data group on stores 1,2,3 plus a registered
  store 4; kill the group voter that is not the metadata leader; within the window the group
  leader commits a move plan to 4 and the driver takes the group to voters `{survivors, 4}`;
  writes and reads continue; the dead store's incarnation appears as the plan's source.

## 6. The increment

The verdict and decision step (`runtime/replica_repair.rs`), the internal `PlanReplica` RPC, the
status line, the model and the e2e. `kv9-admin group describe` already shows the plan and its
stage; nothing new is needed there.

## 7. As built

- **Where the verdict lives.** `RegionManager::led_groups()` hands the reconcile loop every data
  group this node leads with the driver's `peer_progress`; `reconcile_replica_repair` runs right
  after the plan driver's pass. A voter's `last_active` advances on `recent_active` (reset by
  check-quorum each election timeout) or a `matched` advance, exactly as `voter_replacement.rs`.
- **One decision at a time, off the reconcile thread.** The decision is derived on the reconcile
  thread from committed state only (`committed_members`, the node directory, `committed_plans`),
  then committed on a worker thread: locally through `AdminApi::add_replica` when this node leads
  the metadata group, otherwise through `PlanReplica` over the node-authenticated internal service
  at the metadata candidates. The worker's outcome is one `events[]` entry of the status line.
- **R2 in practice.** While a plan for the derived operation exists — in flight or aborted by an
  operator — the leader decides nothing new for that incarnation; an aborted repair is therefore
  an operator's decision that stands until the lost member is moved by hand.
- **Quorum guard.** A decision is only taken when the group's live voters plus the promoted
  replacement still form a majority, so a group that lost two of three voters is left to the
  operator (it also has no leader to decide).
- **Destination.** Lowest `(hosted data groups, node id)` among active registered stores that are
  neither members nor the lost node; "hosted" counts committed memberships across all groups. No
  active candidate is an `error` on the status line, not a decision.
- **Committed membership on the wire.** `ListDataGroups` rows now carry `members`
  (`committed_members`: initial replicas ∪ evidenced destinations − retired sources) beside the
  creation's `replicas`; `kv9-admin group describe` asks the members for the live view and
  `group list` shows them, so a group whose leader is a moved-in destination is still described.
  Store history understands the kind-111 plan row (it introduces no store; its migration does).
- **A third fix, in the runtime.** The checkpoint-retention sweep (the task that replays drain
  reports whose inline submission failed) was cancelled for good by the start-up maintenance
  withdrawal and never came back, so one failed submission left a report in the outbox and
  blocked every later capture on that node — a second plan on the same group leader could not
  capture. The runtime now restarts the sweep once maintenance is allowed again.
- **Two driver fixes found by the e2e.** The destination's install step now confirms the image's
  objects are uploaded (`verify_checkpoint_objects`) before it opens a generation, because the
  capture descriptor is committed before the upload; and adoption checks for a selected generation
  before taking the group owner lock, so a destination that polls adoption cannot starve its own
  installer of that lock.
- **Knobs.** `KV9_AUTO_REPAIR_REPLICA=1`, `KV9_REPLICA_LOST_SECS` (300), `KV9_REPLICA_REPAIR_COOLDOWN_SECS`
  (600). The e2e runs with a 5 s window and a 1 s cooldown.
