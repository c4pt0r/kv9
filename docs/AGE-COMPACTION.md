# Age-based auto-compaction (bound a quiet group's log by time)

Updated 2026-09-18. Parent [#9](https://github.com/c4pt0r/kv9/issues/9),
P1 storage [#20](https://github.com/c4pt0r/kv9/issues/20). The
[entries](AUTO-COMPACTION.md) and [bytes](BYTE-COMPACTION.md) triggers bound
a BUSY group. A LOW-TRAFFIC group whose log never grows past a size threshold
keeps its full log forever and replays ancient entries on recovery. This adds
a time bound.

## What this increment adds

1. **`KV9_AUTO_COMPACT_AGE_SECS`** (0 = off): the seconds the oldest retained
   committed entry may linger before an automatic compaction floor is
   proposed, even when the entries and bytes thresholds are never reached. A
   quiet group compacts at least every T; a fully idle group with nothing new
   to compact does not (the guard requires a retained committed entry beyond
   the floor).
2. **A local, storage-free age clock.** Age is measured as how long this
   replica's `first_index` has stayed put — a compaction is the only thing
   that advances it, so the mark times the CURRENT retained window. A
   per-region `(first_index, since)` mark resets whenever `first_index`
   advances (a compaction executed). No per-entry timestamp is stored; the
   clock is a local observation that resets after restart, which is correct —
   recovery replays the retained log as it stands and the clock restarts.
3. **The same floor and execution.** The proposed floor is the replica's
   applied position, committed as the same kind-110 row and executed through
   the same all-matched-gated, follower-side path as the manual verb. Only
   the trigger differs; either of the three triggers firing proposes.

## Known limits, deliberately out of scope

No bounded absolute log size, no physical reclamation of the append-only log
file, no chaos campaign, no performance claims. The age clock is best-effort
wall-clock on the metadata leader's local replica; it is a compaction trigger,
never a safety-relevant value (the floor is a committed applied position and
the execution is all-matched gated).

## Evidence

- The serial qualifying workspace run (unchanged count; the increment is a
  runtime-only trigger addition), zero failures; strict Clippy (0) and
  formatting pass.
- Thirteen checked Lean theorems for the age-compaction model with six
  semantic mutation controls (including a proposal without something to
  compact, and a proposal without aging) and two proof-policy controls
  ([proofs/lean/age-compaction](../proofs/lean/age-compaction/README.md)); all
  sibling Lean models and the retention TLA/TLAPS model re-accepted against
  the exact working tree.
- One accepted five-process real-MinIO e2e (`scripts/auto-compact-age-e2e.py`):
  with BOTH size triggers OFF (`KV9_AUTO_COMPACT_ENTRIES=0`,
  `KV9_AUTO_COMPACT_BYTES=0`) and ZERO manual verbs, a dozen tiny values leave
  the group far below every size threshold (retained entries = 12), and only
  AFTER the idle 5 s age threshold elapses does every voter compact
  (`log_first_index` 1 → 15 on all three) — provably age-driven; a second tiny
  burst plus idle advances every voter's floor further; a full-cluster restart
  recovers on the compacted logs with every sampled key serving.

Validation packet: [docs/age-compaction-v1](age-compaction-v1/README.md).
