# Operations: the primitives and how to use them

2026-10-29. What an operator can do to a running kv9 cluster, grouped by what it is for. Every
verb is `kv9-admin --endpoints <any node> <noun> <verb>`; any node resolves the metadata leader,
`--output json` is for scripts, `--yes` skips the confirmation, exit codes are 0 ok, 1 refused
(typed), 2 usage, 3 transport, 4 partial, 6 wait deadline ([KV9-ADMIN](KV9-ADMIN.md)). Everything
the cluster then does by itself is a committed row in the TASKS history (`task list`), driven from
committed state on the node whose role it is, and observable in each node's `<data-dir>/status`.

## 1. Lifecycle of a cluster and its nodes

| Need | Primitive | Notes |
| --- | --- | --- |
| Create a cluster | `kv9 store-prepare` on each initial store → `kv9 root-create --voters 1@..,2@..,3@..` → `kv9 init` on each → `kv9 start` | `KV9_BOOTSTRAP_TOKEN` for formation, `KV9_CLUSTER_TOKEN` + `KV9_CLIENT_TOKENS=principal=token,...` for serving ([QUICKSTART](QUICKSTART.md)). |
| Add a node | `node admit <id> --addr <host:port> [--ttl <secs>] [--meta auto\|full\|scoped]` → on the host `KV9_JOIN_TICKET=… kv9 join …` then `kv9 start …` | The ticket is one-time; `node revoke <id>` cancels an unused admission. `full`: the node joins the metadata group as a **learner** and keeps the whole catalog. `scoped`: it joins no metadata Raft group and keeps only its catalog slice, streamed from a full node (below). `auto` (the default) is `full` while the full set has room (`KV9_META_FULL_LEARNERS`, default 4 learners) and `scoped` after. |
| Make it vote | `node promote <id> [--wait]` | Learner → voter of the metadata group. Repeating a membership verb whose state already holds answers `outcome: confirmed`. |
| Retire a node | `node drain <id> [--wait [--wait-secs <n>]]` | One verb: the node hands away every leadership, the cluster moves every data-group replica off it (one plan at a time, `KV9_DRAIN_PARALLEL`), promotes a learner before removing it from a three-voter metadata group, reports `drained` in `node list`. Then stop the process ([DRAIN-NODE](DRAIN-NODE.md)). |
| Remove a metadata voter by hand | `node remove <id> [--wait]` | Refuses below three voters and refuses the leader itself; promote a replacement first. |
| Move metadata leadership | `node transfer-leader --to <id> [--wait]` | Planned maintenance; the transferee must be a caught-up voter, one that holds every committed entry ([LEADER-TRANSFER](LEADER-TRANSFER.md)). |
| Change a node's address | `kv9 client change-node-endpoint …` | Generation-fenced ([ENDPOINT-MIGRATION](ENDPOINT-MIGRATION.md)); not yet a `kv9-admin` verb. |

### Scoped nodes ([META-REPLICA-MODES](META-REPLICA-MODES.md))

A scoped node hosts data groups like any node but is not a metadata Raft member, so the metadata
group's size and traffic stay constant as the cluster grows. What to know when running one:

- **It cannot vote.** `node promote` refuses it. To make it a full node, drain it and admit it
  again with `--meta full`.
- **Its catalog is a slice, per region.** It holds the regions it hosts, their ancestors
  (split and merge parents, restore sources) and those ancestors' other children, with every row
  about them; their keyspaces and tenants; and the node rows of itself, its co-members and the
  full nodes. Not the rest of a keyspace split across other nodes: `catalog_scope_region_ids`
  lists what it holds. It holds no retention ledger; it asks the metadata leader for those rows.
- **Status.** `meta_mode=scoped`; `catalog_scope=current` (a frame within three idle intervals)
  or `stale`; `catalog_scope_index` is the slice position; `catalog_scope_server` is the full
  node streaming to it; `catalog_scope_error` names the last stream problem. A full node shows
  `catalog_scope_streams` (subscribers it serves) and `catalog_keys`.
- **Requests outside the slice** get a `NotLeader` redirect to the metadata leader; clients with
  a `peers` map follow it.
- **A full node going away** moves only its subscribers, each to its next rendezvous choice;
  watch `catalog_scope_server` change and `catalog_scope=current` return.
- **Rehearsal.** `scripts/meta-scoped-e2e.sh` (about 2.5 minutes) and
  `scripts/meta-scoped-tree.sh` run their nodes with `KV9_DRIVER_BACKOFF_PCT=10`, which scales
  the move, backup and restore drivers' retry gates (default 100 = production: 2 s per step,
  10 s per capture, 30 s per install).
- **Health.** `cluster health` has a `catalog_scope` verdict; at hundreds of nodes use
  `cluster health --full-only`, which reads every slice's position from the full nodes.
- **`catalog_scope_error` naming an undeclared metadata key** means a catalog row this binary does
  not classify reached the stream; the slice stops at its last position (fail-closed). Upgrade the
  scoped nodes' binary to the full nodes' version.

## 2. Data groups, keyspaces, placement

