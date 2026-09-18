# Abort-settled source-log truncation (D03 closure)

Updated 2026-09-18. Parent [#9](https://github.com/c4pt0r/kv9/issues/9),
D03 [#24](https://github.com/c4pt0r/kv9/issues/24). The migration-abort
increment left one documented limit: an aborted operation's source kept
its full raft log forever, because the truncation decision required
committed install evidence. That limit is closed.

## What this increment adds

The committed SETTLEMENT authorizing a truncation decision is now
either kind of settlement — mutually exclusive per operation, exactly
one required:

- **install evidence** (the completed transfer): unchanged — the floor
  stays at or below the evidence cut, protecting the very learner the
  evidence names;
- **a committed abort** (the abandoned transfer): the learner is
  detached and no longer consumes the retained tail, so no cut bound
  applies — the floor's guards are the UNCHANGED raft compaction gates
  (leader-only, EVERY voter matched at or beyond the floor, the durable
  REC_COMPACTION record, tail preservation, the deferred-sync barrier)
  plus the RELEASED source pin, which the abort settlement produces
  through the unchanged ledger verbs.

The decision row's settlement reference now points at either the
kind-104 evidence row or the kind-108 abort row; planning and readback
resolve and cross-validate whichever exists, and refuse if both or
neither do. The runtime derives the retention owner ids from either
settlement's destination incarnation. Compaction remains LOCAL prefix
only — followers keep their logs, and a voter matched at the floor
never re-fetches below it.

## Evidence

- Serial qualifying workspace run plus strict Clippy/formatting; new
  catalog test: an abort settles truncation without an evidence cut
  (plan, readback, idempotent confirm, divergent floor refusal), with
  the unsettled and double-settled paths refusing.
- Accepted five-process e2e (`scripts/abort-truncation-e2e.py`,
  extending the full stranded-recovery chain): stranding → committed
  abort → ledger settlement → learner detach → **truncation decision +
  compaction under the raft gates (first_index advanced past the
  floor)** → every voter restarts on the compacted log and the group
  keeps writing → revoke/re-admit/takeover → a fresh incarnation
  re-migrates the COMPACTED region end to end. Retained failures: a
  token-rename slip, and two metadata-leader-migration walls that
  produced per-verb leader re-resolution in the fixture.
- Thirteen-theorem Lean model
  ([proofs/lean/abort-truncation](../proofs/lean/abort-truncation/README.md))
  with five semantic mutation controls and two proof-policy controls;
  thirty sibling Lean models and the retention TLA/TLAPS model
  re-accepted.

## Known limits, deliberately out of scope

Healthy-group (non-migration) log compaction and bounded log growth in
general remain open — this increment extends the MIGRATION settlement
family only. No physical disk reclamation of compacted segments, no
chaos campaign, no performance claims.

Validation packet: [docs/abort-truncation-v1](abort-truncation-v1/README.md).
