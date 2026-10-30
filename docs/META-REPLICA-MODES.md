# Metadata replica modes: full copy or scoped catalog

Design packet, 2026-10-29. Every node stays a peer (same binary, same verbs, same data-group
role); what differs is a per-node choice of how much of the metadata it keeps. Today every node
is either a metadata voter or a metadata **learner** that replicates the whole catalog and the
whole TASKS history through the metadata Raft group. This packet adds a second mode in which a
node keeps only the **scoped** of the catalog its own replicas need, and is not a member of the
metadata Raft group at all.

> **Status: design, not implemented.** §8 lists the increments; §9 the decisions for the owner.

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

`KV9_META_REPLICA=full|scoped` on `kv9 start` (default `full`), recorded at admission:

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

> **S1 — closure.** For every region R the node hosts, is attached to, or is named as a
> destination, target or child of, the scope contains **every** committed row that references R,
> every row those rows reference (creation, migration, capture, owner, keyspace, binding), and
> every abort, removal, evidence or seal row of the same operations. Plus the root, the node's
> own directory row and drain rows, and the directory rows of the stores that are members of its
> regions. Nothing global beyond the root: no row is in every scope.

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
2. If the subscriber is behind the serving node's retained history, the serving node sends a
   **scope snapshot** at its current position (the closure for every region in the subscription),
   then continues with changes.
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

- Lean model: catalog positions, scopes, region membership changes; M1 (closure preserved under
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
