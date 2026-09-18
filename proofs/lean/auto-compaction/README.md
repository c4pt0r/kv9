# Auto-compaction model

A machine-checked model of automatic compaction-floor selection over the
proven manual group-compaction pipeline
(`crates/server/src/runtime.rs::reconcile_auto_compaction`, gated by
`KV9_AUTO_COMPACT_ENTRIES`, default 0 = off). Every safety property of
EXECUTION — leader-only, the all-matched gate, the durable
REC_COMPACTION record, configuration recovery — is a premise from the
[group-compaction](../group-compaction) model; this model constrains
only the AUTOMATIC TRIGGER:

- a floor is proposed only when the retained log has grown past the
  threshold;
- the proposed floor is always the replica's APPLIED position — a
  committed, durable point, never an unbacked or future index;
- it never proposes a floor below or equal to the last committed floor
  (strictly increasing: no churn, no regression);
- a committed floor the group cannot yet execute (a lagging voter) is
  simply retried by the gated reconcile, never forced.

Thirteen checked theorems: `initial_safe`, `step_safe`, `run_safe`,
`a_proposal_requires_a_grown_log`,
`a_committed_floor_requires_a_proposal`,
`the_floor_is_always_committed_backed`, `the_floor_never_regresses`,
`an_unexecutable_floor_is_never_forced`,
`an_unexecutable_floor_only_retries`, `a_commit_is_permanent`,
`a_grown_log_alone_commits_nothing`,
`the_trigger_chain_commits_a_backed_floor`.

`scripts/prove-auto-compaction.py` checks the positive build, audits
every theorem's axioms (only `propext`, `Classical.choice`,
`Quot.sound`), compiles five semantic mutation controls that must fail
exactly at the defect (a proposal without a grown log; a commit without
a proposal; an unbacked floor; a regressed floor; a forced unexecutable
floor), and refuses `sorry`/`axiom` injections (two policy controls).

The model does not claim follower-side compaction, a specific
size/age policy, bounded absolute log size, chaos acceptance, or any
performance property.
