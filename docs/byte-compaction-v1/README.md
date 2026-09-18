# Byte-compaction validation packet

See [the mechanism, the churn guard and the
limits](../BYTE-COMPACTION.md).

- 952 passing workspace tests/doctests in the serial qualifying run
  (including a new `DiskRaftStorage::retained_committed_bytes` unit test:
  the sum includes the live retained prefix and drops a compacted range
  where the append-only file size would not), zero failures; strict
  Clippy (0 warnings) and formatting pass.
- Twelve checked Lean theorems for the byte-compaction model with six
  semantic mutation controls (a proposal without grown bytes; a proposal
  firing on entry growth alone; a commit without a proposal; an unbacked
  floor; a regressed floor; a forced unexecutable floor) and two
  proof-policy controls; all thirty-five sibling Lean models and the
  retention TLA/TLAPS model re-accepted against the exact working tree
  after the source-pin refresh.
- One accepted five-process real-MinIO e2e (`e2e-first`): with the
  ENTRIES trigger OFF (`KV9_AUTO_COMPACT_ENTRIES=0`) and ZERO manual
  verbs, ~4 KiB values cross the 256 KiB byte threshold and every voter
  compacts at ~67 entries (`log_first_index` 1 → 67 on all three) — an
  entry count no entries trigger would fire on, so the compaction is
  provably byte-driven; `retained_log_bytes` is observed plumbed in
  status; a second burst advances every voter's floor to ~202; a
  full-cluster restart recovers on the compacted logs with every sampled
  key serving and the group still writing.

Evidence archive: `evidence.tar.gz` (manifest in `validation.json`,
summary in `portable-readback.json`). Excludes ELF binaries, the
`root.bin` bootstrap credential, and all object-store credentials.
