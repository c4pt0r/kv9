# Drain a node

Design packet, 2026-10-29. Parent [#9](https://github.com/c4pt0r/kv9/issues/9). Third of the four
increments agreed after [REPLICA-ADDITION](REPLICA-ADDITION.md) §7, after
[REPLICA-REPAIR](REPLICA-REPAIR.md). One committed row says "store `S` of node `N` leaves the
cluster"; the cluster then moves every data-group replica off `S` one plan at a time, hands away
every leadership `N` holds, takes `N` out of the metadata group without ever shrinking it below
three voters, and reports `drained`. Nothing is deleted.

> **Status: implemented** (2026-10-29). `runtime/node_drain.rs` and `runtime/placement.rs`,
> `kv9-admin node drain`, the Lean model (`scripts/prove-node-drain.py`) and
> `scripts/node-drain-e2e.sh` on real MinIO, in CI. §9 records what the implementation settled.

## The problem

Taking a store out of service today is a sequence of operator verbs in exactly the right order:
transfer every data-group leadership away, one `group move-replica` per group with a destination
chosen by hand, wait for each, transfer metadata leadership, promote a learner if the metadata
group has only three voters, `node remove`. Each step exists (LEADER-TRANSFER, REPLICA-ADDITION,
VOTER-REPLACEMENT's verbs) and each is safe on its own; what is missing is the one committed
intent that lets the cluster sequence them, so the operator says it once and watches.

## 1. The row: a drain intent

`TASKS` kind **112**, `KV9DRN01`:

```
root 32 | operation 16 | task 8 | node 8 | incarnation 16
```

- `operation = sha16("kv9-drain/v1" ++ root ++ node ++ incarnation)`: a drain is an identity of the
  **store incarnation**, not of the node id. A node that later re-joins with a fresh store is a
  new store and is placeable again; the old row stays as history.
- Planned by the metadata leader (`plan_drain`): the node must be a registered store with that
  incarnation. An identical resubmission is a confirmation; a drain for a store that is already
  the source of an unsettled plan is fine (the plan is simply one of the drain's steps).
- Public RPC `DrainNode { node }`, `kv9-admin node drain <id> [--wait]`; `ListNodes` rows gain
  `draining`.

## 2. What the drained node does for itself

A node that reads a committed drain naming its own store (from its applied catalog, no barrier
needed: the row is immutable) **hands away every leadership it holds**, repeatedly if re-elected:

- each data group it leads → `transfer_leader` to the voter with the highest `matched`
  (LEADER-TRANSFER's guards: caught up, not itself);
- the metadata group, if it leads it → the same, after the data groups.

It keeps serving as a follower throughout: reads, writes it forwards, installs it is the source
of. It never removes itself from anything; the leaders do that.

## 3. What the metadata leader does

Per committed, unfinished drain of store `S` on node `N`, each reconcile pass, one step:

1. **Move the replicas.** For each data group in which `S` is a committed member
   (`committed_members`) and which has no unsettled plan: commit a **move plan** `S → D`
   (REPLICA-ADDITION), `D` from placement (§4). At most `KV9_DRAIN_PARALLEL` (default 1) plans
   with source `S` may be unsettled at once, so a drain is a sequence, not a burst. The plan
   driver does the rest; the source's removal needs the group's leader not to be `N`, which §2
   arranges and the driver retries until true.
2. **Leave the metadata group.** Once no data group counts `S` as a member:
   - `N` a metadata **learner** → `remove_learner` (the admission path's inverse); not a member →
     nothing;
   - `N` a **voter** with four or more voters → `RemoveNode` (the existing verb; refuses the leader
     itself, which §2 makes a transient state);
   - `N` a voter of a **three-voter** group → first promote the best caught-up metadata learner
     (VOTER-REPLACEMENT's rule: recent, within `KV9_VOTER_CATCHUP_LAG` of the commit index), then
     remove. No such learner → the drain **waits** and says so in the status line
     (`needs a metadata learner: admit and join one more node`); it never removes below three.
3. **Done** when `S` is in no data group and `N` is neither voter nor learner of the metadata group.
   `drained` is derived from committed state every time it is asked; there is no completion row.

The verbs are the ones that exist; the drain adds no new membership mechanism.

## 4. Placement, shared with repair

`runtime/placement.rs`: the destination for a replica of group `G` is the active registered store
that is **not a member of `G`**, **not draining**, and **not the lost or drained node**, with the
fewest committed memberships across all groups, lowest id among equals. REPLICA-REPAIR's decision
moves to this helper and gains the "not draining" exclusion.

## 5. Invariants

- **N1 — a group never shrinks.** Every membership change of the drain is a move plan; a group of
  three has three voters at every committed state (REPLICA-ADDITION A2).
- **N2 — the metadata group never has fewer than three voters.** Removal of a three-voter member
  is preceded by a promotion or does not happen.
- **N3 — a draining store receives nothing.** Placement excludes it, for repair and for other
  drains alike.
- **N4 — bounded concurrency.** At most `KV9_DRAIN_PARALLEL` move plans with the drained source
  are unsettled at any time.
- **N5 — progress.** With a placeable destination for every group and (if needed) one caught-up
  metadata learner, the drain reaches `drained` without operator steps; otherwise it waits with a
  reason and resumes when the condition holds.

## 6. What it does not do

Delete the store's files or stop its process (the operator does, once `drained`); drain two
stores of the same group concurrently (the second waits for placement of the first: N1 holds, it
is merely slow); rebalance anything else; survive without a metadata leader (nothing does).

## 7. Evidence

- Lean model `proofs/lean/node-drain`: groups as member sets, metadata voters and learners, drains,
  unsettled plans; steps plan / settle / promote / remove / finish; N1–N4 as theorems; controls
  (plan to a draining store, remove from three voters, two plans in flight with parallel 1, settle
  that shrinks a group).
- `scripts/node-drain-e2e.sh` on real MinIO: stores 1,2,3 with a data group and a bound keyspace;
  node 4 joined and promoted (four metadata voters), node 5 joined as a learner; `kv9-admin node
  drain 1 --wait` while node 1 leads the data group; expect the group on `{2,3,5}` (5 is the only
  non-member active store), metadata voters `{2,3,4}`, node 1 leading nothing, every key readable
  and writable throughout, `node list` showing `draining` then `drained`.

## 8. The increment

`kv9_meta::data_groups::drain` (row, planner, readback, store-history arm), `AdminApi::drain_node`
+ `DrainNode` + `ListNodes.draining`, `runtime/node_drain.rs` (§2 on every node, §3 on the
metadata leader), `runtime/placement.rs`, SDK `drain_node`, `kv9-admin node drain`, the model, the
e2e and its CI step.

## 9. As built

- **Placement never returns a group to a store that held it** (initial replicas and removed
  sources: their retired directory would make the installer refuse). A drain therefore needs,
  for each group on the drained store, one active store that never hosted that group; the status
  line says `no active, non-draining store outside region N` until one is admitted.
- **Abort.** `kv9-admin task abort <drain-task>` ([RANGE-SPLIT](RANGE-SPLIT.md) §12): the drain
  becomes history, placement stops excluding the store, the node stops handing leadership away,
  plans already committed complete, and the store can be drained again (the row carries a
  sequence, `KV9DRN02`). A drain's move plans are refused while a split or merge names the group.

- **Two roles, one reconcile pass.** `reconcile_node_drain` runs after the repair verdict. A
  node whose own store is named by a drain hands away one leadership per pass (data groups first,
  through `AdminApi::transfer_leader` on a worker thread, to the caught-up voter with the highest
  `matched`), then the metadata group. The metadata leader derives every drain's view from
  committed state each pass and takes at most one step: a move plan, a learner promotion, a voter
  removal (`AdminApi::remove_node`, which keeps the three-voter floor) or a learner removal.
- **Plan identity.** `sha16("kv9-drain-move/v1" ++ drain operation ++ region)`: one plan per
  (drain, group), so a repeated pass confirms. A group with any unsettled plan is skipped until it
  settles, and `KV9_DRAIN_PARALLEL` (default 1) bounds the unsettled plans with the drained source.
- **`drained` on the wire.** `ListNodes` rows carry `draining` and `drained`; `drained` is derived
  on the leader from the committed memberships and its raft configuration, and `kv9-admin node
  drain --wait` polls it (default deadline fifteen minutes: whole replicas move through object
  storage). The status line `node_drain=` shows each drain's remaining groups, unsettled plans,
  metadata role and the reason it waits, plus the steps taken.
- **After the drain** the node still runs; out of the metadata group it reports
  `bootstrap_state=Joining` and serves nothing, which is the signal to stop it. Its catalog row
  stays (history), its store incarnation stays named by the drain, and placement keeps skipping it.
- **The e2e** drains the node that leads both the data group and the metadata group with the
  metadata group at three voters: the leader hands both away, the group moves to node 4, a
  learner is promoted before node 1 is removed, every key stays readable and writable.
