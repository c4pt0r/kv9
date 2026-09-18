# Group-compaction model

A machine-checked model of healthy-group raft-log compaction driven by
committed kind-110 floors (`crates/meta/src/data_groups/compaction.rs`,
executed by `reconcile_compaction` through the raft compaction seam;
`record-group-compaction` at the metadata leader). The seam's own safety
(leader-only execution, the all-matched peer gate, the durable
REC_COMPACTION record, tail preservation, the deferred-sync barrier) and
the catalog row's integrity are premises from their models
([source-truncation](../source-truncation),
[abort-truncation](../abort-truncation),
[deferred-sync](../deferred-sync)); this model constrains the
DECISION→EXECUTION chain and the repeated-compaction configuration
recovery:

- compaction requires the committed floor, the local applied position at
  or beyond it, and all voters matched (a lagging voter blocks);
- floors per region strictly increase, and a higher floor requires the
  earlier base to have compacted;
- a SECOND floor above an earlier compacted base still resolves its
  configuration — the fix at the heart of this increment: the
  configuration lookup previously refused ALL compacted logs, so a
  second compaction failed with "no committed configuration"; it now
  inherits the durable base configuration when the retained tail holds
  no unapplied config change;
- no committed entry is ever lost (a committed entry is held by a
  quorum), and only the LOCAL prefix is ever discarded.

Fifteen checked theorems: `initial_safe`, `step_safe`, `run_safe`,
`compaction_requires_floor_apply_and_matched_voters`,
`a_higher_floor_requires_the_first_compaction`,
`no_committed_entry_is_ever_lost`,
`only_the_local_prefix_is_ever_discarded`, `floors_never_regress`,
`recompaction_requires_the_earlier_base`,
`a_compaction_resolves_the_configuration`, `the_gated_chain_compacts`,
`a_second_floor_recompacts_and_resolves_its_configuration`,
`a_committed_floor_alone_compacts_nothing`.

`scripts/prove-group-compaction.py` checks the positive build, audits
every theorem's axioms (only `propext`, `Classical.choice`,
`Quot.sound`), compiles five semantic mutation controls that must fail
exactly at the defect (compaction without all-matched; without the local
apply; a higher floor without the base; a compaction losing a committed
entry; a compaction discarding a foreign prefix), and refuses
`sorry`/`axiom` injections (two policy controls).

The model does not claim follower-side log bounding (v1 executes at the
group leader only — the documented open edge), automatic floor
selection, bounded absolute log size, physical disk reclamation, chaos
acceptance, or any performance property.
