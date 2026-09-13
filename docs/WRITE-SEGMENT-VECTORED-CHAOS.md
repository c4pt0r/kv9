# Vectored WAL candidate: actual Chaos Mesh qualification

Completed 2026-09-12 (America/Los_Angeles). The segmented-WAL vectored-write
candidate `cfd9c927f8ecd33974100f696e6b08b227d25a41` passes the existing
21-window actual Chaos Mesh campaign, independent complete-history audit and
owned cleanup. The [immutable evidence and verification instructions](https://github.com/c4pt0r/kv9/blob/93f60bf2f1f5b9169699bb11eed9c8bf914db060/docs/write-segment-vectored-chaos-v1/README.md)
retain the original histories, source bindings, fault effects and execution
receipts. The workload and post sequence each passed on their first attempt.

| Complete history | Operations | OK | Unknown | Refused |
| --- | ---: | ---: | ---: | ---: |
| CLI/catalog | 5,473 | 4,947 | 514 | 12 |
| Persistent point stream | 1,489 | 1,464 | 20 | 5 |
| Native point/atomic batch | 2,674 | 2,626 | 38 | 10 |
| Total | 9,636 | 9,037 | 572 | 27 |

All invocations have recorded returns. Unknown and refused operations retain
their original outcomes; no uncertain write was replayed for success. Four
fresh final replica drains pass. All 34 recorded server lifetimes and 25
containers have exited. The owned namespace is absent, and all eight historical
namespace UIDs remain unchanged.

## Faults and exact build

The 21 windows cover seed blackholing; failure of each voter; partition;
admission overload; delay; EIO and ENOSPC on each voter; three missing-log and
three replacement-PVC refusals; and pending/recovered endpoint migration with
the retained volume. A normal native baseline precedes the fault windows.
Positive fault effects, complete point/batch/catalog histories, inline/apply
fences and final drained replica observations pass separate checking.

The default ThinLTO server SHA-256 is
`c88b79b53f10f76f4bb87c4293e1dd0de0a753e13dc1cce27bd0b44d45694581`.
The image `kv9-chaos:write-segment-vectored-cfd9c92-full21-first` has ID
`sha256:3c7e2ea1c640c3d0413676821571c020f692977c805b9a160da1a71db7e9c989`.
The evidence binds 661 source files, default production features, actual
compiler/codegen commands, auxiliary clients, copied image payloads, loaded
Kind/CRI identity and observed processes. Test-only pressure features do not
change the production server build.

The [source and ordinary recovery prerequisites](WRITE-SEGMENT-VECTORED-RECOVERY.md)
already pass three universal SMT checks, three counterexample controls,
714 workspace tests/doctests, formatting, Clippy and 352 recovery operations.
Twenty-three existing tests remain ignored. The actual file probe observes
one `writev` and following `fsync` per frame. These prerequisite populations are
separate from the Chaos operation count. The prefix/cursor proof has explicit
Rust `Write`/`IoSlice`, compiler and solver premises; it is not a whole-Rust or
Raft implementation proof.

## Independent acceptance and retained evidence

The live fixture and observer pass session 22461, `fdf4a8/0`. The original post
sequence passes session 88213, `67d0d5/0`, across all six phases: independent
audit, cleanup capture, process tree, archive, exact-UID cleanup and all-lifetime
readback. The independent result precedes cleanup and retains
`cleanup_complete=false`; final cleanup supplements that record without
rewriting it. The auditor prospectively uses the qualified historical timestamp
parser and corrected receipt path. Historical failures and their repair lineage
remain labeled separately from this successful run.

The full local archive passes complete member readback and source rehash before
cleanup: 4,256 original files, 961,786,668 decoded bytes, 94,002,806 compressed
bytes, SHA-256 `f121d511329532c3db77861f90ab3b7cc322eab2dd57f47c84b012552fe6f1c9`.
The published reporting subset contains 2,443 files in 11 bounded parts:
451,167,593 decoded bytes and 23,068,610 compressed bytes. Every published
member passes SHA/size checking. Complete histories, effects, observer/pressure
records and acceptance/cleanup evidence are included; executable and WAL
payloads remain separately bound in the full local archive. The reporting subset
alone cannot replay every original full-file audit.

A distinct, guarded cleanup retired this completed source gate's first-party
dev cache. Fresh reference and hardlink checks passed, and all seven protected
retained binaries remained unchanged. Observed free space increased by
4,756,480,000 bytes; this is not exclusive attribution or credit for earlier
cleanups. Its receipts and reference checks are published. Four large cache
censuses remain local with explicit full SHA/size bindings. The original
correctness reservation and archive limits remained enforced throughout.

## Performance gate and next work

No vectored-WAL performance gain has been measured. The selected default stays
`11113f6`; CRC and frame-buffer candidates remain separate. The [latest measured
write improvement](WRITE-CRC-PERFORMANCE.md) is still CRC's loaded BatchPut(64):
1,063,493.134 items/s, +18.807%, with p99 of 6.947–7.012 ms; loaded point writes
reach 139,402.831/s. Those measurements do not transfer to this candidate.

Next qualify sufficient retained-artifact storage, then compare exact releases
using the fixed native client and the [existing write contract](WRITE-PERFORMANCE-NEXT.md),
including throughput and tail latency. The prepared frame comparison and full
CRC regression matrix remain unrun. Preserve complete point/batch and mixed-read
regressions before promotion; no benchmark scope, retention floor, synchronization
rule or acknowledgment fence was relaxed.

This campaign runs on one Kind host. Independent-host and power-loss recovery,
the complete dedicated client-link/quorum-loss matrix and industrial durability
acceptance remain open. No original industrial checklist item closes. CI remains
local; no hosted workflow was dispatched.
