# Cascade-split model

A machine-checked model of cascade splits over the proven single-split
pipeline: what must stay true when the children of a published automatic
split re-trigger and publish their own splits. Each individual split's
safety (intent, seal, population, atomic publication, retirement) is a
premise from its own model ([split-intent](../split-intent),
[parent-seal](../parent-seal), [child-population](../child-population),
[split-publication](../split-publication),
[parent-retirement](../parent-retirement),
[auto-split](../auto-split)); this model constrains the CASCADE history.

Both constraints are the two real defects found and fixed in the
cascade sealed-unpublished hang (`scripts/cascade-split-e2e.py`
reproduces it; the preserved e2e-third state recovered end to end under
the fixed code):

1. **A published intent's readback validity is permanent.** The
   committed directory's exact child boundaries — which never change
   after binding — are the evidence of a publication. Requiring the
   children to remain UNSEALED made a grandchild's publication
   retroactively invalidate its grandparent's intent row, and that one
   row froze the entire split subsystem (readback is all-or-nothing by
   design: genuine divergence must refuse).
2. **No published parent is starved.** Retirement confirms
   idempotently; processing only the FIRST published intent per
   reconcile turn parked every later cascade parent behind the earliest
   intent's no-op confirmation forever.

Plus the observability invariant the diagnosis depended on: a recorded
refusal stays observable — no later clean reconcile step erases it (the
control-error field was previously overwritten by the final step's
success every turn).

Twelve checked theorems: `initial_safe`, `step_safe`, `run_safe`,
`valid_history_never_refuses_readback`, `no_published_parent_is_starved`,
`no_recorded_refusal_is_erased`,
`a_childs_seal_requires_the_published_parent`,
`retirement_requires_the_publication`, `a_publication_is_permanent`,
`a_childs_seal_never_invalidates_its_grandparent`,
`the_second_pipeline_completes_after_a_siblings_seal`,
`an_earlier_confirmation_starves_no_later_parent`.

`scripts/prove-cascade-split.py` checks the positive build, audits every
theorem's axioms (only `propext`, `Classical.choice`, `Quot.sound`),
compiles five semantic mutation controls that must fail exactly at the
defect (a child's seal refusing the grandparent's readback; a
publication erasing a recorded refusal; an earlier confirmation starving
the later parent; a child sealing without its published parent;
retirement without the publication), and refuses `sorry`/`axiom`
injections (two policy controls).

The model does not claim load-based triggers, physical reclamation,
bounded cascade depth, chaos acceptance, or any performance property.
