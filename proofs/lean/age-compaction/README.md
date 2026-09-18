# Age-compaction model

A machine-checked model of AGE-based automatic compaction-floor selection
(`crates/server/src/runtime.rs` `reconcile_auto_compaction`, the
`KV9_AUTO_COMPACT_AGE_SECS` trigger). The entries and bytes triggers bound a
BUSY group; a LOW-TRAFFIC group whose log never grows past a size threshold
would keep its full log forever and replay ancient entries on recovery. The
age trigger compacts it at least every T. Execution safety (leader-only
truncation, the all-matched gate, the durable REC_COMPACTION record,
configuration recovery, follower-side confirmation) is entirely a premise
from the [group-compaction](../group-compaction) and
[follower-compaction](../follower-compaction) models; this model constrains
the AGE TRIGGER:

- a proposal fires ONLY when the oldest retained entry has aged past the
  threshold — measured as how long `first_index` has stayed put (a compaction
  is the only thing that advances it, so the clock times the CURRENT retained
  window);
- AND only when there is something to compact (a retained committed entry
  beyond the floor) — age alone never compacts an empty window;
- the proposed floor is always the replica's committed applied position
  (backed, never an unbacked or future index);
- the floor never regresses;
- a floor the group cannot yet execute is retried, never forced.

Thirteen checked theorems: `initial_safe`, `step_safe`, `run_safe`,
`a_proposal_requires_aging`, `a_proposal_requires_something_to_compact`,
`a_committed_floor_requires_a_proposal`,
`the_floor_is_always_committed_backed`, `the_floor_never_regresses`,
`an_unexecutable_floor_is_never_forced`, `an_unexecutable_floor_only_retries`,
`a_commit_is_permanent`, `aging_without_retained_proposes_nothing`,
`the_age_trigger_chain_commits_a_backed_floor`.

`scripts/prove-age-compaction.py` checks the positive build, audits every
theorem's axioms (only `propext`, `Classical.choice`, `Quot.sound`), compiles
six semantic mutation controls that must fail exactly at the defect (a
proposal without aging; a proposal without something to compact; a commit
without a proposal; an unbacked floor; a regressed floor; a forced
unexecutable floor), and refuses `sorry`/`axiom` injections (two policy
controls).

The model does not claim bounded absolute log size, physical reclamation of
the append-only log, chaos acceptance, or any performance property. The age
clock is a LOCAL observation (how long `first_index` has stayed put on this
replica); it resets after restart, which is correct — recovery replays the
retained log as it stands and the clock restarts.
