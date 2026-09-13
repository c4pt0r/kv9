# Vectored segmented WAL: exact release and recovery qualified

The [isolated vectored-write candidate](https://github.com/c4pt0r/kv9/blob/cfd9c927f8ecd33974100f696e6b08b227d25a41/docs/WRITE-SEGMENT-VECTORED.md)
now passes exact release verification and ordinary recovery with complete
histories. It remains separate from CRC and the Raft frame-buffer candidate.
Selected runtime stays `11113f6`; no throughput or latency improvement is claimed.

## Exact release

The default-feature release binds source `cfd9c927f8ecd33974100f696e6b08b227d25a41`
and all 661 source files. Independent readback verifies opt-level 3, ThinLTO,
one codegen unit, the existing BuildCache lock, fresh first-party compilation,
the source tree, Cargo graph and copied executable hashes. Previously retained
selected/CRC/frame releases, the fixed benchmark client and the debug syscall
probe remain unchanged.

| Artifact | SHA-256 |
| --- | --- |
| Server | `c88b79b53f10f76f4bb87c4293e1dd0de0a753e13dc1cce27bd0b44d45694581` |
| Native correctness client | `bd18336b5da137adbe008bc69db15dbf7769b73a74a876b9461c493665086d83` |
| Release manifest | `4cf87b0c91fa777453eaca93be6860390674074406205ad9901e44b49bee71bb` |
| Source tree | `74968c91e621716047321e7975591eed9a278cf8e44fd2d0ae37985bcedb9ed6` |

The release and recovery retain the 96 GiB initial reservation, 80 GiB runtime
floor, 16 GiB consumption limit, five-second samples and 1,200-second outer
deadline. Cargo retains its existing per-command limits and four offline jobs.

## Ordinary recovery

The unchanged fixture runs overlapping point and atomic batch operations on
three loopback voters with ordinary WAL files, leader SIGKILL and restart using
the original directory. Both streaming and unary clients retain full histories,
including unknown outcomes, and successful progress across both fault windows.

| Transport | Complete operations | OK | Unknown | Fresh voter drains |
| --- | ---: | ---: | ---: | ---: |
| Streaming | 184 | 169 | 15 | 3 |
| Unary | 168 | 154 | 14 | 3 |
| Total | **352** | **323** | **29** | **6** |

The independent auditor accepts both complete histories and their point/batch
overlap witnesses. All five server and two client lifetimes have exited.
Source, build, executable, process identity, listener and original input bindings
pass. This is one ordinary recovery run; no failed workload was rerun.

[Portable evidence](wal-segment-vectored-recovery-v1/README.md) retains the
original release records, preparation, complete small WAL/history inputs,
process results, independent audit and exact readback. The source checkpoint's
three SMT checks, three countermodels and 714 tests remain separately recorded;
recovery operation counts are not additional unit tests.

Actual 21-window Chaos Mesh qualification, cross-host/power-loss acceptance and
matched throughput/latency with full regressions remain open. The single-host
process test does not close those gates or any original industrial checklist
item. GitHub CI was not dispatched.
