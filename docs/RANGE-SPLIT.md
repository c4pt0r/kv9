# Range split, re-entered

Design packet, 2026-10-29. Parent [#9](https://github.com/c4pt0r/kv9/issues/9), D04
[#25](https://github.com/c4pt0r/kv9/issues/25). Fourth of the four increments agreed after
[REPLICA-ADDITION](REPLICA-ADDITION.md) §7. The split prototype of 2026-09 lives on
`experimental/range-split` (twelve packets, ~6k lines, retired raft tags 11/12/13 and TASKS kind
107); this packet re-enters it against the tree as it is now — committed plans, committed members,
the plan driver, the admin tool — in the same committed-authority-first order the prototype proved:
**intent → seal → population → publication**, each its own increment with its own proof and e2e.

> **Status: parts 1–3 implemented** (2026-10-29): the committed split intent with its children
> (§2), the seal (§3) and the verified population with its evidence rows (§4), driven by
> `runtime/range_split.rs`; `scripts/range-split-intent-e2e.sh` and
> `scripts/range-split-population-e2e.sh` pass and run in CI. Part 4 (publication) is designed
> here and open; until it lands a split stops at `populated` with the parent sealed (§10).

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
populate the child from both halves, publish the successor shape. It is parts 2–4 with two parents;
it is specified here so part 2–4's rows carry a parent *set*, not a single parent.

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
  unavailable. Part 4 closes the window; until then `group split` is an operator's deliberate,
  observable act, never automatic.
- **TTL-activated ranges cannot be sealed** (the state machine refuses the seal as a legacy command
  while TTL is enabled): the seal step fails loudly in the status line and the split waits; the
  TTL-preserving split is future work, as the prototype left it.
- **Restart.** The cursor is in memory; a child leader that restarts (or changes) copies again from
  the first key — idempotent puts of an immutable source. Evidence already committed ends a child's
  population for good; a differing digest for the same child is refused permanently.

## 10. What follows

Part 4 (publication) is the next increment: one catalog transaction with the parent binding's
sealed successor and the two child bindings, the raw directory refresh, and the close of the seal
window. Parent retirement, the auto-split trigger (prototype: `KV9_AUTO_SPLIT_BYTES`, median-key
sampling, derived operations) and merges follow in that order.
