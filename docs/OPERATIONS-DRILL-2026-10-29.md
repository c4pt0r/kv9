# Operations drill, 2026-10-29

A rehearsal of every operator primitive in [OPERATIONS](OPERATIONS.md) against a local seven-node
cluster on MinIO, in the order an operator meets them, plus two unplanned-loss scenarios. The point
of a drill is what it turns up: it found two defects that the end-to-end suites had not (both fixed
in the same change), one environment effect worth knowing before choosing disks, and three things an
operator should expect. The complete transcript of every command and its output is
`/mnt/data/kv9-work/kv9-drill-20261029/drill-kv9-drill-<run>.log`; the script that produced it is
[`scripts/ops-drill.sh`](../scripts/ops-drill.sh) and runs from a checkout (`bash scripts/ops-drill.sh`,
about four minutes, needs Docker for MinIO; `DRILL_KEEP=<dir>` copies the log and status files there).

## 1. Setup

| | |
| --- | --- |
| Binaries | `target/debug/kv9`, `kv9-admin`, `kv9-repl` at the commit this document lands in |
| Nodes | 1–3 initial voters; 4–7 admitted and joined; 4 and 5 promoted; 6 and 7 metadata learners (spare stores) |
| Object storage | one MinIO container, `KV9_STORAGE=minio` on every node |
| Node environment | `KV9_ADMIN_PRINCIPALS=admin`, `KV9_RECLAIM_RETIRED=1`, `KV9_AUTO_REPAIR_REPLICA=1`, `KV9_AUTO_REPLACE_VOTER=1`, loss windows shortened to 20 s and cooldowns to 5 s so the self-repair scenarios run inside the drill |
| Data | keyspace `orders` on a group created on stores 1,2,3; 400 rows `r/00000..r/00399` written through the REPL |
| Disk | runs 2–5 on NVMe (`/tmp`); run 1 on the RAID at `/mnt/data` (see finding 3) |

`kv9-admin` is always addressed at a list of nodes, never the leader by name: resolution is the
tool's job. Its output here is JSON because stdout was a pipe; a terminal gets the table.

## 2. The walkthrough

Run 6, 14:08:47–14:11:53, every step as the operator typed it (`kv9-admin --endpoints <three nodes> --yes`
elided; the transcript has the full output). Metadata term in the right column.

**1. Create the cluster** (`store-prepare` ×3 → `root-create` → `init` ×3 → `start` ×3). All three nodes
`Serving` within a second. `cluster info`: leader n2, voters 1,2,3, one keyspace (`system`), one region.
`cluster health`: `ok` on all eight verdicts. The status document of a follower:

```
leader_id=2 role=follower meta_voters=1,2,3 applied_index=2 fatal=
replica_plans={"enabled":true,"in_flight":null,"events":[],"error":null}
replica_repair={"enabled":true,"lost_after_secs":20,"groups":[], ...}
node_drain={"parallel":1,"this_store_draining":false,"drains":[], ...}
```

**3. Grow to seven nodes.** For each of 4..7: `node admit <n> --addr … --ttl 600` prints the one-time
`join_ticket`; on the new host `KV9_JOIN_TICKET=… kv9 join` then `kv9 start`; the node is `Serving` one
second later and `node list` shows it as `learner adm:consumed`. `node promote 4 --wait` →
`committed, voters [1,2,3,4]`; `node promote 5 --wait` → `voters [1,2,3,4,5]`; repeating `node promote 4`
→ `outcome: confirmed`. `node admit 8` then `node revoke 8` cancels an unused admission. Result: five
voters, learners 6 and 7 (the spare stores), seven nodes.

**4. A data group and a keyspace.** `group create --voters 1,2,3 --label orders` → creation task 100,
region 100. `keyspace create --name orders --group 100` → keyspace 100 `created`. The group is active
behind a leader one second later and the keyspace routable right after. REPL: 400 `put`s, then
`get r/00007` → `value-7`, `scan r/0010 r/0011 3` → three rows, `region r/00300` → `region=100 … leader=3`.
`group list`: one group, members 1,2,3, keyspace 100 live over the whole key space.

**5. Planned leadership moves.** `node transfer-leader --to 4 --wait` →
`completed, previous_leader 2, leader 4, term 2`. Repeating it → `previous_leader 4, leader 4`
(confirmed). `group transfer-leader 100 --to 1 --wait` → `previous_leader 3, leader 1`.

