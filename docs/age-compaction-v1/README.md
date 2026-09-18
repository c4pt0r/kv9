# Age-compaction validation packet

See [the mechanism, the local age clock and the
limits](../AGE-COMPACTION.md).

- The serial qualifying workspace run (a runtime-only trigger addition),
  zero failures; strict Clippy (0 warnings) and formatting pass.
- Thirteen checked Lean theorems for the age-compaction model with six
  semantic mutation controls (a proposal without aging; a proposal
  without something to compact; a commit without a proposal; an unbacked
  floor; a regressed floor; a forced unexecutable floor) and two
  proof-policy controls; all sibling Lean models and the retention
  TLA/TLAPS model re-accepted against the exact working tree after the
  source-pin refresh.
- One accepted five-process real-MinIO e2e (`e2e-first`): with BOTH size
  triggers OFF (`KV9_AUTO_COMPACT_ENTRIES=0`, `KV9_AUTO_COMPACT_BYTES=0`)
  and ZERO manual verbs, a dozen tiny values leave the group far below
  every size threshold (retained entries = 12), and only after the idle
  5 s age threshold elapses does every voter compact (`log_first_index`
  1 → 15 on all three) — provably age-driven since nothing else could
  fire; a second tiny burst plus idle advances every voter's floor to
  ~28; a full-cluster restart recovers on the compacted logs with every
  sampled key serving and the group still writing.

Evidence archive: `evidence.tar.gz` (manifest in `validation.json`,
summary in `portable-readback.json`). Excludes ELF binaries, the
`root.bin` bootstrap credential, and all object-store credentials.
