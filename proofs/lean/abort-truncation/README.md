# Abort-truncation model

A machine-checked model of abort-settled source-log truncation
(`crates/meta/src/data_groups/truncation.rs` extended: the committed
SETTLEMENT authorizing a truncation decision is install evidence — the
completed transfer, whose cut bounds the floor — OR a committed abort —
the abandoned transfer whose detached learner no longer consumes the
retained tail). The raft compaction seam's safety (leader-only, durable
REC_COMPACTION, tail preservation, the deferred-sync barrier) and the
settlement rows' integrity are premises from their models
([source-truncation](../source-truncation),
[migration-abort](../migration-abort),
[deferred-sync](../deferred-sync)); this model constrains the AUTHORITY
CHAIN:

- a decision requires exactly ONE committed settlement (evidence and
  abort are mutually exclusive, and an unsettled operation never
  truncates);
- compaction additionally requires the RELEASED source pin and EVERY
  voter matched at or beyond the floor (a lagging voter blocks);
- the floor never exceeds the evidence cut on the evidence path, and
  only the LOCAL log prefix is ever discarded (followers keep theirs).

Thirteen checked theorems: `initial_safe`, `step_safe`, `run_safe`,
`evidence_and_abort_never_both_settle`,
`a_decision_requires_one_committed_settlement`,
`compaction_requires_decision_pin_and_matched_voters`,
`no_unsettled_operation_ever_truncates`,
`the_floor_never_exceeds_the_evidence_cut`,
`only_the_local_prefix_is_ever_discarded`,
`a_lagging_voter_blocks_compaction`, `a_settlement_is_permanent`,
`an_abort_alone_compacts_nothing`, `the_abort_settled_chain_compacts`.

`scripts/prove-abort-truncation.py` checks the positive build, audits
every theorem's axioms (only `propext`, `Classical.choice`,
`Quot.sound`), compiles five semantic mutation controls that must fail
exactly at the defect (both settlements coexisting; a decision without
any settlement; compaction without the released pin; compaction with a
lagging voter; compaction discarding a foreign prefix), and refuses
`sorry`/`axiom` injections (two policy controls).

The model does not claim healthy-group (non-migration) log compaction,
bounded log growth in general, physical disk reclamation of compacted
segments, chaos acceptance, or any performance property.
