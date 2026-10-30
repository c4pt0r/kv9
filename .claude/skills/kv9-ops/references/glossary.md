# kv9 glossary for operators

What each word means, where you see it (which verb, which status field, which task kind), and
what it relates to. Read the group you need; the SKILL.md table and the cookbook use these words
without re-explaining them.

Contents: 1 cluster and identity · 2 data model · 3 the TASKS history and stages · 4 replica
plans and images · 5 split and merge · 6 drain, repair, replacement · 7 backup and restore ·
8 observing · 9 placement

## 1. Cluster and identity

- **Cluster**: a set of kv9 processes that share one *root descriptor*. There is no external
  control plane; the cluster's own **metadata group** (below) is the control plane.
- **Root descriptor** (`root.bin`): the canonical bootstrap document: cluster id, bootstrap
  generation, the three initial voters and their store incarnations. Written once by
  `kv9 root-create`, copied to every initial voter, never rewritten. Its SHA-256 is the **root
  digest**; every committed row carries it, so a row from another cluster is refused.
- **Node** (`node id`, `NodeId`): one kv9 process. Node ids are operator-chosen integers; 0 means
  "no node" and `u64::MAX` is reserved as the synthetic destination of a backup capture.
- **Store**: a node's data directory. **Store incarnation**: the 16-byte identity minted by
  `kv9 store-prepare` for that directory. A lost disk cannot be recreated under the old
  incarnation: the new disk is a new store, placed through membership. A `(node, incarnation)`
  pair is what rows name when they mean "this exact replica" (`InitialReplica`).
- **Metadata group** (region 0, `META_REGION_0`): the Raft group that holds the catalog (the
  tables below) and the TASKS history. Its **voters** vote and can lead; its **learners**
  replicate without voting. `node list` shows `raft_role` voter/learner; `cluster info` lists
  `meta_voters` / `meta_learners`. The **metadata leader** is the node that commits catalog
  transactions and drives most operations (`cluster info → leader_node_id`).
- **Admission**: `node admit <id> --addr … --ttl …` commits a one-time **join ticket**; the new
  host presents it to `kv9 join` and `kv9 start`, which registers the node as a store and makes
  it a metadata learner. `node revoke` cancels an unused ticket; `node promote` makes a learner a
  voter; `node remove` drops a voter (never below three).
- **Endpoint**: the address a node serves on, recorded in the node directory with a
  **generation** that increments on `change-node-endpoint`. `cluster endpoints` lists them.
- **Tokens**: `KV9_BOOTSTRAP_TOKEN` (formation), `KV9_CLUSTER_TOKEN` (node-to-node),
  `KV9_CLIENT_TOKENS=principal=token,…` (what clients may present). The **admin principal** is
  the principal named by `KV9_ADMIN_PRINCIPALS`; `kv9-admin` must present its token.
- **SDK**: the client libraries (Rust, Python, TypeScript, C++) on the public gRPC service;
  `kv9-repl` and `kv9-admin` are built on the Rust one, whose `admin` module is the operator
  surface. See `sdk.md`.
- **Peers map**: `node id → address` given to an SDK or the REPL so it can follow `NotLeader`
  hints to the leader.
- **Receipt**: the exact applied `(term, index)` a write returns; compare the pair.
- **Internal service**: node-authenticated RPCs between nodes (evidence delivery, install
  evidence, plan steps). Never reachable with a client token.

## 2. Data model

- **Keyspace**: the unit an application writes to (`use orders` in the REPL): an id, a name, a
  tenant. A keyspace's keys live in one or more **ranges**.
- **Tenant**: an owner id attached to keyspaces; tenant 0 is the default.
- **Region** (`RegionId`): one Raft group's id. Region 0 is the metadata group; data regions
  start at 100.
