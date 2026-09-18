# Byte-based auto-compaction (bound the log by bytes, not entries)

Updated 2026-09-18. Parent [#9](https://github.com/c4pt0r/kv9/issues/9),
P1 storage [#20](https://github.com/c4pt0r/kv9/issues/20). The
[auto-compaction](AUTO-COMPACTION.md) trigger fired on retained ENTRY count
(`KV9_AUTO_COMPACT_ENTRIES`). Entries are a poor proxy for log cost when
value sizes vary: a group of large values accumulates disk and replay cost
fast but entries slowly. This adds a byte-based trigger.

## What this increment adds

1. **`KV9_AUTO_COMPACT_BYTES`** (0 = off): the retained committed PAYLOAD
   bytes that trigger an automatic compaction floor. Complements — does not
   replace — the entries trigger; either firing proposes a floor. Same floor
   (the replica's applied position), same committed kind-110 row, same
   all-matched-gated + follower-side execution. Only the trigger differs.
2. **The retained-byte signal.** `DiskRaftStorage::retained_committed_bytes`
   sums the payloads of the LIVE retained entries `[first_index, committed]`.
   The on-disk raft-log file is append-only and never shrinks, so a file-size
   metric would be wrong; compaction advancing `first_index` is what shrinks
   this sum. Exposed on the peer as `retained_log_bytes` and in status
   observability. O(retained), computed only in the opt-in byte branch.
3. **The execution-based churn guard.** A byte-triggered proposal fires only
   once the last floor has been EXECUTED on this replica
   (`log_first_index > last_floor`, or none yet) — so `retained_committed_bytes`
   then measures growth SINCE that floor, and a reading past the threshold is
   genuine post-floor accumulation, never a re-proposal stacked on a floor
   still awaiting its all-matched execution.

## Known limits, deliberately out of scope

Age-based selection (compact entries older than T) is deferred: it needs a
per-entry wall-clock the raft entry does not carry (an entry-format or
side-table change). No bounded absolute log size, no physical reclamation of
the append-only log file, no chaos campaign, no performance claims. The byte
metric excludes per-entry framing overhead (the payload sum dominates and is
the size an operator reasons about).

## Evidence

- 952 passing workspace tests/doctests in the serial qualifying run
  (including a new storage unit test: the byte sum includes the live prefix
  and excludes a compacted range where the append-only file size would not),
  zero failures; strict Clippy (0) and formatting pass.
- Twelve checked Lean theorems for the byte-compaction model with six
  semantic mutation controls (including a proposal firing on entry growth
  alone) and two proof-policy controls
  ([proofs/lean/byte-compaction](../proofs/lean/byte-compaction/README.md));
  all thirty-five sibling Lean models and the retention TLA/TLAPS model
  re-accepted against the exact working tree.
- One accepted five-process real-MinIO e2e (`scripts/auto-compact-bytes-e2e.py`):
  with `KV9_AUTO_COMPACT_ENTRIES=0` (the entries trigger OFF) and ZERO manual
  verbs, ~4 KiB values cross the 256 KiB byte threshold and EVERY voter
  compacts at ~67 entries — an entry count no entries trigger would fire on,
  so the compaction is provably byte-driven; a second burst advances every
  voter's floor further; a full-cluster restart recovers on the compacted
  logs of all three voters with every sampled key serving.

Validation packet: [docs/byte-compaction-v1](byte-compaction-v1/README.md).
