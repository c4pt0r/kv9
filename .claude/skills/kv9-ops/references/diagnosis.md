# Diagnosing a kv9 cluster

Contents: 1 the status document · 2 subsystem lines · 3 task kinds · 4 stages and what "done"
means · 5 stuck-operation checklist · 6 what to capture for a bug report · 7 known environment
effects

## 1. The status document

Every node rewrites `<data-dir>/status` about once a second (`kv9-admin cluster status [--node]`
fetches them all). Read these first:

```
pid=                 the process that wrote this file (after a restart, the old file says Serving)
bootstrap_state=     Joining until registered; Serving once participating
role=                leader | follower | learner | candidate | unconfigured (left the metadata group)
leader_id=           the metadata leader this node knows (0: none yet)
meta_voters= meta_learners=
term= applied_index= applied_term= raft_committed=
endpoint_ready=      true once the public endpoint serves
fatal=               non-empty: the process stopped making progress; read its log
data_groups=         JSON: the groups this node hosts, with role/state/voters per group
```

`<data-dir>/metrics.json` is the metrics export (latency histograms, apply lag).

## 2. Subsystem lines

Each is one JSON object with `events` (recent, with `at_unix`, `step`, `outcome`) and `error`:

| Line | On which node it matters | Says |
| --- | --- | --- |
| `replica_plans=` | the group leader (attach, plan_image, capture, promote, decide_removal, remove) and the destination (bind, install, record_evidence) | `enabled`, `in_flight`, the last outcome of each step |
| `replica_repair=` | each group leader | `lost_after_secs`, per group the lost voters and the decision |
| `voter_replacement=` | the metadata leader | `baseline_voters`, lost voters, `pending` |
| `node_drain=` | the metadata leader (`drains[]`: `memberships_left`, `plans_unsettled`, `meta_role`, `finished`, `waiting`) and the draining node (`this_store_draining`, `transfer_group_leader`) | |
| `range_split=` / `range_merge=` | the parent's leader (seal, publish) and the children's leaders (child_descriptor, populate) | `intents`, `populations`, `in_flight` |
| `backup=` | each range leader (plan_image, capture) and the metadata leader (bind, bundle) | per backup `stage`, per range `state` and `lead`, `waiting` |
| `auto_split=` | each group leader | footprint verdicts when `KV9_AUTO_SPLIT_BYTES` is set |
| `checkpoint_retention=`, `data_checkpoint=` | every node | object-storage housekeeping; `deferred: metadata not ready` lines at startup are normal |
| `fault=` | test only | armed fault points (`KV9_FAULT_DIR`) |

## 3. Task kinds (`task list --kind <n>`)

100 group creation · 101 activation · 102 range binding · 103 migration · 104 install evidence ·
105 truncation · 106 removal · 108 migration abort · 110 compaction · 111 replica plan ·
112 node drain · 113 split intent · 114 split evidence · 115 merge intent · 116 merge evidence ·
117 operation abort (split/merge/drain/backup) · 118 seal evidence · 119 backup intent ·
120 backup anchor · 121 backup retirement · 122 backup completion.

A healthy drill leaves, for one group that was added to, moved, split, merged, drained and
repaired: 100 ×4, 102 ×4, 103 ×5, 106 ×3, 108 ×1, 111 ×5, 112 ×1, 113 ×1, 114 ×2, 115 ×1, 116 ×1,
118 ×3.

## 4. Stages and what "done" means

| Operation | Stages (catalog) | Done when |
| --- | --- | --- |
| Add replica | planned → captured → pinned → evidenced | **evidenced** and the destination is in the leader's live `voters` (there is no later stage) |
| Move replica | … → evidenced → decided | decided and the source is out of the live voters |
| Any plan | aborted | the kind-108 row exists; the driver detaches the learner afterwards |
| Split / merge | intended → populating → populated → published | published: parent row `sealed`, children `live` in `group list` |
| Drain | (status line) `finished: true` | `node list` shows `drained`; `memberships_left: []`, `meta_role: none` |
| Backup | intended → capturing → anchored → complete | complete: the kind-122 row exists (bundle written and GET-verified); `restorable: yes` when every anchor carries rows and a digest |
| Repair / voter replacement | a kind-111 plan / membership change appears | the lost node is out of the live voters |

Settledness, which gates the next operation on a group: a plan is settled when aborted, or
evidenced (Add), or evidenced and decided (Move). A split or merge is settled when published or
effectively aborted (abort row without seal evidence).

## 5. Stuck-operation checklist

Go in this order; each step narrows the owner of the next action.

1. **Is it actually stuck?** Compare the stage with §4. Give a plan 40 s before worrying (one
   install retry is 30 s); a split of a small range publishes in seconds, a large one copies at
   512-row pages.
2. **Which row is missing?** `task list --kind …` for the next row in the chain (plan 111 →
   migration 103 → evidence 104 → removal 106). `group describe` shows `migrations[]` with
   `evidence_task`, `abort_task`, `removal_task`.
3. **Which node owns the next step, and what does its line say?** Leader-side steps on the group
   leader (find it in `group describe → replica_views`), install steps on the destination, drain
   steps on the metadata leader, split steps on parent/child leaders. Look at `events[-1].outcome`
   and `error`.
4. **Known blockers**, in rough order of frequency:
   - exclusivity: a live plan on the group blocks split/merge and the reverse — the refusal or the
     `waiting` text names it; wait for it to settle or abort it;
   - no placement target: drain and repair wait with `no active, non-draining store outside region
     …`; admit and join a node that never held the group;
   - object storage: every plan stalls at `capture` or `install` when MinIO is unreachable; the
     outcome names the endpoint error;
   - the group is retired (sealed binding): the planner refuses, and the drain and the node
     directory skip it; operate on the live child;
   - a three-voter metadata group and a drain of a voter: `needs a metadata learner: admit and join
     one more node`;
   - `fatal=` set on a node: that node has stopped; its log says why; restart it with the same
     command after reading the log;
   - the destination node is down: the plan waits for it; restart it; the plan resumes from
     committed state (install is idempotent).
5. **Wrong rather than slow?** `task abort <id>`. The drivers detach what they attached and
   reclaim what they created; the row stays as history (`aborted`).

## 6. What to capture for a bug report

- `kv9-admin --output json group describe <task>`, `group list`, `node list`, `cluster health`,
  `cluster info`;
- `task list --kind` for 103, 104, 106, 108, 111, 112, 113–118;
- the full `status` file of the group leader, the destination/draining node and the metadata
  leader, plus `metrics.json`;
- the node logs (`kv9 start` stdout/stderr) around the timestamp;
- the exact verb, its JSON response (`applied.term/index` dates it), and the metadata `term`
  before and after (term jumps mean elections).

## 7. Known environment effects

- **Disk latency re-elects the metadata leader.** Tick 20 ms, election timeout 10 ticks,
  `check_quorum` on: a leader whose append blocks > 200 ms is deposed. A RAID with fsync p90 64 ms
  took the term from 1 to 12 in ten minutes under install load; NVMe (p90 5 ms) stayed at term 2.
  Watch `term=` over time; a climbing term on an idle cluster is a disk or network symptom.
- **The metadata log is never idle**: a few entries per second from drivers and heartbeats. Any
  verb that compares against the last index races; the current transfer guard compares against the
  commit index for this reason.
- **Debug builds** make plans ~30 s and population ~1 s per few hundred rows; do not read drill
  timings as capacity numbers.