| Need | Primitive | Notes |
| --- | --- | --- |
| A data group | `group create --voters <id,id,id> [--label <text>]` | Three, five or seven registered stores; empty and unbound until a keyspace is bound. |
| A keyspace on it | `keyspace create --name <n> --group <creation-task> [--tenant <id>]` | Without `--group` the keyspace lives on the metadata group. `keyspace list|describe`, `group list|describe <task>` show the catalog and every replica's live view. |
| Add a replica | `group add-replica <task> --to <node>` | One committed plan; the cluster attaches, captures, installs inside the destination, proves, promotes ([REPLICA-ADDITION](REPLICA-ADDITION.md)). Needs object storage (`KV9_STORAGE=minio`, `KV9_OBJECT_STORE_*`). |
| Move a replica | `group move-replica <task> --from <a> --to <b>` | The same plan plus the source's removal after promotion; a group never shrinks first. |
| Move a group's leadership | `group transfer-leader <task> --to <node> [--wait]` | |
| Split a range | `group split <task> --at <key> \| --at-hex <hex>` | The cluster creates the two children on the same stores, seals the parent, copies and verifies both halves, publishes the new partition ([RANGE-SPLIT](RANGE-SPLIT.md)). The parent is unavailable between its seal and the publication; TTL-activated ranges cannot be sealed yet. The children take the parent's members, four included after an `add-replica`. |
| Merge two ranges | `group merge <low-task> <high-task>` | Adjacent siblings on one store set; the same pipeline backwards. |
| Watch any of the above | `group describe <task>` (`plans`, `splits`, `merges` with stages), `task list [--kind <n>]`, `task describe <id>` | Stages: plans `planned → captured → pinned → evidenced → decided \| aborted`; splits/merges `intended → populating → populated → published \| aborted`. `group list` exits 4 and names the family when a catalog readback fails. |
| Abort any of the above | `task abort <task>` | A split/merge before its first seal, a drain any time, a replica plan (through its migration abort). Membership plans and range operations exclude each other while live: the refusal says which one holds the group ([RANGE-SPLIT](RANGE-SPLIT.md) §12). |
| Pause the range drivers | `KV9_RANGE_DRIVER=0` on a node | Split, merge and auto-split wait there; nothing is lost. `KV9_REPLICA_DRIVER=0` is the same brake for replica plans. |
| Back up a keyspace | `backup create --keyspace <name\|id> [--label <t>] [--wait]` | One committed retention point: each range leader captures its image at an exact cut and anchors it, the metadata leader pins the objects and writes one bundle to the bucket, GET-verified ([BACKUP-RESTORE](BACKUP-RESTORE.md)). Needs object storage. A held range refuses splits, merges and plans until it is anchored. |
| Look at backups | `backup list [--all]`, `backup describe <task>`, `backup verify <task>` | Stages `intended → capturing → anchored → complete \| aborted \| retired`. `restorable: no` means a range exceeded the image budget: the data is kept, a restore needs the streamed path (not built yet). `verify` exits 4 when a range's objects fail. |
| Release a backup | `backup retire <task>` | The only release of the pins; the bundle stays as a tombstone. An incomplete backup aborts instead (`task abort`). Refused while a restore of the backup is live. |
| Restore a backup | `restore create --backup <task> --as <new name> [--tenant <id>] [--voters a,b,c] [--wait]`, `restore list`, `restore describe <task>` | A NEW keyspace, never in place: one empty group per backed-up range (placement's least-loaded three stores unless `--voters`), each target copies its range from the anchored image and proves it against the anchor, one transaction publishes the keyspace. Stages `intended → populating → populated → published \| aborted`; abort before publication. The application switches names. |
| Fault points (test only) | `KV9_FAULT_DIR=<dir>` | Every driver step reads `armed.json` there and crashes or pauses once per armed point ([RANGE-SPLIT](RANGE-SPLIT.md) §14). Never set in production. |

## 3. Repair that the cluster does by itself

| Switch (on every node) | What it enables | Verdict and knobs |
| --- | --- | --- |
| `KV9_AUTO_REPLACE_VOTER=1` | The metadata leader replaces a lost metadata voter: promote a caught-up learner, then remove the lost voter ([VOTER-REPLACEMENT](VOTER-REPLACEMENT.md)). | `KV9_VOTER_LOST_SECS` (300), `KV9_VOTER_REPLACE_COOLDOWN_SECS` (600); needs a joined learner to promote. |
| `KV9_AUTO_REPAIR_REPLICA=1` | Each data group's leader replaces a lost voter of its group with a move plan to the least-loaded active store outside the group ([REPLICA-REPAIR](REPLICA-REPAIR.md)). | `KV9_REPLICA_LOST_SECS` (300), `KV9_REPLICA_REPAIR_COOLDOWN_SECS` (600); needs a spare registered store and object storage. |
| `KV9_REPLICA_DRIVER=0` | Disables the plan driver on this node (plans stay committed and resume when it is on). | Default on. |
| `KV9_AUTO_SPLIT_BYTES=<n>` | Each data group's leader splits a bound range at its median key once its replica footprint reaches `n` bytes ([RANGE-SPLIT](RANGE-SPLIT.md) §11). | `KV9_AUTO_SPLIT_COOLDOWN_SECS` (600). Off by default: a split makes the parent unavailable between its seal and the publication. |

Without the switches the verdicts are still taken and shown (`voter_replacement=`,
`replica_repair=` in the status document); only the decision is held back for the operator.

## 4. Storage housekeeping

| Switch | What it does |
| --- | --- |
| `KV9_AUTO_COMPACT_ENTRIES`, `KV9_AUTO_COMPACT_BYTES`, `KV9_AUTO_COMPACT_AGE_SECS` (0 = off) | Automatic Raft-log compaction floors ([AUTO-COMPACTION](AUTO-COMPACTION.md), [BYTE-COMPACTION](BYTE-COMPACTION.md), [AGE-COMPACTION](AGE-COMPACTION.md)). |
| `KV9_RECLAIM_RETIRED=1` | Delete the files of locally retired groups: removed replicas and the sealed parents of published splits and merges, which retire themselves ([STORAGE-RECLAMATION](STORAGE-RECLAMATION.md), [RANGE-SPLIT](RANGE-SPLIT.md) §9). |
| `KV9_MAX_RAFT_LOG_ENTRIES` (65536, 0 off), `KV9_MIN_FREE_DISK_BYTES` | Write back-pressure and disk headroom ([RAW-WRITE-ADMISSION](RAW-WRITE-ADMISSION.md), [RAW-DISK-HEADROOM](RAW-DISK-HEADROOM.md)). |
| `KV9_DATA_CHECKPOINTS`, `KV9_CHECKPOINT_*`, `KV9_OBJECT_*` | Checkpoints to object storage and their budgets ([OBJECT-STORAGE](OBJECT-STORAGE.md)). |

## 5. Observing

- `cluster info`, `cluster health` (exit 0 only when every verdict is ok: quorum margin, lost voters,
  fatal, lag), `cluster status [--node <id>]` (every node's status document), `cluster endpoints`,
  `node list` (raft role, matched index, recent activity, admission, `draining`/`drained`),
  `node describe <id>`.
- `kv9-admin dashboard [--listen 127.0.0.1:9100] [--refresh 3]` serves a read-only web view of the
  same reports: health verdicts, node cards, the placement matrix (data groups × nodes, leader /
  voter / learner), keyspaces with their ranges in key order, operations in flight and backups;
  `/api/state` is the JSON it renders. It binds localhost by default and the token stays in the
  process.
- The status document (`<data-dir>/status`, rewritten once a second) carries one line per
  subsystem: `replica_plans=`, `replica_repair=`, `voter_replacement=`, `node_drain=`,
  `range_split=`, `range_merge=`, `backup=`, `restore=`, `checkpoint_retention=`, `data_groups=`, … each with recent
  events and the last error. `<data-dir>/metrics.json` is the metrics export.
- `task list --kind <n>`: 100 group creation, 101 activation, 102 range binding, 103 migration,
  104 install evidence, 105 truncation, 106 removal, 108 migration abort, 110 compaction,
  111 replica plan, 112 node drain, 113 split intent, 114 split evidence, 115 merge intent,
  116 merge evidence, 117 operation abort, 118 seal evidence, 119 backup intent, 120 backup
  anchor, 121 backup retirement, 122 backup completion, 123 restore intent, 124 restore evidence.

## 6. Security of the control plane

`KV9_ADMIN_PRINCIPALS=<principal>` on the servers restricts every administrative RPC to that
token's principal; application tokens keep the data plane only. Node-to-node control (install
evidence, replica plans, split/merge evidence) travels over the node-authenticated internal
service and is never reachable with a client token.

## 7. Rehearsed

[OPERATIONS-DRILL-2026-10-29](OPERATIONS-DRILL-2026-10-29.md) walks every verb above on a seven-node
cluster, with the transcript, the timings, what it found and what an operator should expect
(a transfer refused once, an Add plan ending at `evidenced`, health `fail` while a node is down).
The same practice is packaged for Claude Code sessions as the project skill `kv9-ops`
(`.claude/skills/kv9-ops/`: the verbs with what to expect, a command cookbook with the drill's
outputs, a stuck-operation checklist, and how to run and extend the drill).

## 8. What is not a primitive yet

**Restore from another cluster, PITR.** Backup and restore within one cluster exist (§2). What
does not exist yet: restoring another cluster's bundle, the fast path for ranges beyond the image
budget, point-in-time recovery, and rebuilding a cluster from the bucket alone
([BACKUP-RESTORE](BACKUP-RESTORE.md) §10, increments 3–5; QUICKSTART §4).

Also missing: an automatic merge of small siblings, load- or time-based split triggers, atomic batches that
cross a range boundary (scans and delete-range do cross it, following `resume_from`), `node
replace`, `node forget` (a drained-and-stopped or dead-and-replaced node stays in the directory and
keeps `cluster health` at `fail` through its `unreachable` verdict) and retention verbs in
`kv9-admin`, and any placement policy beyond "least-loaded active store"
([DEVELOPMENT-PATH](DEVELOPMENT-PATH.md)).
