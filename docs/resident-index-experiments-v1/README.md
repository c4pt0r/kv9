# Resident-index experiment packet

The [report](../RESIDENT-INDEX-EXPERIMENTS.md) contains the decisions and scope.

- `input.json` identifies the original retained segment and its verified hash.
- `corpus-analysis.json` records all 106 decoded frames and duplicate counts.
- `batches.bin.gz` contains the exact derived corpus; `corpus.json` pins its
  compressed and uncompressed bytes. This corpus is sufficient for the index
  experiment, but lacks WAL headers, checksums and Raft authority.
- `decode-corpus.py` is the original offline decoder. Re-running that extraction
  additionally requires the original segment, which remains on the data volume.
- `index_batch.rs`, `tests.log`, `micro.log` and `result.json` preserve the
  rejected sorting/coalescing prototype and its complete measurements.
- `value_reuse_two_pass.rs` and `value-reuse-*` preserve the rejected two-pass
  implementation and its measurements, including the insertion regression.
- `fused-first.patch`, `fused-first-result.json`, `fused-tests.log` and
  `fused-micro.log` preserve the earlier shared-entry-copy prototype.
- `fused-jemalloc-*` preserve the final paired experiment and helper test. Every
  final baseline/candidate row uses the same executable; per-order mean/p99
  values are retained. Timing terminal: `22010 / 18991d / 0`.
- `rpds-validation/` preserves the independent test execution and its original
  environment failures. Final suite: `56340 / 40896e / 0`, 53 passed.

The final source patch, test hook and archive-checked preparation script are in
[`scripts/resident-upsert`](../../scripts/resident-upsert/README.md). Preparation
readback `3a2c0a / 0` matches both patched Rust files to the actual tested source;
it does not rebuild or repeat a performance row. The immutable final experiment
executable remains under the data-volume campaign, with its hash in the result.

Original campaign:
`/mnt/data/kv9-work/batch-index-coalescing-development-20260915-first`.
No files in this packet establish selected-runtime improvement, new database
QPS, Redis parity, mechanized-proof completion or actual Chaos acceptance.
