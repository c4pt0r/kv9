# kv9 operations cookbook

Command sequences with the outputs to expect, taken from the 2026-10-29 drill
(`docs/OPERATIONS-DRILL-2026-10-29.md`, run 6, debug build, NVMe, seven nodes, 400 rows). Times
are what that run measured; a release build and real data change them, the shape does not.
`kv9-admin` below means `kv9-admin --endpoints <nodes> --yes`, with `KV9_ADMIN_TOKEN` exported.

Contents: 1 cluster formation · 2 adding nodes · 3 data group and keyspace · 4 leadership ·
5 add and move a replica · 6 abort · 7 split and merge · 8 drain · 9 unplanned loss ·
10 restart · 11 refusals you will meet · 12 the REPL

## 1. Cluster formation

```bash
export KV9_BOOTSTRAP_TOKEN=… KV9_CLUSTER_TOKEN=… KV9_CLIENT_TOKENS="admin=…,app=…"
for n in 1 2 3; do kv9 store-prepare --node-id $n --data-dir $DATA/n$n; done
#   store_prepared=true node_id=1 store_incarnation=<hex>      (collect id=hex for each)
kv9 root-create --output $DATA/root.bin --voters 1@h:p,2@h:p,3@h:p --store-incarnations 1=<hex>,2=<hex>,3=<hex>
#   root_created=true cluster_id=… root_digest=…
for n in 1 2 3; do kv9 init --root $DATA/root.bin --node-id $n --data-dir $DATA/n$n; done
for n in 1 2 3; do KV9_STORAGE=minio KV9_OBJECT_STORE_…=… KV9_ADMIN_PRINCIPALS=admin \
    kv9 start --node-id $n --addr h:p --data-dir $DATA/n$n >$DATA/n$n.log 2>&1 & done
until grep -q '^bootstrap_state=Serving' $DATA/n1/status; do sleep 0.5; done
```

Expect within one to two seconds: `cluster info` → one leader, `meta_voters [1,2,3]`, one keyspace
(`system`), one region; `cluster health` → `overall ok` with eight `ok` verdicts. The same
environment (`KV9_STORAGE`, `KV9_OBJECT_STORE_*`, tokens, switches) must go on every node; a
restart uses the same `start` command and environment.

## 2. Adding nodes

```bash
kv9-admin node admit 4 --addr h:p --ttl 600          # → join_ticket: <hex>  (one-time)
KV9_JOIN_TICKET=<hex> kv9 join  --root root.bin --node-id 4 --data-dir $DATA/n4
KV9_JOIN_TICKET=<hex> kv9 start --node-id 4 --addr h:p --data-dir $DATA/n4 &
kv9-admin node list                                   # n4 learner adm:consumed, within ~1 s
kv9-admin node promote 4 --wait                       # {"outcome":"committed","voters":[1,2,3,4]}
kv9-admin node promote 4 --wait                       # {"outcome":"confirmed", …}  (idempotent)
kv9-admin node admit 8 --addr h:p --ttl 60; kv9-admin node revoke 8   # cancel an unused admission
```

Keep some nodes as learners: they are registered stores, which is what replica plans, drains and
repairs place replicas on, and what voter replacement promotes. A learner that hosts data groups is
a normal configuration.

## 3. A data group and a keyspace

```bash
kv9-admin group create --voters 1,2,3 --label orders
#   {"creation_task":100,"region_id":100,"voters":[1,2,3],"outcome":"created"}
kv9-admin keyspace create --name orders --group 100
#   {"id":100,"name":"orders","creation_task":100,"region_id":100,"outcome":"created"}
kv9-admin group describe 100        # replica_views: one Leader/active, voters [1,2,3]
kv9-admin group list                # TASK 100 REGION 100 MEMBERS 1,2,3 KEYSPACES 100
```

The group is active behind a leader about one second after creation and the keyspace routable
right after; a first write that answers an error is a probe to retry, not a failure. Without
`--group` a keyspace lives on the metadata group.

## 4. Leadership

```bash
kv9-admin node transfer-leader --to 4 --wait
#   {"completed":true,"previous_leader":2,"leader":4,"term":2}
kv9-admin node transfer-leader --to 4 --wait          # previous_leader 4 → confirmed, no election
kv9-admin group transfer-leader 100 --to 1 --wait     # {"completed":true,"previous_leader":3,"leader":1}
```

Refusals: `node 5 is not a voter of this group` (learner or non-member); `node 5 is not caught up
(matched …, committed …)` when the transferee is behind the commit index — retry a second later;
`a leadership transfer is already in flight`. Each metadata transfer is one term; a leader restart
is one more.

## 5. Add and move a replica

```bash
kv9-admin group add-replica 100 --to 4 --label add-4
#   plan_task=104 migration_task=103 … (committed)   watch: kv9-admin group describe 100
```

Stages seen in `group describe 100 → plans[].stage` (run 6): `planned` 0 s → `pinned` 1 s →
`evidenced` 33 s, and the leader's view then shows `voters [1,2,3,4]`. **An Add stops at
`evidenced`**: `decided` is the Move's removal decision. Reads and writes are served throughout.

```bash
kv9-admin group move-replica 100 --from 1 --to 5 --label move-1-5
#   planned → pinned → decided (33 s); voters [2,3,4,5]
kv9-admin task list --kind 111       # both plans
```

Status lines while a plan runs: on the group leader `replica_plans=` events `attach → plan_image →
capture → promote → decide_removal → remove`; on the destination `bind → install →
record_evidence`. The first `install` attempt may log `failed: metadata not ready: image not fully
uploaded yet` and succeed on the retry 30 s later; that is the normal shape, not a fault.

## 6. Abort

