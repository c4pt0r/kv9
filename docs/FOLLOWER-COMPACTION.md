# Follower-side raft-log compaction (bounded log on every voter)

Updated 2026-09-18. Parent [#9](https://github.com/c4pt0r/kv9/issues/9),
P1 storage [#20](https://github.com/c4pt0r/kv9/issues/20). The
[group-compaction](GROUP-COMPACTION.md) increment bounded only the group
LEADER's raft log; a lagging follower kept its full log until it led. This
closes that documented open edge: every voter bounds its own log.

## What this increment adds

1. **A group-replicated confirmed floor.** After the group leader's
   all-matched truncation at floor `F` (leader-only, every voter matched
   at or beyond `F`, the durable REC_COMPACTION record — the group-compaction
   contract), it publishes `F` into the group's OWN raft log through an
   ordered `Command::CompactionConfirmed { region, floor }` (tag 9,
   `crates/raft/src/command.rs`). Every voter applies it with an identity +
   monotonic CAS (`apply_compaction_confirmed`,
   `crates/raft/src/state_machine/data_range.rs`) to the reserved
   `kv9_common::data_range::COMPACTION_CONFIRMED_KEY` — the `DataRange`
   precedent. It is NOT a fenced write: the data group's `RangeFence`
   admits only Raw user keys in range, and the floor authorizes prefix
   discard, so it must not be client-forgeable.
2. **Follower-side compaction.** Each replica reads that replicated floor
   from its own applied state and, once its applied position has reached it,
   compacts its own prefix through `RaftPeer::compact_confirmed_prefix`
   (no leadership or peer-progress gate — the confirmation already proves
   every voter matched the floor, so a committed entry at or below it is
   held by all). `reconcile_compaction` (`crates/server/src/runtime.rs`)
   drives both stages on every serving node.
3. **The recovery gate (a latent group-compaction bug this closed).** On
   restart, `region_manager` adopts a durable compacted base into a live
   peer ONLY under committed authority whose floor equals the base. That
   check previously accepted a kind-105 migration truncation decision but
   NOT a kind-110 compaction floor. group-compaction's own e2e never caught
   it because only the leader compacted, so on a full restart just that one
   node failed and the other two (uncompacted) formed a quorum and served.
   Follower-side compaction persists a compacted base on EVERY voter, so all
   three fail recovery and no quorum forms — exposing the gap. Fix:
   `resume_active`/`start_group` also take the committed compaction floors
   (`committed_compaction_floors`) and accept a base matching a kind-105
   decision OR a kind-110 floor.
4. **Observability**: status exposes `confirmed_floor` per group (the
   replicated floor a replica has applied, `None` until the leader
   publishes it) alongside `log_first_index`.

## Known limits, deliberately out of scope

No automatic floor selection beyond the existing `KV9_AUTO_COMPACT_ENTRIES`
trigger, no bounded absolute log size, no physical disk reclamation of the
append-only raft-log file. No chaos campaign, no performance claims. The
confirmation trusts the group leader to publish only an all-matched floor
(crash-fault model, as everywhere else) — a byzantine leader is out of scope.

## Evidence

- Serial qualifying workspace run plus strict Clippy/formatting; new raft
  unit tests (command codec roundtrip for `CompactionConfirmed`; apply
  monotonicity + foreign-region refusal for `apply_compaction_confirmed`).
- Accepted five-process e2e (`scripts/follower-compaction-e2e.py`, real
  MinIO): sustained writes with ZERO manual verbs, auto-compaction proposes
  floors, and EVERY voter's `confirmed_floor` and `log_first_index` advance
  past 1 (not just the leader's); a second burst advances every voter's
  floor further; a full-cluster restart recovers on the compacted logs of
  ALL THREE voters (the recovery gate accepts the kind-110 floor) with every
  sampled key serving and the group still taking writes.
- Fifteen-theorem Lean model
  ([proofs/lean/follower-compaction](../proofs/lean/follower-compaction/README.md))
  with seven semantic mutation controls (including the recovery-authority
  gate) and two proof-policy controls; every sibling Lean model and the
  retention TLA/TLAPS model re-accepted after the source-pin refresh.

Validation packet: [docs/follower-compaction-v1](follower-compaction-v1/README.md).
