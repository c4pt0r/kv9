# Follower-compaction model

A machine-checked model of follower-side raft-log compaction. The
group-compaction increment bounded only the leader's log; a follower
kept its full log until it led. This increment closes that edge: after
the leader's all-matched truncation it publishes the confirmed floor
into the group's OWN raft log via an ordered
`Command::CompactionConfirmed` (`crates/raft/src/command.rs`,
applied by `apply_compaction_confirmed` in
`crates/raft/src/state_machine/data_range.rs` to the reserved
`kv9_common::data_range::COMPACTION_CONFIRMED_KEY`). Every voter reads
that replicated floor and compacts its own prefix through
`RaftPeer::compact_confirmed_prefix`; `reconcile_compaction`
(`crates/server/src/runtime.rs`) drives both stages.

The leader's own compaction safety (leader-only truncation, the
all-matched gate, the durable REC_COMPACTION record, configuration
recovery) is a premise from the [group-compaction](../group-compaction)
model; this model constrains the CONFIRMATION→FOLLOWER chain and the
recovery gate that follower-side compaction forced closed:

- the committed all-matched confirmation is recorded ONLY after the
  leader's all-matched truncation — so a committed confirmation proves
  every voter matched the floor;
- a follower compacts its own prefix ONLY under that committed
  confirmation AND once its own applied position has reached the floor;
- no voter is ever stranded (a committed entry at or below a confirmed
  floor is held by every voter — a new member is image-installed above
  the floor, not log-replayed);
- only the local prefix is discarded;
- a restarting voter adopts its durable compacted base into a live peer
  ONLY under committed authority (the truncation that produced it): a
  base with no committed decision behind it is refused, never adopted.
  Follower-side compaction persists a compacted base on EVERY voter, so
  recovery must honor the committed floor — `region_manager` accepts a
  base matching a kind-105 truncation decision OR a kind-110 compaction
  floor (`crates/server/src/region_manager.rs`).

Fifteen checked theorems: `initial_safe`, `step_safe`, `run_safe`,
`a_confirmation_requires_the_leaders_all_matched_truncation`,
`a_follower_compacts_only_under_a_confirmation`,
`no_voter_is_ever_stranded`,
`only_the_local_prefix_is_ever_discarded`,
`no_confirmation_without_all_matched`,
`a_recovered_base_requires_committed_authority`,
`no_base_is_adopted_without_authority`,
`a_follower_needs_its_own_applied_floor`,
`a_confirmation_is_permanent`,
`a_confirmation_alone_compacts_no_follower`,
`the_confirmed_chain_compacts_a_follower`,
`a_truncation_lets_a_voter_adopt_its_base`.

`scripts/prove-follower-compaction.py` checks the positive build, audits
every theorem's axioms (only `propext`, `Classical.choice`,
`Quot.sound`), compiles seven semantic mutation controls that must fail
exactly at the defect (a confirmation without the leader truncation; a
follower compacting without a confirmation; without its own applied
floor; a follower compaction stranding a voter; a confirmation skipping
all-matched; a base adopted without committed authority; an adopted base
flagging an authority defect), and refuses `sorry`/`axiom` injections
(two policy controls).

The model does not claim automatic floor selection, bounded absolute
log size, physical reclamation of the append-only log, chaos
acceptance, or any performance property.
