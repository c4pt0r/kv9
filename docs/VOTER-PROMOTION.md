# Voter promotion under committed evidence

Updated 2026-09-17. Parent [#9](https://github.com/c4pt0r/kv9/issues/9),
D03 [#24](https://github.com/c4pt0r/kv9/issues/24). Builds directly on
[destination evidence](DESTINATION-EVIDENCE.md) and
[source truncation](SOURCE-TRUNCATION.md).

## What this increment adds

The migration destination — attached, adopted, evidenced, caught up — can
finally become a **voter** of the source group.

1. **Committed authority, no new row.** `PromoteMigrationVoter` (CLI
   `promote-migration-voter`) requires, from the leader's local applied
   metadata, the committed migration intent AND the committed
   destination-install evidence row: the destination provably adopted the
   exact pinned image. Local absence refuses in the safe direction.
2. **Through the group's own log.** Leader-only: the attached learner is
   promoted with a single committed configuration entry
   (`ConfChangeType::AddNode`), the apply is awaited, and the resulting
   voter set is verified and returned. An existing voter confirms
   idempotently; a missing learner refuses.
3. **Restart under committed configuration history.** The fixed-group
   restart validator now accepts any membership the group's own
   unambiguous committed configuration history evolved from the creation
   voters — an attached learner, a promoted voter — and keeps refusing
   ambiguous or foreign histories.

## What it deliberately does not do

No replica removal, no stranded-learner recovery, no split, placement,
chaos campaign, or scaling claim. Reads still refuse at every non-leader.
The [3/6/9-host benchmark contract](HORIZONTAL-SCALING-PLAN.md) remains
the only acceptance path for effective horizontal scaling.

## Evidence

- Serial qualifying workspace run plus strict Clippy/formatting.
- Accepted five-process e2e (`scripts/promotion-e2e.py`, first attempt):
  the full qualified chain (settlement + committed compaction), then a
  wrong-operation promotion refusal, promotion to voters `1,2,3,4` with an
  idempotent retry, a write committed with one ORIGINAL voter down (the
  promoted voter participates in the quorum), non-leader read refusal,
  restart of the promoted voter through its adopted base, rejoin of the
  stopped original voter under the evolved configuration, and every member
  tracking the final write.
- Eleven-theorem Lean model
  ([proofs/lean/voter-promotion](../proofs/lean/voter-promotion/README.md))
  with seven semantic mutation controls and two proof-policy controls;
  thirteen sibling Lean models and the retention TLA/TLAPS model
  re-accepted.

Validation packet: [docs/voter-promotion-v1](voter-promotion-v1/README.md).
