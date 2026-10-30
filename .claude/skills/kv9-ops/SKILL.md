---
name: kv9-ops
description: Operate, grow, repair, back up and diagnose a running kv9 cluster with kv9-admin and the status documents. Use this skill whenever the user wants to bring up or extend a kv9 cluster, add or remove nodes, create data groups or keyspaces, add or move replicas, transfer leadership, split or merge ranges, drain or retire a node, back up a keyspace, verify or retire a backup, restore a backup into a new keyspace or recover from a bad delete or overwrite, abort an operation, read `cluster health` or a node's `status` file, run or extend the operations drill (`scripts/ops-drill.sh`), or asks why a plan, split, merge, drain, backup, restore or repair is stuck, slow or refused — even when they only say "the cluster", "a node", "kv9-admin" or paste a status line without naming the operation.
---

# kv9 operations

kv9 has no external control plane. Every operator verb is one `kv9-admin` call that commits a
row in the TASKS history; the cluster then drives the operation itself from committed state, on
the node whose role it is, and reports in each node's `<data-dir>/status`. Operating kv9 is
therefore three activities: issue the verb, watch the committed stages, and read the status
document of the node that owns the next step when something does not advance. The runbook is
`docs/OPERATIONS.md`; this skill is the practice that goes with it, learned from the 2026-10-29
drill (`docs/OPERATIONS-DRILL-2026-10-29.md`).

## The words this skill uses

kv9 has its own vocabulary and the verbs, status lines and refusals all speak it. The full list,
with where each word shows up, is `references/glossary.md`; read it when a term below is not
enough, and send the user there when they ask what something means.

- **Metadata group** (region 0): the Raft group holding the catalog and the TASKS history; its
  leader commits every verb. **Voter** / **learner**: a member that votes / one that only
  replicates. A **store** is a node's data directory, identified by its **store incarnation**.
- **Keyspace** → one or more **ranges** `[start, end)`, each bound (kind 102 row) to a **data
  group** (a Raft group of 3/5/7 replicas, named by its **creation task**) in a **region**.
  **Epoch** is the shape version clients carry. A **sealed** binding is the retired parent of a
  published split or merge.
- **TASKS history**: immutable rows by **kind** (100…124); a verb commits an **intent** with an
  **operation id** (derived from `--label`; resubmitting **confirms**); the cluster drives it and
  `describe` shows its **stage**. An operation is **settled** when aborted or complete; one live
  operation holds a group at a time. **Evidence** rows are proofs committed by another node.
- **Replica plan**: `add-replica` (Add, ends at `evidenced`) or `move-replica` (Move, ends at
  `decided`). The leader **captures** an **image** at an exact **cut** (term, index), uploads it
  to **object storage** with a **manifest**, the retention **ledger** **pins** its objects for an
  **owner**, the destination **installs** it, is **promoted**, and for a Move the source is
  **removed**.
- **Split** / **merge**: children on the parent's members, the parent **sealed**, each child
  **populated** and proven by a row **digest**, then **published** in one transaction.
- **Drain**: every leadership handed away, every replica moved off, the node out of the metadata
  group. **Repair** / **voter replacement**: the same moves taken by the cluster for a lost
  store. **Placement**: the least-loaded spare that never held the group.
- **Backup**: a **retention point** of a keyspace: one **anchor** per range (exact cut, image and
  manifest digests, rows and row digest, the **Backup** owner that pins it) plus one **bundle**
  object in the bucket, **complete** once the closure is GET-verified; **retire** is the only
  release. **Restorable** is false when an image exceeded the **image budget**. **Restore**: a
  NEW keyspace from a complete backup: one **target** group per range copies from the anchored
  image, is proven against the anchor, and one transaction publishes the keyspace.

## How to address the cluster

```
KV9_ADMIN_TOKEN=<admin token> kv9-admin --endpoints <any nodes, comma-separated> [--output json] [--yes] [--timeout <s>] <noun> <verb> …
```

- Give several endpoints; the tool resolves the metadata leader and follows redirects. Never
  hand-pick the leader: it changes during drains, restarts and transfers.
- Output is a table on a terminal and JSON when stdout is a pipe. For scripts pass `--output json`
  and read `result`; errors come as `{"error": {"kind", "message"}}`.
