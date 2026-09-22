# Log-reclamation crash-injection validation packet

See [the mechanism, the crash-safe swap and the recovery-robustness
fix](../LOG-RECLAMATION.md).

This increment empirically validates the crash-safety that the
[log-reclamation](log-reclamation-v1/README.md) Lean model proves, and
fixes a recovery bug the test uncovered.

- One accepted deterministic crash-injection e2e (`e2e = crash-fourth`,
  `scripts/raft-log-reclaim-crash-e2e.py`, real MinIO). A test-only env
  hook (`KV9_RECLAIM_TEST_ABORT`, unset in production) aborts a follower
  victim EXACTLY at each safety-critical point of the reclamation rewrite:
  - **before-rename** — the victim aborts (SIGABRT), the on-disk shape is
    confirmed (raft.log intact + an orphaned complete raft.log.tmp), the
    victim restarts, recovers on the intact original, catches up, and all
    64 committed keys survive.
  - **after-rename** — the victim aborts, the on-disk shape is confirmed
    (the tmp is renamed away; raft.log is already the rewritten file), the
    victim restarts, recovers on the new file, catches up, and all 90
    committed keys survive.
  The victim recovering to active on its crash-time log (an unrecoverable
  torn/corrupt log would fail to open) plus catching up to the group's
  committed index is the crash-safety proof; committed keys are read back
  through the serving leader.
- The crash test uncovered and this increment FIXES a latent recovery
  bug: a replica that restarts while its group has compacted PAST that
  replica's on-disk base failed to recover (the gate accepted only a base
  equal to the HIGHEST committed floor; a lagging replica's base is an
  earlier committed floor). Recovery now accepts a base any committed
  floor reaches at or beyond. This also hardens the shipped
  follower-compaction feature (a node down during active compaction can
  rejoin).
- Serial qualifying workspace run, zero failures; strict Clippy (0) and
  formatting pass; the log-reclamation Lean crash-safety model and all
  sibling models + the retention TLA/TLAPS model re-accepted.
- Covers process-crash safety. Power-loss durability of the post-rename
  directory entry (before its fsync) is not exercised here — the OS
  buffers survive a process abort; the fsync-before-publish ordering and
  the Lean model carry that logically.

Evidence archive: `evidence.tar.gz` (manifest in `validation.json`,
summary in `portable-readback.json`). Excludes ELF binaries, the
`root.bin` bootstrap credential, and all object-store credentials.
