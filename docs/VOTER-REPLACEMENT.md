# Automatic voter replacement while quorum holds

Design packet, 2026-10-28. Parent [#9](https://github.com/c4pt0r/kv9/issues/9); belongs to the
placement stage (D06, [#27](https://github.com/c4pt0r/kv9/issues/27)) and builds on the membership
verbs of [voter promotion](VOTER-PROMOTION.md) and [replica removal](REPLICA-REMOVAL.md).

> **Status: phase 1 implemented for the metadata group (2026-10-28).**
> `crates/server/src/runtime/voter_replacement.rs` holds the verdict and the promote-then-remove
> driver; `RemoveNode` (RPC + `kv9 client remove-node`) is the new verb; `kv9_raft` exposes per-peer
> `PeerProgress`. `scripts/voter-replacement-e2e.sh` (in the CI e2e matrix) kills two of three original
> voters one after another with two credentialed learners present and checks that each loss is
> repaired in the right order with service never lost. Deviations from the design below, deliberately:
> the decision is **not yet a committed catalog row** (kind 109) — the Raft configuration is the
> committed record and the leader re-derives the step from it after a crash, decisions and verdicts are
> visible in the status line `voter_replacement=`, and the promotion cooldown is per process. Data
> groups (§6) and the committed decision row are the next increments.

## The problem

A group with three voters and any number of learners survives one voter loss and no more. Learners
replicate but do not vote, so the second loss leaves one voter out of three: no election can succeed,
`check_quorum` deposes whoever still believes it leads, writes fail, reads answer `read_unconfirmed` /
`not_leader`, and nothing in the system can act — promoting a learner is itself a committed
configuration change, which needs the quorum that is gone. Today kv9 does nothing between the first
and the second loss either: a dead voter stays a voter until an operator runs the membership verbs
by hand, and for the metadata group there is not even a verb that removes a voter.

The repair has to happen **while the quorum still exists**, in the window after the first loss. That
is what this packet adds. Recovery after quorum is lost is a different, operator-driven tool with
different guarantees; it is named in §8 and deliberately not designed here.

## 1. What exists already

| Need | Where it is today |
| --- | --- |
| Single-step configuration changes through the group's own log | `NodeDriver::{add_learner, promote_voter, remove_voter}` (`crates/raft/src/driver.rs`), applied and receipted (`wait_conf_applied`). |
| Metadata-group membership verbs | `AdmitNode` → `kv9 join` (learner) → `PromoteNode`; `RevokeAdmission` before promotion. No `RemoveNode`. |
| Data-group membership verbs | The migration protocol: committed intent (task 103) → `AttachMigrationLearner` → offline image install → committed install evidence (104) → `PromoteMigrationVoter` → committed removal decision (106) → `RemoveSourceReplica` (keeps ≥ 3 voters, never the leader itself). |
| Per-peer progress at the leader | raft-rs `Progress { matched, recent_active, .. }` via `prs()` (`rawnode.rs`, used by the compaction floor's all-matched check). |
| Catalog facts | `nodes(id, addr, state, last_heartbeat, …)`, `node_admissions`, committed creation/activation/binding rows with exact `(node, store_incarnation)` replicas. |
| A leader-only maintenance loop | `NodeRuntime`'s 100 ms reconcile pass (`reconcile_retirement`, `reconcile_adoption`, compaction, …), gated by `maintenance_allowed`. |
| Committed-authority pattern | One immutable TASKS row per operation, idempotent confirmation, divergence refuses, execution by the responsible leader from *local applied* metadata; proofs pin the planner and executor. |

What is missing is exactly three things: a **verdict** that a voter is lost, a **committed decision**
that names its replacement, and a **driver** that carries the decision through the existing verbs in
the one order that never risks the quorum.

## 2. Invariants

- **I1 — promote before remove.** The voter set goes `{A, B, X} → {A, B, X, R} → {A, B, R}`, never
  through a two-voter state. With X dead, the four-voter configuration still has three live voters
  (quorum 3); the three-voter result has three. Removing X first would leave `{A, B}` with quorum 2 —
  one more failure from unavailability during the change itself.
- **I2 — the replacement is caught up before it counts.** R is promoted only when its `matched`
  index is at least the leader's commit index at decision time, and the `AddNode` entry itself must
  replicate to R before it applies. A learner that cannot catch up within the window is not
  promoted; the decision is abandoned, not forced.
- **I3 — decisions are committed, exact and idempotent.** A decision names the group, the lost
  replica as `(node, store_incarnation)`, the replacement the same way, and the leader's observation
  that justified it. Resubmission with identical content confirms; any divergence refuses; one live
  decision per group.
- **I4 — only a leader decides, only for a group it leads.** A leader exists only while a quorum
  exists, so a decision is never taken for a group that already lost quorum; the executor re-checks
  leadership and the committed decision from local applied state before every step.
- **I5 — the replication factor is preserved and bounded.** The outcome has exactly as many voters
  as before; no step ever leaves fewer than three voters committed; the leader never removes
  itself.
- **I6 — a returning voter is not a special case.** If X comes back after the decision committed, X
  is still removed (the decision is deterministic) and rejoins through the ordinary learner path;
  flapping is bounded by a per-node cooldown, not by guessing.

## 3. The verdict: when is a voter "lost"?

The leader is the only node whose view of a voter is both local and authoritative, so the verdict
comes from Raft progress, not from the catalog heartbeat column:

- `Progress.recent_active` is cleared by `check_quorum` every election timeout and set by any
  response. The leader keeps, per voter, the last tick at which it was active and the last `matched`
  advance.
- **Suspected** after `KV9_VOTER_SUSPECT_TICKS` (default: 20 election timeouts) without activity;
  exported in the status file as `voter_health=[{node, state, last_active_ms, matched}]`.
- **Lost** after `KV9_VOTER_LOST_SECS` (default 300 s) continuously suspected *and* the catalog has
  no fresher registration/endpoint change from that node than the leader's last contact. Both
  thresholds are configuration; `KV9_AUTO_REPLACE_VOTER=1` enables decisions at all (default off,
  as `KV9_AUTO_SPLIT_BYTES` was for split): the verdict and the status line ship first, the
  automatic decision second.
- A partitioned-but-alive voter will look lost. That is accepted and bounded by I6: the cost of a
  false verdict is one extra membership round trip and a re-admission, never data loss, because the
  old voter's log is never truncated on the strength of the verdict.

## 4. The decision row

TASKS kind **109**, magic `KV9RPL01`, module `crates/meta/src/data_groups/replacement.rs`
(107 is retired with split; 109 is free):

```
operation          16 bytes, unique retry identity (sha16("kv9-voter-replacement-v1" ++ root ++ group ++ lost incarnation))
group              region id (META_REGION_0 or a data group's region)
lost               (node, store_incarnation)   — must be a committed voter of the group
replacement        (node, store_incarnation)   — must be a committed learner of the group (see §6 for data groups)
observed           (leader_term, leader_commit_index, replacement_matched, lost_last_active_at)
state              Decided → Promoted → Removed   (each transition is a confirmation of the Raft conf state, not a wish)
```

The planner runs under the catalog transaction lock with the same cross-validation shape as 103/106:
a current committed creation and binding for the group, `lost` ∈ voters, `replacement` ∈ learners,
no other live decision for the group, cooldown satisfied for both nodes. The operation id is derived,
so any coordinator at any time re-derives the same row and confirms idempotently.

## 5. The driver

One reconcile step on the leader of the affected group, in the existing maintenance pass:

1. **Decide.** If auto-replacement is enabled, a voter is `Lost`, and a qualifying learner exists
   (highest `matched`; ties broken toward the node hosting the fewest voters of other groups), commit
   the decision through the metadata group. For a data group the metadata leader commits it; the
   data group's leader executes it, like every other committed task.
2. **Promote.** `promote_voter(replacement)`, await the conf apply, verify `replacement ∈ voters`,
   record `Promoted` (a confirmation row update, idempotent). I2 is checked immediately before the
   proposal; if it no longer holds the step waits, and the decision expires after
   `KV9_VOTER_REPLACE_DEADLINE_SECS` (default 900 s) without ever having touched the voter set.
3. **Remove.** `remove_voter(lost)`, await, verify `lost ∉ voters` and `|voters| ≥ 3`, record
   `Removed`. For data groups the removed replica is then retired through the existing committed
   removal machinery (kind 106 semantics: isolated by membership, storage reclaimed only under
   `KV9_RECLAIM_RETIRED=1`). For the metadata group the removed node's row moves to a
   `demoted` state and its store stays fenced as a non-member until it re-registers.
4. **Replenish.** With one learner fewer, the group may be below its standby target; that is the
   placement scheduler's job (D06) and out of this packet, except that the status file now reports
   `standby_learners=` per group so the gap is visible.

Every step reads the decision and the current configuration from local applied state and is a
confirmation when its effect already holds, so a coordinator that crashes between steps resumes
from the committed facts: replacement already a voter → remove; lost already gone → done.

New verbs this needs: `RemoveNode` for the metadata group (leader-only, `≥ 3` voters remain, never
self, identical guards to `RemoveSourceReplica`), and `ReplaceVoter` to commit a decision by hand
(`kv9 client replace-voter --group <id> --lost <node> --with <node>`), which is also how the e2e
drives the protocol before the automatic verdict is trusted.

## 6. Where the replacement comes from

- **Metadata group.** Every kv9 process that joined the cluster is a learner of `META_REGION_0`
  already (that is what `AdmitNode` + `join` produce), so candidates exist whenever the cluster has
  more processes than voters. Nothing new is started.
- **Data groups, phase 1.** Candidates are learners the group already has: migration destinations
  that were attached and evidenced but not yet promoted, and — once the scheduler keeps them —
  standby learners created on purpose. Phase 1 only promotes an existing, caught-up learner.
- **Data groups, phase 2.** With no learner at hand, replacement is a full migration: pick a
  destination node, run intent → capture → install → attach → evidence → promote → removal. All
  of that exists as verbs; the automatic trigger and the destination choice are placement work
  (D06) and are sequenced after phase 1.

## 7. Evidence this packet must produce

- **Planner tests**: exact-identity refusal (wrong incarnation, learner not caught up, two live
  decisions, cooldown), idempotent confirmation, derived operation id.
- **Lean model** of the decision state machine against single-step configuration changes: with at
  most one further failure during a decision, live voters never drop below a majority (I1, I5), a
  removal never precedes its promotion, and a decision never acts on a group it does not lead (I4).
  Pinned to `replacement.rs`, the driver, and the e2e below.
- **Real-process e2e** (`scripts/voter-replacement-e2e.py`): three voters and two learners of the
  metadata group; kill voter 2; within the deadline the status shows `Promoted`/`Removed`, writes
  and reads continue, `meta_voters` has three members; then kill another *original* voter and the
  group is still serving — the headline property. The same shape for one data group.
- **Chaos**: partition (not kill) a voter so the verdict is false; the voter returns after removal,
  re-registers, no committed entry is lost (compare the full key set and applied positions);
  flapping stays within the cooldown. A second run kills the coordinator between Promote and
  Remove and checks the resume path.
- **Status and observability**: `voter_health`, `replacement=` per group, metrics for decision
  latency; all the existing controls checkers extend with the new verbs.

## 8. Out of scope, on purpose

- **Quorum-loss recovery.** Rewriting a group's configuration from one surviving voter plus
  learners is a different tool: offline, explicit operator token, it must pick the longest log
  among survivors, bump the cluster fingerprint so the old voters are refused on return, and print
  the `(term, index)` after which entries may be lost. That is a separate packet; nothing in this
  one should make it look automatic.
- **Joint consensus.** Single-step changes in the I1 order are sufficient here; two-step joint
  configuration is not needed for a one-node replacement.
- **Automatic admission of new processes** into the metadata group. Replacement promotes existing
  members; adding capacity remains an operator action.
- **Placement quality** (which node should host the standby learner, how many standbys a group
  keeps). D06.