**6. Add a replica.** `group add-replica 100 --to 4 --label add-4` → plan task 104. Stages read from
`group describe 100`: `planned` 14:09:12 → `pinned` 14:09:13 → `evidenced` 14:09:45, and the leader's
view shows voters 1,2,3,4. Reads and writes served throughout (`get r/00007`, `put during-add ok`).

**6b. Move a replica.** `group move-replica 100 --from 1 --to 5` → plan 107: `planned` → `pinned` →
`decided` at 14:10:19, voters 2,3,4,5. `task list --kind 111` lists both plans.

**7. Refusals.** With the application token: `PermissionDenied: principal "app" may not administer this
cluster` (exit 1). `node remove 4` while 4 leads: `a leader never removes itself` (exit 1).

**8. Abort a plan.** `group add-replica 100 --to 6` → plan 111; `task abort 111` one second later →
`abort_task 112, path: migration abort (no image planned)`; `group describe` shows plan 111 `aborted`,
voters unchanged 2,3,4,5.

**9. Split.** `group split 100 --at r/00200 --label split-200` → intent 117, children 113 (region 101)
and 114 (region 102), both on the parent's four members. `group add-replica 100 --to 6` while the split
is live: `a live split binds this group's membership until it is published` (exit 1). Stages: `intended`
14:10:20 → `published` 14:10:21. `group list` then shows the parent's keyspace row `sealed` and the two
children live over `[, r/00200)` and `[r/00200, )`. `task list --kind 113/114`: one intent, two
evidence rows. REPL one second after publication: `region r/00100` → region 101, `region r/00300` →
region 102; `get r/00007` served; `get r/00399` and the second page of `scan r/00198 r/00203` answered
`not leader (hint: node 3)` once, while the high child was still settling its leader (see §3).

**9b. Merge.** `group merge 113 114 --label merge-back` → intent 125, child 123 (region 103):
`intended` 14:10:21 → `published` 14:10:23. `group list`: three sealed rows (100, 113, 114) and 123 live
over the whole key space. REPL: `get r/00007`, `get r/00399`, and `scan r/00198 r/00203 10` → five rows
across the former boundary.

**10. Drain a node.** `node drain 2 --wait --wait-secs 600` → drain task 130; the verb returned at
14:10:57, 33 s later: `region 103: node 2 → node 1` planned as plan 132, driven, decided; then
`remove_voter: metadata member 2: voters now [1, 3, 4, 5]`. `node list`: `n2 none draining drained`.
The process is then stopped; `cluster health` → `fail` with `unreachable: node 2` and every other
verdict `ok`.

**11. Unplanned loss.** `kill -9` of node 3, a metadata voter and a voter of group 123. Six seconds
later `cluster health` adds `node 3` to `unreachable`. With `KV9_AUTO_REPAIR_REPLICA=1` and
`KV9_AUTO_REPLACE_VOTER=1` (loss window 20 s): at 14:11:51, 48 s after the kill, group 123's voters
are 4,5,1,6 (plan 136, a move 3 → 6 decided by the group leader) and the metadata voters are 1,4,5,6
with learner 7 (the leader promoted 6 and removed 3). `task list --kind 111` shows the repair plan next
to the operator's.

**12. Restart the metadata leader.** `kill` of node 4 (leader at that point) and the same `start`
command: `Serving` again in 2 s, leader now n6 at term 3, `node describe 4` shows the directory row and
the live status document. `cluster health` stays `fail` for the two nodes that are gone (§3).

**13. Final state.** Keyspace `orders` → region 103, leader n6. Group 123 members 4,5,1,6; the three
sealed groups still listed with their history. `node list`: 1,4,5,6 voters, 7 learner, 2 `drained`, 3
`none`. TASKS rows by kind: 100 ×4, 102 ×4, 103 ×5, 106 ×3, 108 ×1, 111 ×5, 112 ×1, 113 ×1, 114 ×2,
115 ×1, 116 ×1, 118 ×3. Every row of r/00000..r/00399 and `during-add` reads back.

## 3. What the drill found

### Finding 1 (defect, fixed): a group grown by one replica could not be split or merged

After `group add-replica` a group has four voters (a plan adds before it removes, and an Add never
removes; REPLICA-ADDITION §3). `group split` creates its children on the parent's committed
members, and group creation accepted only 3, 5 or 7 replicas, so the split was refused:

```
config error: group creation needs a nonzero operation and 3, 5 or 7 voters
```

The same refusal would have met a merge. Every e2e split starts from a three-replica group, which
is why no suite had seen it. Fix: the 3/5/7 rule stays for an operator's `group create`; a split or
merge child takes exactly the parent's members, three to seven of them (`plan_child_group`, and the
intent decoder accepts 3..7). Proven by run 5: the split and the merge of a four-member group both
publish (§2, steps 9 and 9b).

