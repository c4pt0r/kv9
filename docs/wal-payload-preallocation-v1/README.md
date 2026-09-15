# WAL preallocation development evidence

This packet records the default-off candidate on base `0fc2753`. Start with
[result.json](result.json) and the [report](../WAL-PAYLOAD-PREALLOCATION.md).
`actual-tool-receipts.json` maps the executions to actual terminal tool handles.
Original command records contain argv, working directory, selected environment,
timestamps and exit status. Outputs have not been normalized or summarized in
place of the originals.

- `micro.*` and `large-corpus-harness.rs` retain the original four-arm large-batch
  experiment and its source. `executable.json` identifies the original retained
  ELF. The corpus is referenced from the existing resident-index packet; its
  decoded SHA-256 is checked before use.
- `small-*` retains the later six-case experiment, added test harness and second
  ELF identity. Both executables use an actual Linux jemalloc global allocator.
  Their different source generations and workloads are not pooled together.
- `workspace-*` and `clippy-*` retain both configurations' full local regressions.
  Each workspace configuration has 851 passing tests and 28 ignored tests. The
  explicit encoder measurements ran separately; ignored external MinIO/history
  acceptance is still outside this increment.
- `proof-evidence.tar.gz` contains all 170 original source/command/result/log
  members for the strict capacity and CRC checks. Every member is verified
  against `proof-archive-inventory.json`. Compiled proof objects and extracted
  Rust executable binaries remain local; the archive contains their producing
  source and exact commands rather than claiming those binaries are included.
- `candidate-source/` freezes the runtime/manifest/test overlays. The preparation
  helper recreates the later isolated manifest and harness byte for byte.
  `preparation.json` includes the initial planned NVMe target; the actual release
  build records use `/home/dongxu/kv9/target`. Workspace regressions use the
  separately recorded reusable `/tmp` target.
- `resident-static-followup/` records the already completed inspection of the
  earlier rejected index path. Its unequal function sizes do not establish a
  causal explanation of the insertion regression. No new index timings ran.

The complete local development directory is
`/mnt/data/kv9-work/wal-payload-preallocation-development-20260915-first`.
Large binaries remain there. New logs, proof tools, source staging and archives
use the data volume; NVMe fixtures/cache retain their declared locations. No
old evidence was migrated, no global `TMPDIR` changed, and no hosted CI ran.

The Lean prototype's first unfinished induction draft failed (`c0beef/1`) before
the compiling draft (`178e45/0`); neither is substituted for the later strict
acceptance run (`77125/fa4d15/0`). Both strict proof runs and all local runtime
checks pass. This packet contains **no actual Chaos Mesh, server performance,
Redis comparison or default-selection result** for this candidate.