- Exit codes: 0 ok, 1 refused (typed, the message says why), 2 usage, 3 transport, 4 partial
  (a catalog readback failed; the message names the family), 6 wait deadline.
- The admin principal is the name before `=` in the servers' `KV9_CLIENT_TOKENS`; with
  `KV9_ADMIN_PRINCIPALS=<name>` set on the nodes, an application token is refused with
  `PermissionDenied`.
- Data-plane work (put/get/scan) goes through `kv9-repl --addr <node> --peers 1=…,2=…` or the SDKs,
  with `KV9_CLIENT_TOKEN`. Give the REPL every node as a peer so it can follow leader hints.

## The verbs, in the order an operator meets them

| Need | Verb | What to expect |
| --- | --- | --- |
| Create a cluster | `kv9 store-prepare` ×N → `kv9 root-create --voters … --store-incarnations …` → `kv9 init` ×N → `kv9 start` ×N | `bootstrap_state=Serving` in every status file within seconds. Never rerun `root-create`/`init` on an existing store. |
| Add a node | `node admit <id> --addr <h:p> --ttl <s>` → on the host `KV9_JOIN_TICKET=<ticket> kv9 join …` then `kv9 start …` | Joins as a metadata **learner**; `node list` shows `learner adm:consumed`. The ticket is one-time; `node revoke <id>` cancels an unused one. |
| Make it vote | `node promote <id> --wait` | `outcome: committed`; repeating answers `confirmed`. |
| Observe | `cluster info`, `cluster health`, `node list`, `cluster status [--node]`, `node describe <id>`; `dashboard [--listen host:port]` for a browser view (placement matrix, ranges, operations, backups) | `health` exits 0 only when every verdict is ok. A stopped or dead node keeps `unreachable=fail` until it is forgotten (no verb yet): read the other verdicts. |
| A data group + keyspace | `group create --voters a,b,c --label <t>` → `keyspace create --name <n> --group <creation-task>` | Three, five or seven *registered* stores (learners count as stores). Routable a second after activation. |
| Move leadership | `node transfer-leader --to <id> --wait`, `group transfer-leader <task> --to <id> --wait` | Refused with `not caught up` if the transferee is behind the commit index; retry once a second, it lands. Transfer to the current leader confirms. |
| Add / move a replica | `group add-replica <task> --to <n>`, `group move-replica <task> --from <a> --to <b>` | One committed plan (kind 111); needs object storage (`KV9_STORAGE=minio` + `KV9_OBJECT_STORE_*`). About 30–40 s each in a debug build. A group never shrinks first. |
| Watch | `group describe <task>` (`plans`, `splits`, `merges` with stages + every replica's live view), `task list --kind <n>`, `task describe <id>` | Plan stages `planned → captured → pinned → evidenced → decided`; **an Add ends at `evidenced`** (only a Move has a removal decision) — completion is the destination in the live voter set. Splits/merges `intended → populating → populated → published`. |
| Split / merge | `group split <task> --at <key>` (`--at-hex`), `group merge <low-task> <high-task>` | Children on the parent's members (four included). The parent refuses reads and writes between its seal and the publication; reads may bounce with `NotLeader` for a second after publication. Membership plans and range operations exclude each other while live; the refusal names the holder. |
| Abort | `task abort <id>` | A split/merge before its first seal, a drain any time, an incomplete backup, a plan through its migration abort (`path: migration abort (no image planned)` when pre-capture). |
| Back up a keyspace | `backup create --keyspace <name> [--label] --wait`, `backup list [--all]`, `backup describe <task>`, `backup verify <task>`, `backup retire <task>` | Stages `intended → capturing → anchored → complete`; one anchor per range with rows and a digest, one bundle object `backups/<cluster>/<task>/bundle`, GET-verified. A held range refuses splits, merges and plans until anchored (seconds for small ranges). `restorable: no` = a range exceeded the image budget: kept, not restorable yet. A complete backup retires, never aborts; retire is refused while a restore of it is live. |
| Restore a backup | `restore create --backup <task> --as <new name> [--voters a,b,c] --wait`, `restore list`, `restore describe <task>` | Always a NEW keyspace (never in place): one group per backed-up range, each copies its range from the anchored image and is proven against the anchor, then one transaction publishes. Stages `intended → populating → populated → published`. The name must be free; the application switches to it. `task abort` before publication; after it the keyspace exists. |
| Retire a node | `node drain <id> --wait --wait-secs <n>` → stop the process | Moves every replica off (one plan at a time, `KV9_DRAIN_PARALLEL`), leaves the metadata group; `node list` → `drained`. Needs, per hosted group, a spare store that **never held that group** (placement never returns a group to a store that held it; initial replicas and removal sources are excluded too). |
| Let the cluster repair itself | `KV9_AUTO_REPAIR_REPLICA=1`, `KV9_AUTO_REPLACE_VOTER=1` on every node | A lost voter (silent `KV9_*_LOST_SECS`, default 300) is replaced by a move plan to the least-loaded spare / by promoting a joined learner. Without the switches the verdicts are still shown (`replica_repair=`, `voter_replacement=`) and the decision is left to you. |
| Pause the drivers | `KV9_REPLICA_DRIVER=0`, `KV9_RANGE_DRIVER=0` on a node | Committed plans wait; nothing is lost. `KV9_FAULT_DIR` is a test hook: never in production. |

Full command sequences with the outputs to expect are in `references/cookbook.md`; read it before
running a sequence you have not run before, and quote its expected outputs back to the user.

## When something does not advance

Work from committed state outward; do not restart nodes on a hunch.

1. `group describe <task>` (or `node list` for a drain): which stage is it in, and what is the live
   voter set? Compare with the table above: an Add at `evidenced` with the destination voting is
   done, not stuck.
2. `task list --kind <n>`: is the row that the next stage needs there (103 migration, 104 install
   evidence, 106 removal, 108 migration abort, 111 plan, 112 drain, 113/114 split intent/evidence,
   115/116 merge, 117 operation abort, 118 seal evidence, 119 backup intent, 120 backup anchor,
   121 backup retirement, 122 backup completion, 123 restore intent, 124 restore evidence)?
3. Read the status line of the node whose role the next step is: `replica_plans=` on the group
   leader (attach, plan_image, capture, promote, decide_removal, remove) and on the destination
   (install, record_evidence); `node_drain=` on the metadata leader; `range_split=`/`range_merge=`
   on the parent's and children's leaders; `backup=` on each range leader (plan_image, capture)
   and the metadata leader (bind, bundle), with per-range `state` and a `waiting` reason;
   `restore=` on each target leader (descriptor, populate) and the metadata leader (publish). Each carries `events` with the last outcome and
   `error`. A first-attempt `install: failed: image not fully uploaded yet` followed by success
   30 s later is normal.
4. Check the known blockers: a live split/merge or plan on the same group (exclusive); no spare
   store the placement may use (drain and repair wait and say so); object storage unreachable
   (every plan stalls at capture/install); the group retired (a sealed binding: split/merge parent —
   it takes no plan); `fatal=` non-empty in a status file (read that node's log).
5. If the operation is wrong rather than slow, `task abort <id>`; the drivers detach and reclaim.

The field-by-field guide to the status document and the capture list for a bug report are in
`references/diagnosis.md`.

## Before touching production

- Put the metadata WAL on a disk with a single-digit-millisecond fsync tail. The Raft tick is
  20 ms and the election timeout 200 ms; a RAID with a 60–100 ms fsync tail re-elected the metadata
  leader every minute under install load in the drill. Nothing broke, but every election is a
  write stall.
- Enable the self-repair switches only with spare stores joined: repair and voter replacement
  need somewhere to go.
- Before a drain, count spares per hosted group (see the drain row). Before a split, know that the
  parent is unavailable between seal and publication (seconds for small ranges).
- Keep `KV9_ADMIN_PRINCIPALS` set; keep application tokens out of `kv9-admin`.

## Rehearsing

`bash scripts/ops-drill.sh` runs every verb above on a local seven-node cluster on MinIO in about
four minutes and prints a `PASS`/`TIMEOUT` line per wait. Run it after changing plans, split, merge,
drain or placement, and on a fast disk (`/tmp`). `references/drill.md` explains its layout, how to
read its log, and how to add a scenario.
