# Byte-compaction model

A machine-checked model of BYTE-based automatic compaction-floor selection
(`crates/server/src/runtime.rs` `reconcile_auto_compaction`, the
`KV9_AUTO_COMPACT_BYTES` trigger; the retained-byte signal is
`DiskRaftStorage::retained_committed_bytes` /
`RaftPeer::retained_log_bytes`). Entries are a poor proxy for log cost when
value sizes vary, so a group of large values must be able to bound its raft
log by the retained committed PAYLOAD bytes. Execution safety (leader-only
truncation, the all-matched gate, the durable REC_COMPACTION record,
configuration recovery, follower-side confirmation) is entirely a premise
from the [group-compaction](../group-compaction) and
[follower-compaction](../follower-compaction) models; this model constrains
the BYTE TRIGGER:

- a proposal fires ONLY when the retained bytes crossed the threshold — entry
  growth alone, however large, proposes nothing under the byte trigger (the
  entries trigger is independent and, in the byte-only configuration, off);
- the proposed floor is always the replica's committed applied position
  (backed, never an unbacked or future index);
- the floor never regresses;
- a floor the group cannot yet execute is retried, never forced.

Twelve checked theorems: `initial_safe`, `step_safe`, `run_safe`,
`a_proposal_requires_grown_bytes`, `a_committed_floor_requires_a_proposal`,
`the_floor_is_always_committed_backed`, `the_floor_never_regresses`,
`an_unexecutable_floor_is_never_forced`, `an_unexecutable_floor_only_retries`,
`a_commit_is_permanent`, `entries_growth_alone_proposes_nothing`,
`the_byte_trigger_chain_commits_a_backed_floor`.

`scripts/prove-byte-compaction.py` checks the positive build, audits every
theorem's axioms (only `propext`, `Classical.choice`, `Quot.sound`), compiles
six semantic mutation controls that must fail exactly at the defect (a
proposal without grown bytes; a proposal firing on entry growth alone; a
commit without a proposal; an unbacked floor; a regressed floor; a forced
unexecutable floor), and refuses `sorry`/`axiom` injections (two policy
controls).

The model does not claim age-based selection (a future increment; it needs
per-entry wall-clock), bounded absolute log size, physical reclamation of the
append-only log, chaos acceptance, or any performance property. The retained
byte metric excludes per-entry framing overhead (the payload sum dominates
and is the size an operator reasons about).
