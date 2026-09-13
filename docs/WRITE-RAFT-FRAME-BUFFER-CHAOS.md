# Raft frame-buffer candidate: actual Chaos Mesh qualification

Completed 2026-09-12 (America/Los_Angeles). The single-buffer Raft WAL
candidate `01d128fd771dfbf0e6826ee5b6821411afac1fec` passes the existing
21-window actual Chaos Mesh campaign, independent complete-history audit and
owned cleanup. The [immutable evidence and verification instructions](https://github.com/c4pt0r/kv9/blob/692f5e9211f7a80d02126b6ffdba84aba566b85a/docs/write-raft-frame-buffer-chaos-v1/README.md)
include the original failed post-run audit and its receipt-path repair.

| Complete history | Operations | OK | Unknown | Refused |
| --- | ---: | ---: | ---: | ---: |
| CLI/catalog | 5,604 | 5,093 | 499 | 12 |
| Persistent point stream | 1,512 | 1,496 | 14 | 2 |
| Native point/atomic batch | 2,702 | 2,661 | 26 | 15 |
| Total | 9,818 | 9,250 | 539 | 29 |

All invocations have recorded returns. Unknown and refused operations retain
their original outcomes; no uncertain write was replayed to obtain success.
Four fresh final replica drains pass. All 34 recorded server lifetimes and
25 containers have exited. The owned namespace is absent, and all eight
historical namespace UIDs remain unchanged.

## Faults, source and scope

The 21 windows cover seed blackholing; failure of each voter; partition;
admission overload; delay; EIO and ENOSPC on each voter; three missing-log and
three replacement-PVC refusals; and pending/recovered endpoint migration with
the retained volume. A normal native baseline precedes the fault windows.
Positive fault effects, complete point/batch/catalog histories, inline/apply
fences and final drained replica observations pass separate checking.

The default ThinLTO server SHA-256 is
`53945784b39f7c90951b96d6f0700f432412369c24f26dcdd1d27e1988541c8c`.
The image `kv9-chaos:write-raft-frame-buffer-01d128f-full21-first` has ID
`sha256:d207f1937c7043113f8598765ae2e34d03b5e0823a3654ede1f34294ad75b7b8`.
The evidence binds 657 source files, default production features, actual
compiler/codegen commands, auxiliary clients, copied image payloads, loaded
Kind/CRI identity and observed processes. Test-only pressure features do not
change the production server build.

The [source and ordinary recovery prerequisites](WRITE-RAFT-FRAME-BUFFER-RECOVERY.md)
already pass three universal SMT checks, three counterexample controls,
710 workspace tests/doctests, formatting, Clippy and 353 recovery operations.
Twenty-three existing tests remain ignored. The byte-layout proof has explicit
Rust primitive, allocation, compiler and solver premises; it is not a whole-Rust
or Raft implementation proof. These prerequisite populations are separate
from the Chaos operation count.

This campaign runs on one Kind host. Independent-host and power-loss recovery,
the complete dedicated client-link/quorum-loss matrix and industrial durability
acceptance remain open. No original industrial checklist item closes.

## Original failure and independent acceptance

The live fixture and observer pass session 18991, `d17a3c/0`. The first separate
post sequence fails immediately with `63c5fa/1`: its auditor expects an existing
timestamp-control receipt in an unpopulated local directory. No audit command,
independent artifact copy or cleanup runs in that failed attempt.

The repaired auditor reads and pins the original receipt already declared by
the frozen plan. The current timestamp reader differs from the historically
qualified reader only in two import paths; parser logic and acceptance
predicates are unchanged. The original auditor, failed results/stderr, exact
repair diff and source are retained. No live workload was rerun.

The repaired post sequence passes session 36002, `bedb0d/0`, across all six
phases: independent audit, cleanup capture, process tree, archive, exact-UID
cleanup and all-lifetime readback. The original independent result precedes
cleanup and retains `cleanup_complete=false`; final cleanup supplements that
record without rewriting it. The auxiliary preflight reader's earlier symlink
schema failure also remains visible as a separate preparation failure.

The full local archive passes complete member readback and source rehash before
cleanup: 4,238 original files, 972,448,591 decoded bytes, 94,380,807 compressed
bytes, SHA-256 `4e56ec8b842e4e6ecc47ee27faa434016b15aba0adc05cd2daa0a4e12d3be610`.
The published reporting subset contains 2,361 files in 12 bounded parts:
456,176,082 decoded bytes and 23,140,146 compressed bytes. All published
members pass SHA/size checking. It includes complete histories, fault effects,
observer/pressure records and acceptance/cleanup evidence; executable and WAL
payloads remain separately bound in the retained full archive. The reporting
subset alone cannot replay every original full-file audit.

## Performance gate and next work

No frame-buffer performance gain has been measured. The selected default stays
`11113f6`, and the CRC candidate remains separate. The [latest measured write
improvement](WRITE-CRC-PERFORMANCE.md) is still CRC's loaded BatchPut(64):
1,063,493.134 items/s, +18.807%, with p99 of 6.947–7.012 ms; loaded point writes
reach 139,402.831/s. Those measurements do not transfer to this candidate.

Next compare the exact frame-buffer release against the selected server with
the fixed native client, measuring throughput and tail latency under the
[existing write contract](WRITE-PERFORMANCE-NEXT.md). Preserve full point/batch
and mixed-read regressions before promotion. The prepared full CRC matrix
retains 24 smokes and 48 timed cohorts; its additional storage reservation
still needs qualification. No benchmark cap, retention floor, synchronization
rule or acknowledgment fence was relaxed. CI remains local.
