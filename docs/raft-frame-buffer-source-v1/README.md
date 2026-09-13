# Raft frame buffer: original source qualification records

This package retains both actual proof attempts and the successful local source
qualification of the isolated experiment based on selected `11113f6`. The first
proof attempt failed because a bounds declaration was on a comment line. The
second passes three universal SMT queries and three counterexample controls.
The production Rust source is identical across those proof attempts and the
source qualification. The proof assumes Rust vector/slice/serialization and
compiler semantics; it does not prove the entire storage implementation or Raft.

The local source run passes 710 tests and doctests, with 23 existing ignored,
workspace formatting, and Clippy with warnings denied. The new compatibility
case writes and replays 3,840 records. BuildCache receipts retain the shared lock,
first-party invalidation, actual compilation artifacts, source snapshots,
commands and full output. The root supervisor retains its five-second resource
samples and terminal receipt (`11743`, `5d6e04`, exit 0). Its source snapshots
include the earlier draft documentation; only documentation and this package
were added or updated after the tests. No subsequent runtime edit is covered by
this evidence.

`manifest.json` binds every archive member and the compressed archive. All
members were read back and byte-compared before publication. Paths inside the
original records are historical local paths. The archive contains no executable
or WAL payload. Exact release, ordinary recovery, actual Chaos Mesh, full
regressions and throughput/latency measurements remain pending. This source
checkpoint does not change the selected server or qualify a performance gain.
