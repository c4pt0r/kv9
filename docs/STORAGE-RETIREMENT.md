# Local retirement of removed replicas

Updated 2026-09-17. Parent [#9](https://github.com/c4pt0r/kv9/issues/9),
D03 [#24](https://github.com/c4pt0r/kv9/issues/24). Builds directly on
[replica removal](REPLICA-REMOVAL.md) and closes the migration's local
loose end: the removed replica no longer runs at all.

## What this increment adds

1. **A durable Retired phase.** The local group record gains
   `Phase::Retired`: a checksummed, atomically published receipt that this
   store's replica of the group is permanently fenced. Older readers
   refuse the unknown phase instead of guessing.
2. **Reconcile-driven retirement.** Each turn, a committed kind-106
   removal decision naming THIS exact store — node **and** incarnation —
   retires the local replica automatically
   (`NodeRuntime::reconcile_retirement` →
   `RegionManager::retire_removed`): the excluded driver stops, the group
   leaves the raw directory, the Retired record is published, and the
   group lock stays held. Idempotent; a decision naming any other store is
   ignored; errors surface in the control status and retry.
3. **Restarts open nothing.** Discovery loads a Retired record, takes the
   group lock, and never opens the raft log or engine again. The storage
   stays durable and untouched — physical reclamation is deliberately a
   later increment — and the retired state survives any number of
   restarts.

## What it deliberately does not do

No physical deletion or reclamation, no stranded-learner recovery, no
split, placement, chaos campaign, or scaling claim. The
[3/6/9-host benchmark contract](HORIZONTAL-SCALING-PLAN.md) remains the
only acceptance path for effective horizontal scaling.

## Evidence

- Serial qualifying workspace run plus strict Clippy/formatting.
- Accepted five-process e2e (`scripts/retirement-e2e.py`, first attempt):
  the full qualified chain through removal, then the removed node retiring
  its replica **automatically** under the committed decision while
  running, the durable group storage intact, the retired state surviving
  restart with nothing reopened, and the surviving three-voter quorum
  committing throughout.
- Ten-theorem Lean model
  ([proofs/lean/storage-retirement](../proofs/lean/storage-retirement/README.md))
  with six semantic mutation controls and two proof-policy controls;
  fifteen sibling Lean models and the retention TLA/TLAPS model
  re-accepted.

Validation packet: [docs/storage-retirement-v1](storage-retirement-v1/README.md).
