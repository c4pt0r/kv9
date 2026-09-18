# Healthy-group raft-log compaction (bounded log growth)

Updated 2026-09-18. Parent [#9](https://github.com/c4pt0r/kv9/issues/9),
P1 storage [#20](https://github.com/c4pt0r/kv9/issues/20). Ordinary data
groups' raft logs grew forever: only migration flows (source truncation,
abort-settled truncation) could compact. This generalizes compaction to
any healthy group.

## What this increment adds

1. **A committed kind-110 floor** (`KV9CMP01`,
   `crates/meta/src/data_groups/compaction.rs`): immutable,
   creation-bound, one HIGHEST floor per region; `plan_compaction`
   enforces strictly-increasing floors (an equal floor confirms
   idempotently; a lower or term-regressed floor refuses). No
   migration, settlement or retention involvement.
2. **`record-group-compaction`** at the metadata leader commits the
   floor; **`reconcile_compaction`** executes it. v1 executes at the
   GROUP LEADER only, through the existing all-matched-gated truncation
   seam (leader-only, every voter matched at or beyond the floor, the
   durable REC_COMPACTION record, tail preservation), behind the
   data-group engine's deferred-sync barrier. A committed entry is held
   by a quorum, so compacting the leader's prefix once all voters have
   matched strands no one.
3. **The configuration-recovery fix (the increment's core).** The raft
   configuration lookup refused ALL compacted logs
   (`first_index != 1 → ProtocolHistoryCompacted`), so a SECOND
   compaction floor above an earlier one failed with "no committed
   configuration". It now inherits the durable base configuration
   (restored into the mem snapshot by `compact_memory`) when the cut
   sits at or above the compacted base and the retained tail holds no
   unapplied configuration change; a cut below the base still refuses.
   This ALSO fixes migration re-truncation on an already-compacted log.
4. **Observability**: status exposes `log_first_index` per group (the
   on-disk raft log is append-only — the compaction signal is the
   first-index advance, not a file-byte drop).

## Known limits, deliberately out of scope

Follower-side log bounding: v1 compacts at the leader only (a lagging
follower keeps its full log until it leads or the leader's floor
becomes safe for it) — the documented open edge. No automatic floor
selection (an operator or a future policy chooses the floor), no bounded
absolute log size, no physical disk reclamation of compacted records
(the raft log file is append-only; a rewrite/rotation is separate). No
chaos campaign, no performance claims.

## Evidence

- Serial qualifying workspace run plus strict Clippy/formatting; new
  catalog tests (strictly-increasing floors, creation binding, readback
  rebinding refusals) and the rewritten configuration-recovery test (a
  compacted base resolves a configuration; a cut below it refuses).
- Accepted five-process e2e (`scripts/group-compaction-e2e.py`): two
  write bursts, each followed by a committed floor whose execution
  advances the group leader's `log_first_index` past the floor; the
  SECOND round exercises the configuration-recovery fix; a stale floor
  refuses; a full-cluster restart recovers on the compacted logs (the
  startup history gates accept the durable compacted bases) with every
  sampled key serving and the group still taking writes.
- Thirteen-theorem Lean model
  ([proofs/lean/group-compaction](../proofs/lean/group-compaction/README.md))
  with five semantic mutation controls and two proof-policy controls;
  thirty-one sibling Lean models and the retention TLA/TLAPS model
  re-accepted.

Validation packet: [docs/group-compaction-v1](group-compaction-v1/README.md).
