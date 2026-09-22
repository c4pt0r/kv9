# Log-reclamation model

A machine-checked crash-safety model of physical raft-log reclamation
(`crates/raft/src/storage.rs` `DiskRaftStorage::rewrite_log`, driven by
`reconcile_auto_reclaim` under `KV9_RECLAIM_RAFT_LOG_BYTES`). The append-only
`raft.log` is never rewritten by compaction (only `first_index` advances), so
the file grows without bound. Reclamation rewrites it to contain only the live
state — the compacted base (a new `REC_RETAINED_BASE` record), the retained
entries, the configuration history and the current HardState — dropping the
compacted-away prefix from disk.

The recovered-content correctness (the rewritten records replay to an IDENTICAL
runtime view) is carried by the storage round-trip test
(`rewrite_log_round_trips_a_compacted_base_and_shrinks_the_file`); this model
constrains the CRASH-SAFETY of the file swap (build tmp → fsync → atomic rename
→ fsync dir):

- the tmp is renamed into place ONLY after it is fully built and fsync'd — never
  a partial file becoming the live log;
- the image is built ONLY from a COMPLETE reclamation record set (every live
  committed entry, the base and the preserved HardState);
- consequently no committed entry is ever lost and the committed watermark
  never regresses, at ANY crash point (a crash before the rename recovers the
  intact original; a crash at/after it recovers the complete new file — the
  atomic rename is the single commit point);
- recovery never reads the tmp file (it only ever opens `raft.log`).

Twelve checked theorems: `initial_safe`, `step_safe`, `run_safe`,
`a_build_requires_a_complete_image`, `a_rename_requires_a_built_image`,
`no_committed_entry_is_ever_lost`, `the_commit_never_regresses`,
`no_partial_file_becomes_live`, `recovery_never_reads_the_temp_file`,
`a_rename_is_permanent`, `a_crash_before_the_rename_keeps_the_original`,
`the_full_reclamation_completes`.

`scripts/prove-log-reclamation.py` checks the positive build, audits every
theorem's axioms (only `propext`, `Classical.choice`, `Quot.sound`), compiles
six semantic mutation controls that must fail exactly at the defect (a rename
without a built image; a build from an incomplete image; a rename that loses a
committed entry; a rename that regresses the commit; a partial file becoming
live; recovery reading the temp file), and refuses `sorry`/`axiom` injections
(two policy controls).

The model does not claim reclamation of a protocol-snapshot or lease group
(a documented v1 scope limit — the caller skips them), chaos acceptance, or any
performance property.