- **Data group**: a Raft group of 3, 5 or 7 replicas that hosts one range. Created by `group
  create --voters a,b,c` (operator) or by a split, merge or restore (children/targets). Its
  **creation task** (the kind-100 row's task id) is how every verb names it: `group describe
  <creation task>`, `group split <creation task> …`.
- **Replica**: one copy of a data group on one store. A replica is a **voter** (votes, may lead)
  or a **learner** (catching up). A group's **leader** serves reads and writes for its range.
- **Range binding** (kind 102): the row that binds a keyspace's key interval `[start, end)` to a
  data group. `group list` shows a group's keyspaces with `start`/`end` (hex) and `sealed`.
  Empty `start` means "from the beginning", empty `end` means "to the end".
- **Descriptor** (`DataRange`): the binding as the group itself holds it in its log: keyspace,
  region, bounds, **epoch**. The group's **fence** admits only writes inside its descriptor.
- **Epoch** (`conf_ver` / `version`): the version of a range's shape; clients carry it and get a
  typed stale-epoch error when a split or merge changed the shape, then refresh.
- **Sealed binding**: a binding whose range was split or merged; publication seals the parent
  row in place. A sealed binding's group is **retired**: it serves nothing, takes no plan, and
  (with `KV9_RECLAIM_RETIRED=1`) frees its files. `group list` keeps it as history.
- **Routing**: a REGIONS row per live range lets `get_region` map a key to a region and its
  leader. A client that asks a non-hosting node gets `NotLeader` with a committed member as
  hint; scans that cross a range boundary continue from `resume_from`.

## 3. The TASKS history and stages

- **TASKS history**: the append-only table of committed rows that drive everything; `task list
  --kind <n>`, `task describe <id>`. Every verb that changes the cluster commits one row and
  returns its **task id**. Rows are immutable; later rows reference earlier ones.
- **Task kind**: the row type. 100 group creation · 101 activation · 102 range binding ·
  103 migration intent · 104 install evidence · 105 source truncation · 106 source removal ·
  108 migration abort · 110 compaction · 111 replica plan · 112 node drain · 113 split intent ·
  114 split evidence · 115 merge intent · 116 merge evidence · 117 operation abort · 118 seal
  evidence · 119 backup intent · 120 backup anchor · 121 backup retirement · 122 backup
  completion · 123 restore intent · 124 restore evidence.
- **Intent**: the row an operator's verb commits (a plan, a split, a drain, a backup…). The
  cluster drives it from there; the operator watches.
- **Operation id**: the 16-byte identity of an operation, derived from `--label` (or given with
  `--operation <hex>`, or minted). Resubmitting the same operation **confirms** instead of
  creating a second one (`outcome: confirmed`); every row of the operation carries it.
- **Stage**: a catalog-only summary of how far an operation got, derived from which rows exist
  (`group describe → plans[].stage`, `backup describe`, `restore describe`). Plans: `planned →
  captured → pinned → evidenced → decided` (Add stops at `evidenced`); splits/merges: `intended →
  populating → populated → published`; backups: `intended → capturing → anchored → complete`;
  restores: `intended → populating → populated → published`; plus `aborted` and (backups)
  `retired`.
- **Settled**: an operation that no longer holds its group: aborted, or complete for its kind
  (a plan: evidenced, and decided if it is a Move). One live operation binds a group at a time;
  the refusal names the holder.
- **Abort**: `task abort <task>`: a kind-117 row (split/merge/drain/backup/restore) or a kind-108
  row (a plan, through its migration). Allowed only before the point of no return: a split's or
  merge's first seal, a restore's publication, a backup's completion.
- **Evidence**: a row a *different* node commits to prove a step happened with the right content:
  install evidence (104, the destination installed this exact image), split/merge evidence
  (114/116, the child holds exactly the parent's half, digest and row count), seal evidence
  (118), backup anchor (120), restore evidence (124). Evidence is delivered to the metadata
  leader over the internal service.

## 4. Replica plans and images

- **Replica plan** (kind 111): `group add-replica` (mode Add: the group grows by one voter) or
  `group move-replica` (mode Move: grow, then remove the source). Driven by the group leader,
  the destination and the metadata leader, each doing the steps that are its role.
- **Migration** (kind 103): the underlying intent "this group gets a replica on store D"; a plan
  wraps one. The legacy `kv9 client migrate-*` verbs drive migrations by hand.
- **Capture** / **cut**: the group leader freezes its engine at an exact applied position
  `(term, index)`, the **cut**, together with the Raft configuration at that cut, and writes the
  state as SST files. The result is an **image**; its content hash is the **image digest**.
- **Manifest**: the list of an image's objects (keys, sizes, digests) plus its scope (cluster,
  region, epoch). The **closure** is every object the manifest references; "GET-verified" means
  each was fetched and its digest checked.
- **Object storage** / **bucket**: the S3-compatible store (`KV9_STORAGE=minio`,
  `KV9_OBJECT_STORE_*`) where images, checkpoints and backup bundles live. Required by plans,
  split, merge, backup and restore.
- **Checkpoint**: the same mechanism used by a group for itself (`KV9_DATA_CHECKPOINTS`), so a
  replica can reopen from object storage plus its WAL tail.
- **Retention ledger**, **owner**, **pin**: the replicated table that says which operation needs
  which objects. An **owner** (kinds Snapshot, Migration, Backup, Publisher…) **pins** a
  closure; phases `Held → Published → Quiesced → Released`. Nothing pinned may be deleted; GC is
  not enabled yet, so today pins are bookkeeping that a future GC will honour. "Bind" in a plan
  or backup means "publish the owners for this image".
- **Install**: the destination writes the image into its store and joins the group as a
  learner at the cut; then **record evidence** (104). A first `install` attempt logging `image
  not fully uploaded yet` and succeeding 30 s later is the normal shape.
- **Promote** / **decide** / **remove**: the destination becomes a voter; for a Move the leader
  commits the removal decision (106) and then removes the source replica.
- **Capture slot** (`capture.plan` in a group's directory): one durable capture per group at a
  time; freed when the operation that owns it settles (install evidence, anchor, or abort).

## 5. Split and merge

- **Split** (`group split <task> --at <key>`, kind 113): two **children** are created on the
  parent's members (four included), the parent is **sealed** (one-way, through its own log),
  each child **populates** its half from a local sealed parent replica in 512-row pages and
  proves it with a chained **digest** (114), and one catalog transaction **publishes**: the parent
  binding sealed, two child bindings inserted, routing updated.
- **Merge** (`group merge <low> <high>`, kind 115): the same pipeline backwards for two adjacent
  siblings on one store set; one child.
- **Seal window**: between the parent's seal and the publication the range refuses reads and
  writes; clients retry. Seconds for small ranges.
- **Auto-split** (`KV9_AUTO_SPLIT_BYTES`): a group leader splits its range at the median key
  when its footprint exceeds the threshold; off by default.
- **Child descriptor**: the half a child carries before publication, proposed by its leader.

## 6. Drain, repair, replacement

- **Drain** (`node drain <id>`, kind 112): the node hands away every leadership, the metadata
  leader moves every replica off the store one Move plan at a time (`KV9_DRAIN_PARALLEL`),
  promotes a learner before removing a voter from a three-voter metadata group, and `node list`
  reports `drained`. The process is stopped by the operator.
- **Replica repair** (`KV9_AUTO_REPAIR_REPLICA=1`): a group leader replaces a voter silent for
  `KV9_REPLICA_LOST_SECS` with a Move plan to the least-loaded spare.
- **Voter replacement** (`KV9_AUTO_REPLACE_VOTER=1`): the metadata leader promotes a caught-up
  learner and removes a metadata voter silent for `KV9_VOTER_LOST_SECS`.
- **Leader transfer** (`node transfer-leader`, `group transfer-leader`): hands leadership to a
  caught-up voter (one holding every committed entry).

## 7. Backup and restore

- **Backup** (`backup create --keyspace`, kind 119): a **retention point** of one keyspace: one
  **anchor** per range plus a **bundle** in the bucket. Consistency is per range (kv9 has no
  cross-range atomic batch, so there is nothing else to be consistent about).
- **Anchor** (kind 120): one range's exact cut, image and manifest digests, object count and
  bytes, the row count and a chained **row digest** computed from the image, and the owner that
  pins it. `restorable` is false when the image exceeded the **image budget** (the memory a
  node may spend restoring an image); the data is kept, a restore needs the streamed path.
- **Bundle** (`backups/<cluster-id>/<task>/bundle`): one self-describing object: provenance,
  the keyspace's shape with boundaries, every anchor and capture descriptor. Written after every
  anchor, GET-verified before the **completion** row (122) makes the backup `complete`.
- **Retire** (`backup retire`, kind 121): the only release of a backup's pins; the bundle stays
  as a tombstone. Refused while a restore of the backup is live.
- **Restore** (`restore create --backup --as <name>`, kind 123): a NEW keyspace, never in place:
  one **target** group per backed-up range, each copies its range from the anchored image
  through its own log and is proven against the anchor (124); one transaction publishes the
  keyspace. The application switches names.

## 8. Observing

- **Status document** (`<data-dir>/status`, `cluster status`): rewritten once a second by each
  node; `role`, `leader_id`, `term`, `applied_index`, `fatal`, `data_groups` and one JSON
  **subsystem line** per driver (`replica_plans=`, `node_drain=`, `range_split=`, `backup=`,
  `restore=`…) with `events` (the last outcomes), `in_flight`, `waiting` and `error`.
- **Verdicts** (`cluster health`): quorum margin, lost voters, auto-replacement, fatal,
  unreachable, apply lag, group control error, admissions expiring; `overall` is `ok` only when
  every verdict is. A stopped node stays `unreachable=fail` (no `node forget` yet).
- **Exit codes** of `kv9-admin`: 0 ok, 1 refused (typed, the message says why), 2 usage,
  3 transport, 4 partial (a readback or a verification failed in part), 6 wait deadline.
- **Fault point** (`KV9_FAULT_DIR`, test only): a named place in a driver step where a test can
  crash or pause the process; the fault matrix walks every one.

## 9. Placement

- **Least-loaded**: the active, non-draining registered store hosting the fewest groups. Used by
  repair, drain and restore defaults.
- **Never again**: placement never returns a group to a store that held it (initial replicas and
  removal sources included), so a drain or repair needs one never-hosting spare per group.
- **Spare**: a registered store (a metadata learner is enough) that hosts no replica of the group
  in question.