### Finding 2 (defect, fixed): a drain waited forever on a retired group

After the split and merge, store 2 was still a *committed member* of regions 100, 101 and 102, the
sealed parents of the published split and merge. Those groups retire themselves (RANGE-SPLIT §9)
and have no leader. The drain counted them as memberships to move, planned `region 100: node 2 →
node 6`, and waited on a plan nobody would ever drive:

```
node_drain={... "memberships_left":[100,101,102,103], "plans_unsettled":[132],
            "waiting":"1 move plan(s) in flight"}
```

With the drain's own list fixed (run 4) the drain finished in 33 s, but `node drain --wait` still
timed out after 600 s: the node directory's `drained` flag counted the same retired memberships.
Fix, in one helper (`ranges::retired_regions_in_txn`): the migration planner refuses a plan on a
retired group ("the group is retired: its range was sealed by a published split or merge"), the
drain skips retired memberships, and the node directory does too. Proven by run 5 (step 10).

### Finding 3 (environment): the RAID churns metadata leadership

Run 1 ran on `/mnt/data` (md0). The metadata term went from 1 to 12 in ten minutes with only two
operator transfers; every burst coincided with replica installs or split population. Run 2–5 on
NVMe stayed at term 2–4, the transfers accounting for all of it. The Raft tick is 20 ms and the
election timeout 10 ticks, so a leader whose log append blocks for more than 200 ms is deposed
(`check_quorum`). Measured on this machine:

| disk | fsync p50 | p90 | max |
| --- | --- | --- | --- |
| `/mnt/data` (RAID md0) | 2.6 ms | 64 ms | 98 ms |
| `/tmp` (NVMe) | 5.0 ms | 5.0 ms | 6.5 ms |

Nothing broke: pre-vote and check-quorum did their job and every verb completed. But each
election is a write stall, and the timeout is a constant, not a knob. Recommendation: put the
metadata WAL on a disk with a single-digit-millisecond fsync tail, and consider making the tick
configurable for slower media.

### What an operator should expect (behaviour, documented, unchanged)

- **A transfer can be refused once.** `node transfer-leader --to 5` right after a burst of writes
  answered `node 5 is not caught up (matched Some(318), last index 319)`: one entry behind. The
  retry a second later succeeded. The drill script now retries typed refusals of transfers.
- **An Add plan ends at `evidenced`.** `decided` is the Move's removal decision; an Add has none.
  Completion is the live voter set in `group describe`, not the stage (REPLICA-ADDITION §3). Run 1
  waited five minutes for a stage that never comes.
- **Health is `fail` while a node is down, by design.** `cluster health` exits 1 with an
  `unreachable` verdict for the stopped drained node and for the killed voter until the repair
  lands; the other verdicts stay `ok`.
- **Reads may bounce right after a publication.** One second after the split published, a read in
  the high child and the second page of a scan answered `not leader (hint: node 3)` while that
  child's leader settled; the next attempt served. Clients that follow hints retry this themselves.
- **Health never returns to `ok` after a node is gone.** A drained-and-stopped node and a dead,
  replaced node both stay in the directory and keep `unreachable=fail`. There is no verb to forget a
  node yet (OPERATIONS §8); until there is, read the other verdicts.
- **One refusal not reproduced.** Run 1 once answered `the parent has an unsettled replica plan`
  to a split issued one second after a plan abort, while all three plans read back as settled.
  Runs 2–5 ran the same sequence and never saw it (they met finding 1 instead at that point).
  Kept here as an open observation; `group describe` and `task list --kind 103/104/106/108` are
  the diagnostics to capture if it recurs.

## 4. Timings (run 6, debug build, NVMe, 400 rows)

| Operation | Wall time |
| --- | --- |
| Three-node formation to `Serving` | 1 s |
| Admit + join + start one node | 1 s |
| Add a replica (plan 104, attach → install → evidence → promote) | 33 s |
| Move a replica (plan 107, plus removal) | 33 s |
| Split (intent → children populated → published) | 1 s |
| Merge (intent → child populated → published) | 2 s |
| Drain a node holding one group (one move plan + leave the metadata group) | 33 s |
| Lost voter → repaired group and replaced metadata voter (20 s loss window) | 48 s |
| Leader restart to `Serving` | 2 s |
| Whole drill | 3 min 6 s |

A plan's 33 s is the image round trip through MinIO (capture, upload, install) plus the 30 s
retry after `image not fully uploaded yet` on the first install attempt, visible in `replica_plans=`.
