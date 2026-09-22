# Log-reclamation validation packet

See [the mechanism, the crash-safe swap and the
limits](../LOG-RECLAMATION.md).

- The serial qualifying workspace run (new storage unit tests: the rewrite
  round-trips a compacted base to an IDENTICAL recovered view and shrinks
  the file; a stale `raft.log.tmp` is ignored by recovery and a completed
  swap recovers the new file — crash-safe before and after the atomic
  rename), zero failures; strict Clippy (0 warnings) and formatting pass.
- Twelve checked Lean crash-safety theorems with six semantic mutation
  controls (a rename without a built image; a build from an incomplete
  image; a rename that loses a committed entry; a rename that regresses
  the commit; a partial file becoming live; recovery reading the temp
  file) and two proof-policy controls; all sibling Lean models and the
  retention TLA/TLAPS model re-accepted against the exact working tree
  after the source-pin refresh.
- One accepted five-process real-MinIO e2e (`e2e-third`): sustained
  ~4 KiB writes grow every voter's append-only `raft.log` past the
  131072-byte reclaim threshold; reclamation then PHYSICALLY shrinks each
  voter's file to under HALF its peak (peaks ~134 KiB → 33-58 KiB — the
  shrink the append-only log alone never delivered) while the compacted
  base survives (`log_first_index` stays > 1) and no key is lost; the
  group keeps serving and writing after the swap; a full-cluster restart
  recovers on the REWRITTEN logs with every sampled key serving.
- The retained earlier e2e attempts are load-bearing findings: `e2e-first`
  shrank only ~375 bytes (mismatched thresholds left the retained tail as
  nearly the whole file — a trivial, not physical, reclamation), and the
  second attempt could never observe all three voters above the threshold
  at once (reclamation is per-replica and async). Both drove the final
  meaningful gate: aggressive compaction plus per-voter peak/drop tracking
  asserting each file at least halves.

Evidence archive: `evidence.tar.gz` (manifest in `validation.json`,
summary in `portable-readback.json`). Excludes ELF binaries, the
`root.bin` bootstrap credential, and all object-store credentials.
