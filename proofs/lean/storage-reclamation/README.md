# Storage-reclamation model

A machine-checked model of physical reclamation for RETIRED local data
groups (`crates/server/src/region_manager.rs::reclaim_retired`, driven
by `reconcile_reclamation` when `KV9_RECLAIM_RETIRED=1`). Retirement's
own safety — the committed removal / published-split authority and the
durable Retired fence — is a premise from its models
([storage-retirement](../storage-retirement),
[parent-retirement](../parent-retirement),
[replica-removal](../replica-removal)); this model constrains the
DELETION itself:

- payload deletion happens only after the durable `Reclaimed` record
  commits it (record-before-delete: a crash between the two resumes
  idempotently at discovery, which finishes the deletion);
- that record itself requires the retired fence AND the re-verified
  committed authority (a sealed binding for the region, whose child
  bindings are the permanent publication evidence, or a committed
  removal decision naming this exact store) — a retired group with
  neither is left untouched;
- the fence is never lost: the record and lock survive every deletion,
  a reclaimed group never re-prepares and never serves again;
- active leaves are never touched.

Thirteen checked theorems: `initial_safe`, `step_safe`, `run_safe`,
`deletion_requires_the_durable_record`,
`the_durable_record_requires_the_retired_fence`,
`the_durable_record_requires_committed_authority`,
`the_fence_is_never_lost`, `no_active_leaf_is_ever_touched`,
`a_reclaimed_group_never_serves`, `deletion_is_permanent`,
`a_crash_after_the_record_resumes`,
`a_retired_group_alone_deletes_nothing`,
`the_authorized_chain_completes`.

`scripts/prove-storage-reclamation.py` checks the positive build, audits
every theorem's axioms (only `propext`, `Classical.choice`,
`Quot.sound`), compiles five semantic mutation controls that must fail
exactly at the defect (deletion without the durable record; the record
without the retired fence; the record without committed authority; a
deletion that loses the fence; a deletion that touches a leaf), and
refuses `sorry`/`axiom` injections (two policy controls).

The model does not claim object-store reclamation (retention-ledger
image pins remain the object-store authority), bounded disk usage,
chaos acceptance, or any performance property.
