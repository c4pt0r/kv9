# Replica addition and move as one committed plan

Design packet, 2026-10-28. Parent [#9](https://github.com/c4pt0r/kv9/issues/9); placement stage (D06,
[#27](https://github.com/c4pt0r/kv9/issues/27)). Builds on every migration packet —
[MIGRATION-AUTHORITY](MIGRATION-AUTHORITY.md), [LEARNER-ATTACH](LEARNER-ATTACH.md),
[SOURCE-CAPTURE](SOURCE-CAPTURE.md), [JOINT-SNAPSHOT-INSTALL](JOINT-SNAPSHOT-INSTALL.md),
[RUNTIME-ADOPTION](RUNTIME-ADOPTION.md), [DESTINATION-EVIDENCE](DESTINATION-EVIDENCE.md),
[VOTER-PROMOTION](VOTER-PROMOTION.md), [REPLICA-REMOVAL](REPLICA-REMOVAL.md), [MIGRATION-ABORT](MIGRATION-ABORT.md)
— and on [KV9-ADMIN](KV9-ADMIN.md), whose increment 3 this is.

> **Status: implemented (2026-10-28).** Plan rows (`crates/meta/src/data_groups/addition.rs`), the
> driver (`crates/server/src/runtime/replica_addition.rs`, status line `replica_plans=`), the
> online install at the destination, the node-authenticated `RecordInstallEvidence` RPC, the
> repeat-migration rules of §1.1, `kv9-admin group add-replica / move-replica` and the plan stage in
> `group describe`. `scripts/replica-addition-e2e.sh` (real MinIO) passes: one `add-replica` takes a
> group from voters `1,2,3` to `1,2,3,4` and one `move-replica --from 1 --to 5` to `2,3,4,5` with no
> operator step in between; the installs and evidence deliveries ran on the destinations only, every
> other step on the group/metadata leader. The Lean model (19 theorems, 10 semantic controls) is
> checked by `scripts/prove-replica-addition.py`. Not done: leader transfer, automatic repair, drain
> (§7); `group abort` stays the operator's `record-migration-abort` + `detach-aborted-learner`.

## The problem, and the question behind it

Growing or moving a data group today is thirteen operator commands against three parties (the
metadata leader, the group leader, the destination host), one of which requires stopping the
destination store. Every one of those commands is sound: each is a committed, exact, idempotent task
row or configuration change, and each refuses when its prerequisite is not committed. What does not
exist is anything that *drives* them. The operator is the state machine.

The natural question is why a new replica cannot simply load the group's image from object storage.
It can — that is what the destination does — but object storage alone cannot give it three things:

1. **The log tail.** SSTs are checkpoints, not a read path; everything after the captured cut is in
   the replicas' Raft logs. The new store must join the group as a learner and catch the tail up
   through ordinary replication. That is a committed configuration change.
2. **A pinned image.** Images are reclaimed when nothing holds them. The capture must be bound to the
   operation in the retention ledger (source and destination owners, `Published`) before the
   destination reads it, and the pins outlive the install.
3. **Proof that the right image landed on the right store.** The evidence row names the destination
   *incarnation*, the image digest, the manifest subject and the cut. Promotion and removal are
   authorized by that row, not by a process claiming to be ready.

So the protocol stays. This packet makes the cluster drive it: one committed **replica plan** per
operation, and a reconcile step on each party that derives the next verb from committed state and
executes it, including the install, **online, inside the running destination process**. The
offline `install-migration-image` command remains for a store that is not running.

## 1. The plan row

TASKS kind **111**, magic `KV9ADD01`, `crates/meta/src/data_groups/addition.rs`:

```
operation        16 bytes, the migration operation this plan drives (same retry identity)
migration_task   the kind-103 intent row, committed in the SAME catalog transaction
region           the group
mode             1 = Add (grow by one voter)   2 = Move (grow, then retire one source replica)
source           (node, store_incarnation) for Move: one of the creation's initial replicas, never
                 the destination; zero for Add
```

`AddReplica(root, operation, creation_task, destination, mode, source)` at the metadata leader plans
the migration intent (`plan_migration`) and the plan row in one transaction. An identical
resubmission confirms both; a divergent one refuses. One live plan per group: a new plan for a
region is refused while an earlier plan is neither terminal (§3) nor aborted. The existing
`MigrateDataGroup` verb still commits a bare intent that nothing drives; the two coexist.

### 1.1 A group is migrated more than once

Before this packet a group could be migrated once in its life (a second intent was refused unless
the first was aborted), and only the creation's *initial* replicas could ever be retired. Neither
survives contact with the goals this exists for (HA repair, scale-out, moving data off a node), so
the committed-authority rules become:

- **One operation at a time, not one per life.** A predecessor operation is **settled** when it is
  aborted, or when its evidence row exists and — if a plan drives it as a move — its removal
  decision exists (`operation_settled`). `plan_migration`, the evidence planner and the migration
  readback admit the next operation of a group exactly then.
- **Membership is committed, not initial.** The group's committed members are the creation's
  replicas, plus every evidenced migration destination, minus every replica a removal decision
  retired (`committed_members`). A move's source must be a committed member; a removal decision that
  retires a replica introduced by an earlier migration records the evidence task that introduced it
  (`KV9RMV02`, the V1 encoding is unchanged for initial replicas), so the store-history projection
  still verifies the origin with one exact point read and never a scan.
- What stays closed: re-adding a node to a group it was created in (the capture's membership gate
  refuses a destination among the creation replicas), and two live operations on one group.

## 2. Who does what, from committed state only

Each party runs one reconcile step per pass of the existing maintenance loop (100 ms) and never
blocks it: a step is executed on a worker thread, at most one in flight per process, and its result
is observed on a later pass. Every step is one of the existing verbs, called in-process, so every
guard those verbs already have (root, abort, leadership, learner-before-promote, three voters remain,
never self) applies unchanged. The step a party takes is a pure function of committed rows plus the
facts only it can observe (its role in the group, its own adoption receipt):

| party | precondition (committed unless noted) | step (existing verb) | makes true |
| --- | --- | --- | --- |
| group leader | plan, destination ∉ voters ∪ learners (live) | `attach_migration_learner` | destination ∈ learners, cut advanced |
| group leader | plan, destination ∈ learners (live), no capture | `plan_migration_image` | capture descriptor committed (`committed_capture`) |
| metadata leader | plan, capture, owners not `Published` | `bind_migration_image(descriptor.manifest)` | source and destination pins `Published` |
| group leader | plan, capture, pins `Published`, no evidence | `capture_migration_image` | objects uploaded (idempotent; re-verifies) |
| destination | plan names my store, capture, pins `Published`, no evidence, no adoption receipt (local) | **online install**: `install_captured_record_with_reader` with the record from the committed descriptor and the in-process routed ledger | an installed generation |
| destination | (existing `reconcile_adoption`) | adoption | adoption receipt (local) |
| destination | adoption receipt (local), no evidence | emit the receipt, deliver it to the metadata leader over the node-authenticated internal service (`RecordInstallEvidence`) | kind-104 evidence |
| group leader | evidence, destination ∉ voters (live) | `promote_migration_voter` | destination ∈ voters |
| metadata leader | Move, evidence, no removal decision | `record_source_removal(plan.source)` | kind-106 decision |
| group leader | Move, decision, source ∈ voters (live), destination ∈ voters | `remove_source_replica` | source ∉ voters |

Nothing in the table waits for another party: each precondition is either a committed row that
replication delivers to everyone or a local fact. A party that crashes mid-step loses nothing — the
verb is idempotent and the next pass re-derives the same step from the same committed rows. The
install is the one step with a cost worth bounding: it runs once per process per plan and is
retried only after a failure, and it reuses a selected generation if one already exists (the
installer's own exact-retry rule).

**Abort** is unchanged: a committed kind-108 row makes every verb refuse, so the driver stops on the
next pass; `DetachAbortedLearner` remains the explicit cleanup.

## 3. Terminal states and the stage shown to operators

A plan is **terminal** when: mode Add and the evidence row exists and the destination votes; mode Move
and the removal decision exists and the source no longer votes. The voter facts are live, so the
catalog-only stage reported by `ListDataGroups` is `planned → captured → pinned → evidenced →
decided` (Move) and `group describe` adds the live voter set from the replicas' own views. A plan
with a committed abort shows `aborted`.

## 4. Invariants (the model, §6, proves these for the driver)

- **A1 — every step is enabled only by committed rows.** Capture needs the plan; bind needs the
  capture; install needs the pins; evidence needs adoption which needs install; promotion needs
  evidence; a removal decision needs evidence; removal needs the decision and the promoted voter.
- **A2 — the voter floor.** The group's voter count is never below three; a Move passes through
  four voters (promote first, remove second), never through two.
- **A3 — the destination is never removed, and the source is removed at most once.**
- **A4 — after an abort no step runs.**
- **A5 — the next step is deterministic.** For every non-terminal, non-aborted state the driver has
  exactly one enabled step per party, and it is the one the table names; committed facts are
  monotone, so re-deriving after a crash yields the same step or a later one, never an earlier one.

## 5. Security and the internal service

The destination must tell the metadata leader "I adopted exactly this image". That is the only
cross-node message the driver needs, and it travels on the existing node-authenticated internal
service (`Kv9Raft`, cluster token + declared node id, catalog-checked membership), not on the public
API: public handlers refuse node identities by design (`client identity required`), and this packet
keeps that. The handler commits through the same `record_install_evidence` planner the public verb
uses, so a receipt that names a different subject than the published pin is refused either way.

## 6. Evidence this packet produces

- **Lean model** `proofs/lean/replica-addition/Addition.lean`: the driver as a state machine over the
  committed rows and the live voter facts, with the next-step function; theorems for A1–A5 plus
  reachability of both terminal states, pinned by `source-contract.json` and run by
  `scripts/prove-replica-addition.py` with semantic mutation controls (promote without evidence,
  remove without the decision, remove the destination, bind without a capture, install without
  pins, a step after abort) and the policy controls (no `sorry`/axiom).
- **Catalog tests** for the plan planner (confirmation, divergence refusal, Move requires an initial
  replica that is not the destination, one live plan per group).
- **Real-process e2e** `scripts/replica-addition-e2e.py` on real MinIO: three voters and a fourth
  registered store; `kv9-admin group add-replica` → the group reaches four voters with no further
  operator action; `group move-replica --from 1 --to 5` with a fifth store → voters `2,3,4,5`;
  writes and linearizable reads continue throughout; a plan that names the destination as the source
  is refused; the status line `replica_plans=` on each party shows the steps it took.
- `group describe` shows the plan and its stage; `task list` shows kind 111.

## 7. What follows, in this order

Agreed on 2026-10-28 as the sequence after this packet, each its own increment with its own
proof and e2e:

1. **Leader transfer** for a data group and for the metadata group — a committed, leader-only verb
   that hands the lease to a caught-up voter, so planned maintenance does not wait for an election.
2. **Automatic data-group repair** — the data-group analogue of voter replacement: a replica that is
   lost (by the group leader's progress verdict) is replaced by a move plan to a live store chosen
   by the operator's policy, driven by this packet's machinery.
3. **Drain a node** — move every replica off a store (one plan per group, sequenced), then retire
   the store.
4. **Range split and merge** — re-entry from `experimental/range-split` against the current tree.

## 8. Out of scope, on purpose

- **Placement.** Which node receives the replica is the operator's (or D06's) choice.
- **Source log truncation** (kind 105) is not part of the chain; the group's ordinary compaction
  covers log growth.
- **Rolling upgrades.** The internal service gains one RPC; an older node answers `Unimplemented`,
  which the driver reports and retries. Mixed-version operation remains unqualified, as before.
- **Online install for a replica this store already serves** (re-adding a removed replica to the same
  store): the installer refuses an occupied group, as it always did.
