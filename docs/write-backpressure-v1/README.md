# Write-backpressure validation packet

See [the bound, the pre-append gate and the typed
refusal](../WRITE-BACKPRESSURE.md).

- The serial qualifying workspace run (955 passed, 0 failed, 35 ignored;
  the new `Error::WriteBackpressure` variant, its `RESOURCE_EXHAUSTED`
  mapping and the `RawGroup::permit` gate), zero failures; strict Clippy
  (0 warnings) and formatting pass.
- Ten checked Lean theorems (`proofs/lean/write-backpressure/`) with five
  semantic mutation controls (admit AT the bound, an append that grows the
  log by two, a refusal that mutates state, reads gated by the bound, a
  drain that grows instead of shrinks the log) and two proof-policy
  controls: the retained log never exceeds the bound, writes are admitted
  only strictly below it, no write grows the log at the bound, the refusal
  is typed and changes nothing, reads serve at any retention, draining
  always releases the bound, and the bound is reachable (non-vacuous). All
  38 sibling Lean models and the retention TLA/TLAPS model (7 theorems /
  55 obligations) were re-accepted against the exact working tree after the
  source-pin refresh for `range_api.rs`, `grpc.rs` and `error.rs`.
- One accepted five-process real-MinIO e2e (`backpressure-third`): with
  auto-compaction OFF (so nothing drains the log), 120 sustained writes
  fill the retained committed raft log to the absolute bound
  (`KV9_MAX_RAFT_LOG_ENTRIES=120`); the very next write is refused with a
  typed `RESOURCE_EXHAUSTED` (`Error::WriteBackpressure`); the retained log
  stays BOUNDED at the cap under continued refused writes (120 → 120, never
  grows past it); reads still serve while writes are backpressured; and a
  committed compaction floor drains the log below the cap so writes RESUME
  (backpressure released, not a permanent wedge).

Evidence archive: `evidence.tar.gz` (manifest in `validation.json`,
summary in `portable-readback.json`). Excludes ELF binaries, the
`root.bin` bootstrap credential, and all object-store credentials.
