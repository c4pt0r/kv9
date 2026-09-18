# Migration-abort model

A machine-checked model of the committed migration abort — the
stranded-destination recovery (`crates/meta/src/data_groups/abort.rs`,
kind 108, `KV9ABT01`). A destination store incarnation lost
mid-operation can never adopt or evidence its image; without the abort
the operation wedges forever: the source pin sticks at Published,
truncation stays blocked and the attached learner stays in the source
configuration. Each underlying mechanism's safety (the migration
intent, install evidence, the retention ledger's pin discipline,
learner attach) is a premise from its own model
([migration-authority](../migration-authority),
[install-evidence](../install-evidence),
[learner-attach](../learner-attach)); this model constrains the
SETTLEMENT algebra:

- evidence and abort are mutually exclusive per operation — permanently,
  in both directions (evidence planning refuses on a committed abort;
  abort planning refuses on committed evidence);
- the source pin releases only against ONE committed settlement (the
  ledger's quiesce fence accepts evidence-matched OR abort-matched
  owner pairs, same subject-and-operation-digest discipline);
- the stranded learner detaches and the region re-migrates only under
  the committed abort (a live or evidenced predecessor keeps the region
  blocked; readback stays whole with the aborted history committed);
- an aborted operation never adopts (the reconcile skips it; attach
  refuses permanently);
- the destination image pin never drops (the abandoned image stays
  pinned — a documented cost until object-store reclamation exists).

Thirteen checked theorems: `initial_safe`, `step_safe`, `run_safe`,
`evidence_and_abort_never_coexist`,
`release_requires_one_committed_settlement`,
`detach_requires_the_committed_abort`,
`remigration_requires_the_committed_abort`,
`no_aborted_operation_ever_adopts`, `the_destination_pin_never_drops`,
`an_abort_is_permanent`, `a_confirmation_changes_nothing`,
`a_migration_alone_settles_nothing`, `the_stranded_chain_recovers`.

`scripts/prove-migration-abort.py` checks the positive build, audits
every theorem's axioms (only `propext`, `Classical.choice`,
`Quot.sound`), compiles five semantic mutation controls that must fail
exactly at the defect (evidence landing on an aborted operation; an
abort landing on an evidenced operation; release without any
settlement; detach without the abort; re-migration without the abort),
and refuses `sorry`/`axiom` injections (two policy controls).

The model does not claim source-log truncation for aborted operations,
object-store reclamation of abandoned images, liveness of the operator
verbs, chaos acceptance, or any performance property.
