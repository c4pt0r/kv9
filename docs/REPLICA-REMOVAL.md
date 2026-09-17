# Source replica removal under committed decisions

Updated 2026-09-17. Parent [#9](https://github.com/c4pt0r/kv9/issues/9),
D03 [#24](https://github.com/c4pt0r/kv9/issues/24). Builds directly on
[voter promotion](VOTER-PROMOTION.md). This is the migration's other half:
the group finally MOVES off one source replica instead of only growing.

## What this increment adds

1. **Committed removal decisions** (TASKS kind 106, `KV9RMV01`,
   `crates/meta/src/data_groups/removal.rs`). One immutable decision per
   operation naming one EXACT source replica — node and store incarnation,
   validated against the creation's initial replicas — and never the
   migration destination. Bound to the committed evidence row; a replaced
   disk cannot be named; idempotent confirmation; divergence refuses.
2. **Leader-side execution** (`RemoveSourceReplica`, CLI
   `remove-source-replica`). Requires the committed decision AND evidence
   from local applied metadata, then: the destination must already vote
   (the replacement is real), at least three voters must remain, a leader
   never removes itself, and the removal is one committed
   `ConfChangeType::RemoveNode` entry with awaited apply and a verified
   voter-set receipt. Already-removed confirms idempotently.
3. **The removed replica is isolated, not deleted.** Its local group
   record and storage stay durable and fenced by membership: out of the
   configuration it cannot vote, serve, or be replicated to, and its node
   restarts as a healthy non-member. Physical retirement/reclamation of
   that storage is deliberately left to a later increment.

## What it deliberately does not do

No local storage retirement or reclamation, no stranded-learner recovery,
no split, placement, chaos campaign, or scaling claim. The
[3/6/9-host benchmark contract](HORIZONTAL-SCALING-PLAN.md) remains the
only acceptance path for effective horizontal scaling.

## Evidence

- Serial qualifying workspace run plus strict Clippy/formatting; catalog
  unit tests: only an initial creation replica may be named (never the
  destination, never a replaced disk), one retirement per operation,
  idempotent confirm, and the readback defect matrix.
- Accepted five-process e2e (`scripts/removal-e2e.py`; one failed launch
  retained — the fixture reused a stale metadata leader after the earlier
  restarts, a real harness lesson, not a server defect): the full
  qualified chain through promotion, then removal refusal without the
  decision, the destination refused as target, removal to exactly three
  voters including the migrated destination (idempotent), writes committed
  by the surviving quorum, and the removed node restarting as a healthy
  isolated non-member while the survivors keep committing.
- Twelve-theorem Lean model
  ([proofs/lean/replica-removal](../proofs/lean/replica-removal/README.md))
  with seven semantic mutation controls and two proof-policy controls;
  fourteen sibling Lean models and the retention TLA/TLAPS model
  re-accepted.

Validation packet: [docs/replica-removal-v1](replica-removal-v1/README.md).
