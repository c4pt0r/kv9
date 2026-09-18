# Automatic compaction-floor selection (bounded log growth, self-driving)

Updated 2026-09-18. Parent [#9](https://github.com/c4pt0r/kv9/issues/9),
P1 storage [#20](https://github.com/c4pt0r/kv9/issues/20). The
group-compaction increment shipped committed floors executed under an
all-matched gate, but a floor had to be recorded by an operator. This
makes it self-driving.

## What this increment adds

With `KV9_AUTO_COMPACT_ENTRIES > 0` (default 0 = off), the metadata
leader observes each bound group's LOCAL replica retained raft-log
length (`raft_committed - log_first_index`). When it exceeds the
threshold, it proposes a compaction floor and commits a kind-110 row —
exactly the manual `record-group-compaction`, no new authority — which
the existing gated `reconcile_compaction` then executes.

The proposed floor is the local replica's APPLIED position
(`applied_term`, `applied_index`): any replica's applied position is a
durable, committed point, so the floor is always committed-backed and
never a future or unbacked index. A floor is proposed only when applied
has advanced a full threshold past the last committed floor, so floors
strictly increase without churn. Correctness is inherited entirely from
group-compaction: the floor is still all-matched-gated at execution, so
a too-aggressive automatic floor simply fails the gate and retries — it
never strands a voter. One automatic floor per group per turn.

## Known limits, deliberately out of scope

Still leader-only execution (follower-side log bounding remains the
group-compaction open edge). The trigger is a fixed entry-count
threshold — no age or byte policy, no bounded absolute log size, no
physical reclamation of the append-only log file. No chaos campaign, no
performance claims.

## Evidence

- Serial qualifying workspace run plus strict Clippy/formatting.
- Accepted five-process e2e (`scripts/auto-compaction-e2e.py`): 800
  sustained writes with ZERO compaction verbs — the metadata leader
  auto-proposes a committed floor, the gated reconcile advances the
  leader's `log_first_index` past 1, a second burst auto-advances the
  floor further (strictly increasing), and a full-cluster restart
  recovers on the auto-compacted logs with every sampled key serving.
- Twelve-theorem Lean model
  ([proofs/lean/auto-compaction](../proofs/lean/auto-compaction/README.md))
  constraining only the trigger (execution safety is the
  group-compaction model's), with five semantic mutation controls and
  two proof-policy controls; thirty-two sibling Lean models and the
  retention TLA/TLAPS model re-accepted.

Validation packet: [docs/auto-compaction-v1](auto-compaction-v1/README.md).
