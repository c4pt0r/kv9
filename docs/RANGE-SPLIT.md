# Range split, re-entered

Design packet, 2026-10-29. Parent [#9](https://github.com/c4pt0r/kv9/issues/9), D04
[#25](https://github.com/c4pt0r/kv9/issues/25). Fourth of the four increments agreed after
[REPLICA-ADDITION](REPLICA-ADDITION.md) §7. The split prototype of 2026-09 lives on
`experimental/range-split` (twelve packets, ~6k lines, retired raft tags 11/12/13 and TASKS kind
107); this packet re-enters it against the tree as it is now — committed plans, committed members,
the plan driver, the admin tool — in the same committed-authority-first order the prototype proved:
**intent → seal → population → publication**, each its own increment with its own proof and e2e.

> **Status: split and merge implemented** (2026-10-29): the committed split intent with its
> children (§2), the seal (§3), the verified population with its evidence rows (§4), the
> one-transaction publication (§5) and the merge, the same pipeline backwards (§6), driven by
> `runtime/range_split.rs` and `runtime/range_merge.rs`; `scripts/range-split-intent-e2e.sh` and
> `scripts/range-split-population-e2e.sh` (split → published → merged back → published) pass and
> run in CI. `kv9-admin group split <task> --at <key>` and `group merge <low> <high>` are the two
> verbs; the rest is the cluster's. §9 records what the implementation settled; §10 what follows.

## What master kept, and what it did not

Kept (and relied on below): the committed range directory with its partition rule (a keyspace's
UNSEALED bindings cover it exactly once; sealed bindings are routing history,
`ranges::read_in`), the key-aware raw directory (`RawDirectory::get_for`), the one-way seal through
the group's own log (`Command::DataRange` CAS, `DataRange::may_follow`: version+1, sealed), the
apply-time fence that checks every key against the group's descriptor, and `RegionManager::
retire_locally`. Not kept: the intent row (kind 107, refused by store history), the populate and
TTL-split commands (tags 11/12/13, never reused), the seal/populate/publish RPCs and their
drivers, and the auto-split trigger.

Changed since: a group's members are `committed_members` (moves happen), not the creation's
replicas; replica movement is a committed plan the cluster drives; `kv9-admin` is the operator
surface; the engine is the versioned index under the WAL; there are no transactions, only the
atomic range batch (a batch that crosses a range is refused whole).

## 1. The shape, unchanged

A split of bound range `P = [s, e)` at key `k` produces children `L = [s, k)` and `H = [k, e)` on
**the same stores as `P`** — a split moves no replica; movement is REPLICA-ADDITION's job and can
run before or after. The parent is sealed (one-way, through its own log), both children are
populated from the sealed parent's state, and one catalog transaction publishes the new partition:
the parent's binding sealed (version+1), two child bindings inserted. Readers see the old shape or
the new, never a partial one (the partition rule refuses partial shapes at readback).

## 2. Part 1 — the intent (implemented)

`TASKS` kind **113**, `KV9SPL02`:

```
root 32 | operation 16 | task 8 | parent_bind_task 8 | parent_region 8 | child_low 8 | child_high 8 | key_len 2 | split_key ≤1024
```

- One transaction (`plan_split`): the parent's current binding (unsealed), the split key strictly
  inside it, and **the two children created and activated right there** — `plan_empty_group` on the
  parent's committed member nodes with operations derived from the split operation
  (`sha16(op ++ "/low")`, `sha16(op ++ "/high")`), then `plan_activation` on both. The prototype
  made the operator create the children first; nothing was gained by the extra verbs.
- Cross-validation shared by planning and readback: the parent binding row is exact; the key is
  inside; both children are committed, activated, bound to no keyspace, with replicas equal to the
  parent's committed members; neither child is the parent. A sealed parent is legal only as this
  intent's own published shape (part 4), with the children bound to exactly `[s,k)` and `[k,e)`.
- One live intent per parent region; an identical resubmission confirms; divergence refuses.
- Public RPC `SplitRange`, `kv9-admin group split <creation-task> --at <key> [--at-hex <hex>]
  [--operation|--label]`, `ListDataGroups` rows carry `splits` with a catalog-only stage
  (`intended` now; `sealed`, `populated`, `published` as the parts land).
- The parent keeps serving unchanged; the children run as empty, unbound groups (exactly what
  `group create` makes) and refuse every public key, because no route names them.

## 3. Part 2 — the seal

The parent group's leader, seeing the committed intent and both children **serving** (`Ready`
with a driver on every member), proposes `Command::DataRange { expected: current, next: sealed
(version+1) }` through the parent's log (the CAS that exists). From that index on the parent
refuses writes with the typed sealed error; reads keep working. The seal position `(term, index)`
is evidence the children's population needs: the parent's state at the seal is the state to copy.
A committed row is not needed for the seal itself (it is in the parent's log and descriptor); the
child population reports it.

## 4. Part 3 — population

Each child's leader runs on a store that also holds a **parent replica** (same members). Once that
local parent replica has applied the seal, the child leader copies the parent's half — a local
snapshot read of `[s,k)` or `[k,e)` at or after the seal — into the child through the child's own
log as ordinary `Write` batches, idempotently from a durable cursor (the last key proposed, kept in
the child's engine under a reserved key), until the half is exhausted. Then it computes the digest
of its state and the digest of the parent half, and commits a **population evidence** row (kind
114: operation, child region, seal position, half digest) through the metadata leader (internal
RPC, like install evidence) only when the two digests agree. Prototype lesson kept: population
writes must not be refusable by the child's own fence, so the child carries its half descriptor
from creation (installed by the intent's activation path), while routing — the catalog — still
names only the parent. No new raft command: the prototype's tag 12 is not needed.

## 5. Part 4 — publication

The metadata leader publishes when both children's evidence rows name the same seal position:
one catalog transaction with the parent's binding successor (sealed, version+1, kind 102) and two
child bindings `[s,k)` and `[k,e)` (kind 102, conf_ver 1, version 1) — the partition rule accepts
exactly this shape. `RawDirectory` refreshes, public routes flip, the sealed parent keeps refusing
with the typed error until it is retired (removal decisions for each member through the existing
REPLICA-REMOVAL rows: a later increment, as the prototype left it too).

## 6. Merge (D05), the same pipeline backwards

Two adjacent unsealed siblings on the same members → one new child `[s_L, e_H)`; seal both,
populate the child from both halves, publish the successor shape. **As built:** kind 115
`KV9MRG01` (`plan_merge`: both bindings unsealed, `low.end == high.start` in one keyspace, same
committed members, no live split or merge on either parent; the child created and activated in
the same transaction), kind 116 `KV9MRE01` evidence (the split evidence's layout, carried over the
same internal RPC and told apart by its magic), `plan_merge_publication` (both parent bindings
sealed in place, the child bound to `[s_L, e_H)` with its region row, both parent region rows
removed). The driver (`runtime/range_merge.rs`) copies the low parent to its end, then the high
one, from the local sealed replicas, and verifies both chained through one digest against the
child. `kv9-admin group merge <low-task> <high-task>`; `ListDataGroups.merges` on both parents.

## 7. Invariants

- **S1 — one intent per parent, permanent.** An intent row is immutable and never cancelled.
- **S2 — the children are the parent's stores.** Child replicas equal the parent's committed
  members at intent time.
- **S3 — the parent serves until published.** Part 1 changes nothing about routing or serving.
- **S4 — no partial shape.** Publication is one transaction; readback refuses partial partitions.
- **S5 — population is exact.** A child is published only with evidence that its state equals the
  sealed parent half at the seal position.

## 8. Evidence for part 1

- Lean model `proofs/lean/range-split-intent`: bound parent, ready children, the intent step, no
  seal/publish/reroute step; S1–S3 as theorems; controls (intent on an unbound parent, on a sealed
  parent, with a key outside, with children on other stores, a second intent for the parent).
- `scripts/range-split-intent-e2e.sh`: three stores, a group with a bound keyspace under writes;
  `group split --at` commits the intent and two activated unbound children on the same members;
  the refusal matrix (sealed/unbound parent, second intent, divergent key under the same operation,
  a child as parent); exact idempotent confirmation; the parent still serving the same data and
  accepting new writes afterwards; the children visible in `group list` with no keyspace.

## 9. As built (parts 2–3)

- **One driver, three roles, one step in flight** (`runtime/range_split.rs`, after the drain pass).
  A child's leader proposes the child's half descriptor (initial `DataRange` publication through
  the child's log, once per term); the parent's leader seals (`DataRange` CAS from the current
  descriptor's digest to version+1 sealed, through `propose_and_wait`) once both children have a
  leader and an applied descriptor; a child's leader copies its half from the LOCAL parent replica
  once that replica shows the sealed descriptor — `RawExecutor::scan` pages of 512 rows from a
  parent snapshot, written through the child's own `raw_batch_put` (the child's fence admits
  exactly its half) — then digests parent half and child row for row and commits the kind-114
  evidence (`KV9SPE01`: operation, intent, child, digest, rows) locally or over the internal
  `RecordSplitEvidence`. `ListDataGroups.splits[].stage` is `intended` → `populating` →
  `populated`, with the evidence rows attached.
- **No new raft command.** Population is ordinary raw writes under the child's descriptor; the
  catalog routes nothing to a child until part 4, so no client can reach one.
- **The seal window.** `RawGroup::authorize` refuses reads and writes alike on a sealed range
  (typed `stale region epoch`), so between the seal and the publication the parent's keyspace is
  unavailable: in the e2e that is the population of a few rows plus one catalog commit, in
  production it is proportional to the parent's size. Clients with auto-refresh re-resolve the
  route on the typed error and continue on the children once published. An automatic trigger
  must weigh that window; `group split` stays an operator's deliberate act for now.
- **Publication, as built** (`plan_split_publication`, the metadata leader's `publish` step once
  both evidence rows exist). The parent's kind-102 binding row is sealed **in place** (payload →
  version+1, sealed; the row's task is the intent's `parent_bind_task` and stays valid), the two
  child bindings are inserted with `[s,k)` and `[k,e)` at epoch 1/1 together with their routable
  region rows, and the parent's region row is removed so the catalog's key→region lookup names a
  child. `ranges::read_in` validates the staged shape inside the transaction before it commits.
  On every node the binding reconcile then finds each child's applied descriptor equal to its
  new binding and inserts it into the raw directory; the parent re-enters it sealed and
  `get_for` skips it. Nothing new is proposed into any group at publication.
- **TTL-activated ranges cannot be sealed** (the state machine refuses the seal as a legacy command
  while TTL is enabled): the seal step fails loudly in the status line and the split waits; the
  TTL-preserving split is future work, as the prototype left it.
- **Restart.** The cursor is in memory; a child leader that restarts (or changes) copies again from
  the first key — idempotent puts of an immutable source. Evidence already committed ends a child's
  population for good; a differing digest for the same child is refused permanently.
- **Retirement of the sealed parent (and merge parents).** A sealed binding is written only by a
  publication, together with the successors that cover its keys, so it is the committed authority
  that the group is history — reclamation already read it that way. Every replica now retires
  itself on it (`reconcile_retirement` → `retire_locally`: the driver stops, the group leaves the
  raw directory, the durable `Retired` record fences any restart), and with `KV9_RECLAIM_RETIRED=1`
  the payload is deleted. No new row, no new verb; the e2e watches all three replicas reach
  `retired` then `reclaimed` after the split and after the merge.

## 10. What follows

Load- or time-based triggers beyond the footprint, and an automatic merge of small siblings.

## 12. Aborts, and the guards that keep intents honest (implemented)

The review of 2026-10-29 found two defects in the increments above and this section is their
fix.

**Membership and range operations exclude each other while live.** A split or merge intent
compares its children's replicas with the parent's *current* committed members on every
readback, so a move on the parent after the intent would have broken that readback forever —
the prototype's "one row froze the subsystem", re-created. Now `plan_migration` (every
`add-replica`, `move-replica`, repair decision and drain plan) refuses while an unaborted,
unpublished split or merge names the group (`live_range_operation`), and `plan_split` /
`plan_merge` refuse while the parent has an unsettled migration (`live_plan_on_region`). The
listing no longer hides a readback failure: `ListDataGroups` carries `readback_errors`,
`kv9-admin group list` reports them as partial (exit 4), and the drivers' status lines say
"intents do not read back" instead of going quiet.

**Every long-running intent can be aborted: `kv9-admin task abort <task>`.**

- A **split or merge intent** is abortable until its first seal. The parent's leader commits
  **seal evidence** (kind 118, `KV9SLE01`) right after sealing, over the shared evidence RPC, and
  the abort planner refuses once any exists. The one race left — an abort committed between the
  seal and its evidence — is decided by the catalog alone: an abort row (kind 117, `KV9ROA01`)
  is **effective iff no seal evidence names its operation**. Every reader applies that rule
  (planners, drivers, listings); a late abort is visibly ignored (`stage` stays on its way to
  `published`) and the split completes. An effective abort makes the intent history: its
  children retire on every node and are reclaimed with `KV9_RECLAIM_RETIRED=1`, the parent is
  free for a new split, a merge or a move, evidence and publication for it refuse, and readback
  decodes it without re-validating it (its children's membership may legitimately diverge).
- A **replica plan** aborts through the migration abort (kind 108) it already had: once an image
  is planned, the published pin supplies the subject and the retention ledger settles it;
  before that, a derived no-image subject that no pin can ever match. The driver's last step for
  an aborted plan is `detach` (the group leader detaches the learner it attached); the plan's
  stage reads `aborted`.
- A **drain** aborts any time; placement stops excluding the store, the drained node stops
  handing leadership away, and the same store can be drained again: the drain row (`KV9DRN02`)
  now carries a sequence, so an aborted drain leaves the next identity free. Move plans the
  drain already committed complete on their own.

**A routing gap found by the same review's regression run.** A request for a data keyspace that
landed on a node hosting none of its range (typically the metadata leader after a move or drain
took its replica away) was refused with "not available through the metadata group". It now
answers the typed `NotLeader` redirect with a committed member of the covering range as the
hint, and `get_region` carries the same hint when the range is not local; the SDKs already
follow both to the group's actual leader. The REPL's `delrange`, like its `scan`, follows
`resume_from` across range boundaries.

`KV9_RANGE_DRIVER=0` pauses the split, merge and auto-split drivers on a node without losing
anything: committed intents wait. It is the operator's brake and what the abort e2e uses to
abort deterministically before a seal.

Evidence: `proofs/lean/operation-abort` (ten theorems: one abort per operation, a published
operation's abort is ineffective, a move needs the abort, children retire only on abort, an
abort is refused after a seal, a late seal makes the abort ineffective; five controls) and
`scripts/operation-abort-e2e.sh` (CI): a split intent aborted with the drivers paused — children
reclaimed, parent serving and free for a new split; a plan aborted after its learner was
attached — learner detached; a drain aborted and the store drained again; a split refused while
a plan is live; an unknown task a typed refusal; and no catalog family failing to read back.
Cross-range **scans and delete-range** already work over the partition (the server walks the
unsealed ranges in key order, clamps each chunk at its range end and serves it under that group's
own authorization; when the next chunk's leader is another node the page carries `resume_from` and
the SDKs continue from it) — the split/merge e2e proves it across the published boundary. An
atomic **batch** that crosses a boundary stays refused whole, by design: atomicity is per range.

## 11. The automatic split trigger (implemented)

`KV9_AUTO_SPLIT_BYTES` (0 = off, the default) on every node arms it. Every five seconds each data
group's **leader** sizes its own replica — `data.wal` plus `data.segments/`, the same write-volume
footprint the prototype used, honest about overwrites — for every bound, unsealed range it leads
that has no live split, merge or unsettled replica plan. At or above the threshold it samples the
median user key from one snapshot (strictly inside the range; fewer than two rows never split) and
commits the intent: `SplitRange` locally when it leads the metadata group, otherwise the
node-authenticated `PlanSplit` at a metadata candidate. The operation is
`sha16("kv9-auto-split/v1" ++ root ++ region ++ bind task)`: a binding splits once and any repeated
decision confirms. `KV9_AUTO_SPLIT_COOLDOWN_SECS` (600) spaces one node's decisions; the children
are new regions and split again only when they reach the threshold themselves. The status line
`auto_split=` shows every led group's footprint and eligibility and the decisions taken.

**Its cost is §9's seal window.** Between the seal and the publication the parent refuses reads
and writes with the typed stale-epoch error, for the time it takes to copy the half; clients with
auto-refresh continue on the children once published. That is why the trigger is off by default
and why the threshold should be sized to what an application tolerates. The e2e writes through a
trigger that fires mid-fill: the batches that land in the window are refused and succeed on the
children; every key is served afterwards.

Evidence: `proofs/lean/auto-split` (nine theorems: one intent per binding, a decision needs every
guard, a decision starts the cooldown, the key is inside; seven controls), `scripts/auto-split-e2e.sh`
(zero verbs after the writes: the intent at a sampled median, published, children serving, the
parent retired) in the CI e2e matrix.
