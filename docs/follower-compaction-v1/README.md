# Follower-compaction validation packet

See [the mechanism, the recovery-gate fix and the
limits](../FOLLOWER-COMPACTION.md).

- 951 passing workspace tests/doctests in the serial qualifying run
  (including the new raft `CompactionConfirmed` codec roundtrip and the
  `apply_compaction_confirmed` monotonicity + foreign-region-refusal
  tests, and the updated `region_manager` recovery test), zero failures;
  strict Clippy (0 warnings) and formatting pass.
- Fifteen checked Lean theorems for the follower-compaction model with
  seven semantic mutation controls (including the recovery-authority
  gate) and two proof-policy controls; all thirty-four sibling Lean
  models re-accepted against the exact working tree after the
  source-pin refresh.
- One accepted five-process real-MinIO e2e (`e2e-cmd`): sustained writes
  with ZERO manual verbs; auto-compaction proposes floors and EVERY
  voter's `confirmed_floor` and `log_first_index` advance past 1 (not
  just the leader's); a second burst advances every voter's floor
  further; a full-cluster restart recovers on the compacted logs of ALL
  THREE voters with every sampled key serving and the group still
  writing.
- The retained diagnostic failures are load-bearing findings. `e2e-diag`
  proved the first design wrong: a fenced write of the reserved key was
  SILENTLY rejected by the data group's `RangeFence` (which admits only
  Raw user keys in range), so `confirmed_floor` stayed `None` on every
  node and earlier "passes" were only leadership churn letting each node
  self-truncate as leader — this drove the redesign to a dedicated
  ordered `Command::CompactionConfirmed`. `e2e-accept` then surfaced a
  LATENT bug in the shipped group-compaction increment: `region_manager`
  recovery accepted a compacted base only under a kind-105 truncation
  decision, never a kind-110 compaction floor; group-compaction's own
  e2e missed it because only the leader compacted (a quorum of two
  uncompacted nodes masked the one failed node), whereas follower-side
  compaction persists a base on every voter — fixed by threading the
  committed compaction floors into recovery.

Evidence archive: `evidence.tar.gz` (manifest in `validation.json`,
summary in `portable-readback.json`). Excludes ELF binaries, the
`root.bin` bootstrap credential, and all object-store credentials.
