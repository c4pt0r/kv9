# Metadata replica modes: full copy or scoped catalog

Design packet, 2026-10-29. Every node stays a peer (same binary, same verbs, same data-group
role); what differs is a per-node choice of how much of the metadata it keeps. Today every node
is either a metadata voter or a metadata **learner** that replicates the whole catalog and the
whole TASKS history through the metadata Raft group. This packet adds a second mode in which a
node keeps only the **scoped** of the catalog its own replicas need, and is not a member of the
metadata Raft group at all.

> **Status: implemented 2026-10-30** (increments 1–4 of §8, and per-region scopes). §10 records
> what was built and where it departs from the design: lineage is followed upward only, and the
> retention ledger stays on full nodes. Runbook: docs/OPERATIONS.md, "Scoped nodes".

## The problem

The target is clusters of hundreds of nodes. The metadata group's log goes to every node. With tens of nodes that is tens of Raft followers
behind one metadata leader: every catalog commit is replicated to all of them, every heartbeat
fans out to all of them, and every node stores and replays the whole history, most of which
concerns groups it will never host. The data plane does not need it: a node serves a group it
hosts from the group's own descriptor and fence (`raw_directory.get_for`, the group's `DataRange`
in its own log), and consults the catalog only to redirect a request for a key it does not host.

What a node does need from the catalog, found by reading every local metadata read in the
server (about 180 call sites):

| Need | Rows | Who reads them |
| --- | --- | --- |
| Which groups to prepare, activate, retire | group creations (100), activations (101), bindings (102), membership (103/104/106/108), sealed bindings | `RegionManager::recover`, group reconcile, `reconcile_retirement` |
| Driving its own plan steps as a destination | the plan (111), migration (103), capture descriptor, the retention owners for that image | replica-plan driver: install, record evidence |
| Split / merge / restore children it hosts | intents (113/115/123), evidence, seal evidence, aborts (117) | range drivers, restore driver |
| Data checkpoints of its groups | retention ledger rows for its groups' images and owners | data checkpoint worker, checkpoint sweep |
| Talking to peers | node directory and endpoints (address, generation, active) | transport `register_catalog_peer`, registration |
| Redirecting a request for a key it does not host | keyspace row, the covering binding, its committed members | request gate (`check_context_in`, `data_range_member_hint`) |
| Identity and trust | the certified root | everywhere |

Everything else — other groups' plans, other keyspaces, backup and restore rows of ranges it does
not host, drains of other nodes, the whole TASKS history — is irrelevant to a node that will never
lead the metadata group.

## 1. The two modes

The mode is chosen at admission (`node admit --meta auto|full|scoped`, §9) and recorded there:

- **full** — today's behaviour: the node joins the metadata group as a learner (and may be
  promoted to voter). It holds the whole catalog and history and can be a metadata voter,
  leader, or voter-replacement candidate.
- **scoped** — the node joins as a store but **not** as a metadata Raft member. It keeps a local,
  read-only **scoped catalog**: the rows in the table above for the groups it hosts or is a
  destination of, plus the root and the directory rows of its own peers (§3a). It can host any data group,
  be a plan destination, a split/merge/restore child, a drain target; it cannot be promoted to a
  metadata voter (`node promote` refuses, voter replacement never picks it).

Both modes run every data-plane and data-group code path; the difference is only where the catalog
rows come from.

## 2. The scope and its invariant

The scope is defined by closure, not by a list of tables:

