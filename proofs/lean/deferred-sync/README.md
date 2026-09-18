# Deferred-sync model

A machine-checked model of deferred engine-apply synchronization for
data-group engines (`crates/engine/src/wal_segment.rs` +
`wal_stream.rs`, plumbed by `crates/server/src/region_manager.rs` under
`KV9_DATA_SYNC_DEFER_BYTES`, default 0 = strict). The raft log's own
persistence contract (fsync before send/commit) and the recovery
machinery (torn-tail truncation, committed-entry replay — exercised
today for a crash between the raft sync and the engine sync) are
premises; this model constrains the DEFERRAL:

- an acknowledgement requires the raft-durable entry and the applied
  write — never the engine sync;
- a crash loses only the unsynced engine tail (bounded by the byte
  window AND a 100ms age bound, which also keeps dirty pages from
  entangling other files' fsyncs in one journal commit);
- replay from the durable raft log restores the applied write: no
  acknowledged write is ever lost, and both raft durability and the
  acknowledgement itself survive a crash;
- the replay source is never discarded under deferral: log compaction
  requires the sync barrier first (`sync_applied_now` before
  `truncate_source_log`'s compaction).

Fourteen checked theorems: `initial_safe`, `step_safe`, `run_safe`,
`an_ack_requires_the_durable_raft_entry`, `no_acked_write_is_ever_lost`,
`the_replay_source_is_never_discarded_under_deferral`,
`an_ack_requires_the_applied_write`,
`compaction_requires_the_sync_barrier`,
`raft_durability_survives_a_crash`,
`an_acknowledgement_survives_a_crash`,
`a_crash_loses_only_the_unsynced_tail`,
`replay_restores_the_applied_write`,
`an_unsynced_crash_leaves_the_write_to_replay`,
`the_crashed_acked_write_reads_back_after_replay`.

`scripts/prove-deferred-sync.py` checks the positive build, audits every
theorem's axioms (only `propext`, `Classical.choice`, `Quot.sound`),
compiles five semantic mutation controls that must fail exactly at the
defect (an ack without the durable raft entry; a crash keeping the
unsynced tail; compaction discarding the replay source under deferral;
a sync covering an unwritten record; a crash dropping the
acknowledgement), and refuses `sorry`/`axiom` injections (two policy
controls).

The model does not claim performance numbers (the benchmark publishes
them under predeclared targets), metadata-catalog deferral (never
deferred), backpressure budgets, chaos acceptance, or bounded recovery
time.
