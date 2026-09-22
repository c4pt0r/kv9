# Physical raft-log reclamation (shrink the on-disk log)

Updated 2026-09-22. Parent [#9](https://github.com/c4pt0r/kv9/issues/9),
P1 storage [#20](https://github.com/c4pt0r/kv9/issues/20). Compaction advances
`first_index` but the APPEND-ONLY `raft.log` is never rewritten, so a
long-lived group's file grows without bound even though most of it is
compacted-away. The [observability groundwork](../docs/BYTE-COMPACTION.md)
exposed this as `log_file_bytes - retained_log_bytes`; this closes it.

## What this increment adds

1. **`KV9_RECLAIM_RAFT_LOG_BYTES`** (0 = off): the on-disk `raft.log` size past
   which a COMPACTED group physically reclaims its log. `reconcile_auto_reclaim`
   drives it per-replica (a LOCAL maintenance op, like follower-side compaction),
   gated on the file size AND on the group actually being compacted
   (`log_first_index > 1`, so there is waste to drop).
2. **A crash-safe rewrite** (`DiskRaftStorage::rewrite_log`). The live state is
   re-serialized into `raft.log.tmp` — the compacted base, the retained entries,
   the configuration history and the current HardState — then fsync'd and
   ATOMICALLY renamed over `raft.log` (the single commit point), then the
   directory is fsync'd. A crash BEFORE the rename leaves the original intact; a
   crash AT/AFTER it leaves the complete new file. No committed entry is ever
   lost and the commit watermark never regresses. The writer lock is held
   throughout, so no append races the swap. Recovery is UNCHANGED — it replays
   the rewritten records exactly as before.
3. **A new `REC_RETAINED_BASE` record.** A compacted base cannot be reproduced as
   entries-then-`REC_COMPACTION` (the floor's entries are already discarded), so
   the rewrite emits the base directly; recovery installs it via a base image
   while preserving the configuration history and lease (it is a compaction base,
   not a protocol snapshot). Older readers reject the new kind (forward-only).

## Known limits, deliberately out of scope

v1 reclaims ONLY ordinary groups — those with no protocol snapshot and no lease.
A protocol-snapshot base has its own `REC_SNAPSHOT` semantics (it resets the
configuration history), and a lease epoch is a SEQUENCE that a single retained
epoch cannot rebuild; both are skipped rather than reclaimed unsafely (an
ambiguous configuration history is also skipped). This covers the ordinary
fixed-voter data groups that grow via writes. No bounded absolute log size, no
chaos campaign, no performance claims.

## Evidence

- Serial qualifying workspace run (new storage round-trip and crash-safety unit
  tests: the rewrite recovers an IDENTICAL view and shrinks the file; a stale
  `raft.log.tmp` is ignored and a completed swap recovers the new file), zero
  failures; strict Clippy (0) and formatting pass.
- Twelve checked Lean crash-safety theorems
  ([proofs/lean/log-reclamation](../proofs/lean/log-reclamation/README.md)) with
  six semantic mutation controls (a rename without a built image; a build from an
  incomplete image; a rename that loses a committed entry or regresses the
  commit; a partial file becoming live; recovery reading the temp file) and two
  proof-policy controls; all sibling Lean models and the retention TLA/TLAPS
  model re-accepted.
- One accepted five-process real-MinIO e2e (`scripts/raft-log-reclaim-e2e.py`):
  sustained ~4 KiB writes grow every voter's append-only `raft.log` past the
  reclaim threshold; reclamation then PHYSICALLY shrinks each voter's file to
  under HALF its peak (peaks ~134 KiB → 33-58 KiB — a shrink the append-only log
  alone never delivered) while the compacted base survives (`log_first_index`
  stays > 1) and no key is lost; the group keeps serving and writing after the
  swap; a full-cluster restart recovers on the REWRITTEN logs with every sampled
  key serving.
- An accepted deterministic CRASH-INJECTION e2e (`scripts/raft-log-reclaim-crash-e2e.py`,
  env hook `KV9_RECLAIM_TEST_ABORT`, unset in production): a follower victim is
  aborted EXACTLY at each safety-critical point — `before-rename` (raft.log
  intact, orphaned tmp) and `after-rename` (raft.log already the rewritten file);
  the on-disk shape is verified; the victim restarts, recovers on its crash-time
  log (an unrecoverable torn/corrupt log would fail to open), rejoins and catches
  up, and every committed key survives. Covers process-crash safety; power-loss
  durability of the post-rename directory entry rests on the fsync-before-publish
  ordering and the Lean model.

## Recovery robustness (found by the crash test)

The crash test surfaced a latent bug in the compaction recovery gate, now fixed:
a replica that restarts while its group has compacted PAST that replica's on-disk
base failed to recover, because the gate accepted only a base EQUAL to the
HIGHEST committed floor (compaction advances a sequence of floors, and a lagging
or restarted replica's base is an earlier committed floor). Recovery now accepts
a base that any committed compaction floor reaches at or beyond
(`floor().index >= base.index`) — that committed authority covers discarding
every entry at or below the base. This also hardens the shipped
[follower-compaction](FOLLOWER-COMPACTION.md) feature (a node down during active
compaction can now rejoin); kind-105 migration truncation stays exact-match.

Validation packet: [docs/log-reclamation-v1](log-reclamation-v1/README.md);
crash-injection packet: [docs/log-reclamation-crash-v1](log-reclamation-crash-v1/README.md).