```bash
kv9-admin group add-replica 100 --to 6 --label add-6     # plan 111
kv9-admin task abort 111
#   {"abort_task":112,"path":"migration abort (no image planned)","target":"plan","target_task":111}
kv9-admin group describe 100                              # plan 111 stage aborted, voters unchanged
```

A split or merge aborts only before its first seal (`task abort <intent>`; after the seal the
refusal says `sealed: irrevocable`); a drain aborts any time and can be re-issued.

## 7. Split and merge

```bash
kv9-admin group split 100 --at r/00200 --label split-200
#   {"intent_task":117,"child_low":{"creation_task":113,"region_id":101},"child_high":{"creation_task":114,"region_id":102},"stage":"intended"}
kv9-admin group add-replica 100 --to 6        # refused: a live split binds this group's membership until it is published
kv9-admin group describe 100                  # splits[].stage: intended → populating → populated → published (1–2 s for 400 rows)
kv9-admin group list
#   100  region 100  members 2,3,4,5  keyspace 100 [ , )        sealed   splits 117 published
#   113  region 101  members 2,3,4,5  keyspace 100 [ , r/00200) live
#   114  region 102  members 2,3,4,5  keyspace 100 [r/00200, )  live
kv9-admin group merge 113 114 --label merge-back
#   {"intent_task":125,"child":{"creation_task":123,"region_id":103},"stage":"intended"} → published (2 s)
```

The children sit on exactly the parent's members, four included. Between the parent's seal and the
publication the range answers neither reads nor writes (clients retry through it); in the first
second after publication a read in a child may answer `NotLeader` while that child's leader
settles. Sealed parents retire themselves and, with `KV9_RECLAIM_RETIRED=1`, free their files; they
stay in `group list` as history with their `sealed` keyspace rows. TTL-activated ranges cannot be
sealed yet. Atomic batches that cross a range boundary are refused by design; scans and
delete-range cross it, following `resume_from`.

## 8. Drain

```bash
kv9-admin node drain 2 --wait --wait-secs 600
#   {"node_id":2,"task":130,"outcome":"committed"} … --wait: node 2 drained after 33 s
kv9-admin node list                  # n2 none draining drained
kill <pid of node 2>                 # the process is stopped by you; the cluster does not do it
kv9-admin cluster health             # overall fail: unreachable (node 2) — every other verdict ok
```

On the metadata leader `node_drain=` shows per drain `memberships_left`, `plans_unsettled`,
`meta_role`, `finished` and `waiting` (`1 move plan(s) in flight`, `needs a metadata learner: admit
and join one more node`, `no active, non-draining store outside region …`). The drained node hands
its leaderships away first (`transfer_group_leader` events on that node). Count spares first:
placement never returns a group to a store that already held it, including the group's initial
replicas and earlier removal sources, so each hosted group needs one never-hosting store.

## 9. Unplanned loss, repaired by the cluster

With `KV9_AUTO_REPAIR_REPLICA=1 KV9_AUTO_REPLACE_VOTER=1` (loss windows `KV9_REPLICA_LOST_SECS`,
`KV9_VOTER_LOST_SECS`, default 300 s; the drill used 20 s) and a spare store joined:

```
kill -9 node 3  (metadata voter + voter of group 123)
+6 s   cluster health: unreachable fail (node 3)
+48 s  group 123 voters [4,5,1,6]  (repair plan: move 3 → 6 decided by the group leader)
       meta voters [1,4,5,6], learners [7]  (leader promoted 6, removed 3)
```

`replica_repair=` on each group leader and `voter_replacement=` on the metadata leader carry the
verdicts and decisions; `task list --kind 111` shows the repair plan beside the operator's. The dead
node stays in the directory (`node list` → `none`) and in the `unreachable` verdict.

## 10. Restart

Same `start` command, same environment, same data directory. A node is back in about two seconds;
restarting the metadata leader costs one election (one term). After a restart verify `pid=` in the
status file changed before trusting `bootstrap_state`: the old file still says `Serving`.

## 11. Refusals you will meet

| Message | Meaning |
| --- | --- |
| `PermissionDenied: principal "app" may not administer this cluster` | Application token against `kv9-admin`; use the admin principal's token. |
| `a leader never removes itself` | `node remove <leader>`; transfer first. |
| refuses below three voters | `node remove` on a three-voter group; promote a learner first (a drain does this for you). |
| `node N is not caught up (matched …, committed …)` | Transferee behind the commit index; retry. |
| `a live split binds this group's membership until it is published` / `the parent has an unsettled replica plan` | Membership plans and range operations exclude each other on one group. |
| `the group is retired: its range was sealed by a published split or merge` | No plan on a sealed parent; operate on the live child (`group list`). |
| `group creation needs a nonzero operation and 3, 5 or 7 voters` | `group create` only; children of split/merge take the parent's count. |
| `migration destination is not an active registered store` | Admit and join the node first; a learner is enough. |
| `sealed: irrevocable` | Abort after the first seal; let the split/merge publish, then merge/split back. |

## 12. The REPL

```
kv9-repl --addr h:p --peers 1=h:p,2=h:p,… --timeout-ms 10000
kv9> use orders
kv9> put r/00007 value-7            applied (term=1, index=…)
kv9> get r/00007                    value-7
kv9> scan r/00198 r/00203 10        rows, following resume_from across a range boundary
kv9> region r/00300                 region=102 epoch=(conf_ver=1, version=1) leader=2
kv9> info                           nodes=7 keyspaces=2 regions=2
```

`not leader (hint: node N); add the node to --peers to follow it` means the hint chain ended on a
node the REPL could not follow or that was itself not leading yet; with every node in `--peers` a
retry a second later serves.