> **S1 — closure (per region, 2026-10-30).** A node's **anchors** are the regions it hosts,
> is attached to, or is named as a replica, destination or source of, closed **upward** over
> lineage: a split's or merge's child brings its parents, a restore target its source range.
> Its scope holds the anchors and every region a row about an anchor names, **one hop** (a
> parent's split names both children), and **every** committed row about those regions: their
> creations, bindings, migrations, captures, evidence, aborts, removals, seals, the intents
> that name them and the rows those reference. Rows about a whole keyspace (backup and restore
> intents, backup completions and retirements) are delivered where they touch the scope and
> never widen it. Plus the keyspaces and tenants of those rows, the root, the node's own
> directory, admission and drain rows, and the directory and admission rows of the full nodes
> and of every store those rows name. Retention-ledger rows are not in any scope (§10).
>
> Lineage is followed up, not down: a region needs its ancestors' history, not its siblings'
> descendants, so a slice holds a region's ancestor chain and the siblings along it — about
> 2 × depth + 1 regions — and never a whole keyspace that has been split across nodes.
> (Between 2026-10-30's first build and this rule the closure was per keyspace; §10 says why
> that changed.)

S1 is what makes the existing readers safe on a scope. Most catalog readers infer from absence:
"no abort row ⇒ not aborted", "no evidence ⇒ not installed", "no sealed binding ⇒ live". Absence
is only meaningful if the reader is looking at everything that could be present for the operation
in question. A scope that is closed per region preserves exactly that for every decision a scoped
node is allowed to take, and S2 forbids the others:

> **S2 — scope.** A scoped node never takes a decision about a region outside its scope. Every
> leader-only path (plans' metadata steps, drains, backup bundles, publications, verbs) already
> runs only on the metadata leader, which is always a full node. The request gate on a scoped node
> answers a key outside its scope with a redirect to a full node, never "not found".

> **S3 — freshness.** A scope carries the metadata applied position `(term, index)` it reflects
> and only moves forward. A scoped node acts on its scope exactly as a learner acts on its applied
> catalog today: possibly behind, never ahead, never inconsistent within one position.

## 3. How a scoped node gets its rows

A scoped node subscribes to any **full** node (metadata voter or learner) with a long-lived
internal RPC, `CatalogScope(from_position, regions)`:

1. The serving node replays its applied catalog changes after `from_position` and forwards each
   committed batch **filtered** to the subscriber's scope; a batch that adds a region to the scope
   (a new plan naming this node, a split child placed here) also carries that region's closure as
   of that position. Every message carries the position it brings the scope to.
2. Every subscription, a reconnect included, starts with a **scope snapshot** at the serving
   node's current position, then continues with changes (as built: resuming mid-stream from a
   position would need the server to know which keys the subscriber holds, and sending those is
   as large as the snapshot; a snapshot is O(scope) and rate-limited by
   `KV9_SCOPE_SNAPSHOT_PARALLEL`). The retained history (the change ring) serves the deltas of
   a live stream; a stream that falls behind it gets a fresh snapshot.
3. The subscriber writes each message into its local catalog store (the same `MetaStore` the
   readers use) in one batch with the new position, so a restart resumes from the last position
   it persisted. The store is never written by anything else on a scoped node.
4. The subscription's region set is computed by the **serving** node from the subscriber's
   committed facts (its store incarnation as replica, destination, child or target), not taken on
   trust from the subscriber; the subscriber may only ask for a superset to prefetch.

Any full node can serve, so the fan-out is no longer all on the metadata leader: scoped nodes can
subscribe to learners, which spreads the load and lets the metadata Raft group stay at 3–7
members regardless of cluster size.

Trust: the stream travels on the node-authenticated internal service (cluster token + node id),
like every other node-to-node RPC; each batch carries the root digest and is refused if it differs.

## 3b. How a change is routed to scoped nodes

The serving full node applies the metadata log like any learner, then, for each committed batch,
answers one question: **which scoped nodes' scopes does this batch touch?** It does so with two
derived indexes kept in memory and rebuilt from the catalog on restart:

1. **Row → regions.** Every catalog row maps to the region(s) it concerns, by its kind:

| Row | Regions |
| --- | --- |
| group creation (100), activation (101) | the group's region |
| range binding (102), REGIONS row | the bound region |
| migration (103), plan (111), install evidence (104), truncation (105), removal (106), migration abort (108), compaction (110) | the migration's region (rows that name only an operation resolve it through the migration index) |
| capture descriptor (keyed by operation) | the operation's region (migration or backup range) |
| retention owner, pin, publisher rows | the region in the owner descriptor |
| split (113), merge (115), restore (123) intents | parent region(s) and every child or target region |
| split / merge / restore evidence (114/116/124), seal evidence (118), range abort (117) | the regions of the operation they name |
| backup intent (119), anchor (120), completion (122), retirement (121) | the backed-up regions |
| node directory row of store S | every region S is a member of (R3: peers only) |
| drain (112) of store S | none: delivered to S itself |
| keyspace row | the keyspace's bound regions |

2. **Region → subscribers.** For every region, the scoped nodes that must see it: its committed
   members, a learner being attached, a plan destination, a split/merge child's or restore
   target's members. This index changes only when one of those rows commits, and it is updated
   **in the same step** as the batch that changes it.

Routing one batch: `regions = ⋃ row_regions(row)`, `recipients = ⋃ subscribers(region) ∩
my_subscribers`, send the batch's rows (filtered to those regions) to each recipient with the new
position. Cost is proportional to the recipients, not to the cluster.

**A node enters a region's scope** when a batch adds it to `subscribers(region)` — a plan names
it as destination, a split places a child on it, a restore targets it. That batch is sent to it
with the **closure** of the region at that position (every row the index maps to the region),
so the node's scope for the new region starts complete (M1). **It leaves** when a removal, drain
or retirement takes it out; the server sends a "drop region R" marker and the node deletes those
rows once its own replica of R is retired.

**Positions without gaps.** Every message carries the metadata applied position it brings the
scope to. A batch that touches none of a node's regions still advances its position, coalesced: the
server sends a bare position update at most every `KV9_SCOPE_IDLE_MS` (default 1000) — a few
bytes per second per subscriber, not the batch. The subscriber therefore always knows how far
behind it is, and a reader that waits for "my scope is at least at position P" (a destination
waiting for the pins its plan just committed) can wait for exactly that.

**Client requests are routed as today.** The REGIONS rows live on full nodes; a client that asks a
scoped node for a key in its scope is served by the group as now; for a key outside its scope
the node redirects to a full node (which answers with the group's members), never "not found".

## 3a. Scale: no metadata change reaches every node

With hundreds of nodes, any change that every node must receive is a broadcast storm, and so is
any per-node work that every node repeats on every change. The design rules, each enforced:

- **R1 — bounded full set.** The full nodes are the metadata voters (3, 5 or 7) plus at most
  `KV9_META_FULL_LEARNERS` learners (default 4). The metadata Raft group's fan-out is therefore at
  most 11 followers, whatever the cluster size. `node admit` defaults a new node to `scoped` once
  the full set is complete; `--meta full` beyond the cap is refused.
- **R2 — routed, not broadcast.** A committed catalog change is delivered only to the scoped nodes
  whose scope it touches. Almost every frequent change is per region (plans, captures, evidence,
  retention-ledger updates of one group's checkpoints), so it reaches that region's 3–7 replicas
  and destinations. A full node keeps an index region → subscribers, so routing one change costs
  O(recipients), never O(nodes).
- **R3 — no global rows in scopes.** The node directory is not replicated to scoped nodes. A scoped
  node holds the directory rows of the stores it shares a region with (in its closure, so a
  member's address change reaches exactly the nodes that talk to it); any other address is looked
  up on demand from a full node and cached with its endpoint generation; a stale generation is
  refused by the peer and triggers one re-lookup. Drain rows go to the drained node only.
- **R4 — spread serving.** Each scoped node subscribes to one full node chosen by rendezvous hash
  of its node id over the full set, so subscribers spread evenly and a full node serves about
  N / F of them. When a server goes away, only its subscribers move, each to its next hash
  choice, after a jittered backoff; snapshot catch-up is rate-limited per server
  (`KV9_SCOPE_SNAPSHOT_PARALLEL`), so a server loss is not a reconnect storm.
- **R5 — push, coalesced, one stream.** One long-lived stream per scoped node; the server
  coalesces consecutive changes for the same subscriber into one message; no polling of the
  catalog by scoped nodes.
- **R6 — liveness without the catalog.** Heartbeats are not catalog writes (they are not today
  either): a store's liveness is what its data groups' Raft progress shows its group leaders, and
  what the scope stream shows its server. Nothing per node is committed periodically.
- **R7 — bounded observation.** `cluster status`, `cluster health` and the dashboard fan out to
  every node today; at hundreds of nodes they must page through nodes and read scope lag from the
  serving full nodes instead of asking every node every poll.

Steady-state cost, with N nodes, F full nodes, G groups of R replicas and c catalog changes per
second spread over the groups: the metadata leader sends c × (V + learners) Raft appends (constant
in N); each full node routes about c × R / F messages to subscribers; each scoped node receives
only the changes of its own groups. Today the same cluster sends c × N appends from one leader.

## 4. What changes elsewhere

- **Admission.** `AdmittedRole` gains `Scoped`; `node admit <id> --addr … [--meta scoped]` records
  it, `kv9 join` / `kv9 start` with a scope ticket registers the store without adding a metadata
  learner. Changing modes later is a re-admission (drain, then admit again).
- **Membership verbs.** `node promote` refuses a scoped node; voter replacement and the drain's
  "promote a learner first" step ignore scoped nodes; `node remove` does not apply (the node is not
  a member). `node list` shows `meta: full|scoped` and, for scoped nodes, the scope position and lag.
- **Placement.** Unchanged: scoped nodes are stores like any other.
- **Redirects.** The request gate on a scoped node redirects a key outside its scope to a full
  node; clients already follow redirects.
- **Status and health.** A `catalog_scope=` status line (serving node, position, lag, regions,
  last error); `cluster health` gains a scope-lag verdict.
- **Admin RPCs on a scoped node** answer `NotLeader` with the metadata leader as hint, as any
  non-leader does today.

## 5. Invariants

- **M1** (S1) — on a scoped node, every committed row that could change a decision about a region
  in its scope is present in the scope at the scope's position.
- **M2** (S2) — a scoped node takes no decision about a region outside its scope, and never
  becomes a metadata voter or leader.
- **M3** (S3) — scope positions are monotonic and every scope state equals the full catalog's
  restriction to the scope at that position.
- **M4** — the metadata Raft group's size and traffic do not depend on the number of scoped nodes,
  and every committed change reaches at most the full nodes plus the scoped nodes whose scope
  contains it (R1, R2); no row is in every scope (R3).

## 6. What it does not do

- Make a scoped node a metadata voter (re-admit it as `full`).
- Partial-copy the metadata group's own Raft log on full nodes: full nodes stay full.
- Hide catalog contents for security: a scope is an efficiency boundary, not an access boundary
  (any node can still ask a full node; the internal service is trusted).

## 7. Evidence

- **Written first:** `proofs/lean/catalog-scope` (`scripts/prove-catalog-scope.py`), sixteen
  theorems and seven mutation controls: scoped verdicts equal full-catalog verdicts for every
  scoped region (M1), nothing beyond the catalog (M3), no rows for unrelated changes (R2),
  out-of-position and duplicate messages are no-ops, monotonic positions, fail-closed on an
  undeclared row kind. The implementation pins its sources into this model's contract.
- Lean model (remaining): catalog positions, scopes, region membership changes; M1 (closure preserved under
  every committed change), M3, and that a scope reader's absence verdict equals the full reader's
  for regions in the scope; controls for a scope that drops an abort row, a subscription that
  skips a closure on region add, a non-monotonic position.
- Scale run: 3 voters, 4 full learners and 200 scoped nodes on one host (small groups, debug
  build): the metadata leader's send rate stays flat as scoped nodes are added; one full node is
  killed and its subscribers move without a snapshot spike above the configured parallelism.
- e2e on MinIO: three full voters, three scoped nodes; the metadata group keeps three members;
  data groups placed on scoped nodes; add/move/split/merge/drain/backup/restore with scoped nodes as
  sources, destinations and children; kill a serving node mid-stream (the subscriber resumes from a
  learner); restart a scoped node behind the retained history (snapshot path); a request to a scoped
  node for a key it does not host is redirected.
- A run of the existing fault matrix and drill with half the nodes in scoped mode.

## 8. Increments

1. **Admission and membership.** `AdmittedRole::Scoped`, `--meta scoped`, join without a metadata
   learner, promote/replacement refusals, `node list` mode. Scoped nodes cannot host groups yet.
2. **The scope stream.** `CatalogScope` RPC on full nodes (changes + snapshot), the subscriber that
   writes the local store, positions, the status line and health verdict, redirects outside the
   scope.
3. **Hosting.** Placement and every data-group driver on scoped nodes; the closure rules for plans,
   split/merge/restore children and retention rows; the e2e and the model.
4. **Scale acceptance.** Many scoped nodes behind a three-voter metadata group; measure the
   metadata leader's load against the full-learner baseline.

## 9. Decisions (2026-10-29, with the owner)

- **Default mode for new nodes:** the scoped mode once the full set is complete. `node admit`
  admits a new node as a full learner while the full set has room, and in the scoped mode after
  that; `--meta full` beyond the cap is refused unless the cap is raised.
- **Size of the full set:** `KV9_META_FULL_LEARNERS` defaults to 4 and is the operator's to change
  explicitly; no other minimum is enforced.
- **Name:** `scoped` (`KV9_META_REPLICA=full|scoped`, `node admit --meta scoped`, status line
  `catalog_scope=`); the rows a scoped node holds are its **scope**.
- **Closure granularity (2026-10-30):** per region with upward lineage (S1). A first build was
  per keyspace; the owner asked for per-region scopes, which the slice-tolerant readers of §10
  make possible.
- **Mode selection:** at admission only (`node admit --meta`), recorded in the admission row and
  the data-dir marker; there is no `KV9_META_REPLICA` start switch (a node cannot choose a mode
  the cluster did not admit it in).

## 10. As built (2026-10-30)

**Admission.** `node admit <id> --addr <a> [--meta auto|full|scoped]` (`AdmitNodeRequest.meta_mode`,
`AdmittedRole::Scoped = 2`). `auto`, the CLI default, admits `full` while voters' learners plus
pending full admissions stay under `KV9_META_FULL_LEARNERS` (default 4) and `scoped` after; an
explicit `full` beyond the cap is refused. The SDK's `admit_node` keeps `full`; `admit_node_mode`
chooses. Registration of a scoped admission adds no metadata learner (a leftover learner entry is
removed, a voter is refused) and marks the node's row Active as the receipt, which says `scoped`.
`node promote` refuses a scoped node. `node list` has a `META` column (`meta_mode` in JSON).

**The joiner.** On a scoped receipt the node writes `kv9-meta-scoped` in its data-dir, starts
the subscriber and opens data-group Raft ingress; it never starts the metadata Raft owner. It
Serves once its slice is at or past the receipt's index and shows its own row Active. A restart
is recognised by the marker, the init marker and that row; the startup checks that bind the
metadata engine's position to the metadata Raft log (committed term, log budget) do not apply to
a slice, whose positions come from the stream. Endpoint recovery and catalog waits use the
slice position (`SliceWatch`) instead of the metadata driver.

**The closure, per region.** `crates/meta/src/scope.rs` (`classify`, `closure`, `route`,
`snapshot_rows`) and `crates/meta/src/data_groups/subject.rs` (`task_subject`, one decoder per
declared kind, with each row's lineage edges and whether it is an umbrella row) are the whole
rule. The router's widening test (`Closure::widened_by`) is the closure's own growth rule, so a
snapshot's rows never ask for another snapshot; a unit test checks that fixpoint over every
committed row of a two-level split tree.

**Readers on a slice.** The subscriber writes a local marker (`\0kv9\0catalog-slice\0`,
never streamed) with every snapshot, and `MetaTxn::is_slice` tells readers they are on one.
`committed_ranges` then keeps its per-row rules and the no-overlap rule and drops the
"unsealed ranges cover the keyspace" rule (a slice has gaps: other nodes' ranges). Every other
reader was audited: the split and merge planners skip intents whose parents they do not hold
and always have both children's creations (a delivered intent brings them); backup and restore
planners act on led and local ranges only and look nothing up for the others; anchors,
completions, retirements and restore evidence are always delivered with their intents; the
request gate answers a key in a gap with a redirect. The first per-keyspace build is why S1 was
once per keyspace: those readers judged whole keyspaces.

**Per-region scopes, measured** (`scripts/meta-scoped-tree.sh`): one keyspace split into a
balanced tree, every leaf moved onto its own triple of scoped nodes (moves run in parallel).

| Leaves | Depth | Regions in the keyspace | Regions per leaf host (2 × depth + 1) | Rows per leaf host | Full catalog keys |
| --- | --- | --- | --- | --- | --- |
| 8 | 3 | 15 | 7 | 98 | 3 731 |
| 16 | 4 | 31 | 9 | 109 | 7 790 |

The e2e's phase J2 checks the same rule on a smaller tree with backup, restore and drain
afterwards. Two move-plan defects surfaced while placing leaves and are fixed: a group leader
moving off itself now hands leadership to the destination before the removal ("a leader cannot
remove itself"), and a destination's install resubmits a ledger request whose outcome came back
unknown (`ResubmittingLedger`), because each failed attempt spends one of the eight retained
install generations for good.

**What stays on full nodes.** The retention ledger (`\0kv9\0retention_v1\0…`), the task-history
stamp, id sequences, SST and quota rows, the metadata group's own manifest keys, engine-owned
singletons and the data of keyspaces the metadata group serves itself. A scoped node reads a
retention owner from the metadata leader through the routed ledger client (`retention_owner_for_mode`):
the replica-plan driver's "pinned" fact and the source's check that both image owners are
published before upload.

**Fail-closed.** A key that no rule declares stops the stream with an error naming it (the
`catalog_scope_error=` status line); nothing undeclared is routed or dropped silently. The
totality unit test requires every key a data keyspace and a drain write to be declared, as a put
and as a delete; the e2e streams every key its verbs write (moves, captures, retention, splits)
through the same classifier, and any undeclared one would stop it.

**The stream.** `Kv9Raft.CatalogScope` on the node-authenticated service; served only by a node in
the metadata configuration, only to the registered owner of the named store, only under the
certified root. A change ring (`ApplyObserver` on the metadata state machine, bounded by
`KV9_SCOPE_RING_ENTRIES` 8192 and `KV9_SCOPE_RING_BYTES` 64 MiB) holds its lock across the apply
write, so a snapshot's engine view and position are one instant. Per subscriber: a snapshot
first (parts of at most 1 MiB, then an end frame with the position, the scope's regions, the full
set with addresses and the leader), then deltas that name the position they apply on, coalesced,
plus an idle delta every `KV9_SCOPE_IDLE_MS` (1000). An op that widens the closure, a full-set
change or a position the ring no longer holds sends a fresh snapshot instead. A row that leaves
the scope, or a delete of a value-routed row, is deleted where the subscriber holds it.

**The subscriber.** One stream to one full node, chosen by rendezvous hash over the full set
(R4), failing over with jitter after an error or three idle intervals of silence. A snapshot
older than the slice is refused; a delta whose position is not the slice's ends the stream and
resubscribes; a snapshot at the slice's own position replaces it without moving the position.

**Redirects.** On a scoped node a keyspace or range not in the slice is answered with
`NotLeader` naming the metadata leader. The metadata driver of a scoped node never runs, so it
reports the stream's leader as its hint (`set_external_leader_hint`), which also routes evidence,
plan and ledger RPCs to the leader.

**Status lines.** Full node: `meta_mode=full`, `catalog_ring_last`, `catalog_scope_streams`,
`catalog_scope_frames_sent`, `catalog_scope_bytes_sent`, `catalog_scope_snapshots_sent`,
`catalog_keys`. Scoped node: `meta_mode=scoped`, `catalog_scope=current|stale`,
`catalog_scope_index`, `catalog_scope_rows`, `catalog_scope_regions`, `catalog_scope_region_ids`,
`catalog_scope_server`, `catalog_scope_leader`, `catalog_scope_snapshots`, `catalog_scope_deltas`,
`catalog_scope_refused`, `catalog_scope_frame_age_ms`, `catalog_scope_error`,
`catalog_scope_bytes_received`.

**Evidence.**
- Unit: `crates/meta/src/data_groups/subject.rs` tests (a scope is its keyspaces, tenants and the
  nodes it names; a snapshot holds nothing of another keyspace; totality; routing of unrelated,
  widening, drain and delete changes); the change ring's tests in
  `crates/server/src/catalog_scope/ring.rs`.
- e2e on MinIO, `scripts/meta-scoped-e2e.sh`: three voters, scoped nodes 4–6 join with no
  metadata Raft owner; a group on the voters stays out of their slices; a group on 4–6 alone
  activates, binds a keyspace and serves raw traffic through a scoped node; each slice holds its
  own region only; a scoped node restarts from its slice; the full node serving a subscriber stops
  and the subscriber moves; a replica moves from scoped n6 to a new scoped n7; the scoped group
  splits and both sides serve; a request to a scoped node for a keyspace outside its slice gets
  a redirect to a full node and a client with peers is served; the children merge back; the
  keyspace is backed up while a scoped replica is down; that node restarts after the metadata log
  moved hundreds of entries past a 32-entry ring and catches up through a snapshot; the backup is
  restored onto scoped voters and serves; a scoped node drains onto a fresh scoped spare; and
  `cluster health` reports `catalog_scope ok` for every slice.
- Scale, `scripts/meta-scoped-scale.sh`, debug build, one host (NVMe), groups of three on disjoint
  scoped triples, one keyspace each. The metadata group stayed three voters, no learners:

  | Scoped nodes | Groups | Full catalog keys | Rows per slice | Slice / catalog | Streams per full node | Join (s) | Failover (s) |
  | --- | --- | --- | --- | --- | --- | --- | --- |
  | 24 | 8 | 135 | 22 | 16.3 % | 7 / 11 / 6 | 10 | < 1 |
  | 48 | 16 | 247 | 22 | 8.9 % | 15 / 16 / 17 | 18 | 1 |
  | 201 | 67 | 961 | 22 | 2.3 % | 78 / 59 / 64 | 81 | 2 |

  A slice's size does not move with the cluster (R2/R3). The full node serving the most
  subscribers was stopped and exactly its subscribers moved (78 at 201 nodes), every slice
  current again within about two seconds (R4); snapshots peaked at the limiter's 4 at once.
  `cluster health` took 424 ms over all 204 nodes and 164 ms with `--full-only` (R7).

  Against the baseline (`KV9_SCALE_MODE=full`: the same nodes as full learners), the metadata
  leader's sends (`meta_raft_sent_msgs`):

  | Nodes beyond the voters | Mode | Messages while creating the groups | Messages / s, steady |
  | --- | --- | --- | --- |
  | 24 | full learners | 2 860 | 537 |
  | 24 | scoped | 216 | 41 |
  | 48 | full learners | 11 683 | 1 030 |
  | 48 | scoped | 452 | 41 |
  | 201 | scoped | 2 290 (67 groups) | 40 |

  With full learners the leader's load grows with every node; with scoped nodes it stays at the
  three-voter level (M4).
- The Lean model `proofs/lean/catalog-scope` with this implementation's sources pinned in its
  contract.

**Snapshot limiter and health.** `KV9_SCOPE_SNAPSHOT_PARALLEL` (default 4) bounds the scope
snapshots one full node computes at once (status `catalog_scope_snapshots_peak`,
`catalog_scope_snapshots_waited`). `cluster health` has a `catalog_scope` verdict: `fail` for a
stale slice (with its error), `warn` for a slice more than 1024 entries behind the leader's
commit; scoped nodes are excluded from the `apply_lag` verdict (their metadata driver never runs).

**Observation at scale (R7).** `kv9-admin` fans out with at most `KV9_ADMIN_FANOUT` (32) calls in
flight. A full node publishes `catalog_scope_subscribers=node:index:ms,…`, and `cluster health
--full-only` asks only the full nodes and judges every scoped slice from those tables (a node no
full node streams to, or one silent for 10 s, is `fail`).

**Not yet.** Nothing from §7/§8 remains open; scopes are per region (S1).

