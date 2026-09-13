# CRC write candidate: actual Chaos Mesh qualification

Completed 2026-09-12 (America/Los_Angeles). The isolated slicing-by-eight CRC
candidate `e748620a7b0ba326ac5f4fa8e3a6b1ff48553c6a` passes the original
21-window Chaos Mesh campaign, independent history audit and final cleanup.
The selected runtime remains `11113f6`; no candidate performance gain has
been measured. See the [immutable evidence and verification instructions](https://github.com/c4pt0r/kv9/blob/a46405277584055385f2d95a155b4609555d0db6/docs/write-crc-chaos-v1/README.md).

## Accepted operations and faults

| Complete history | Operations | OK | Unknown | Refused |
| --- | ---: | ---: | ---: | ---: |
| CLI/catalog | 5,629 | 5,063 | 552 | 14 |
| Persistent point stream | 1,512 | 1,491 | 15 | 6 |
| Native point/atomic batch | 2,692 | 2,651 | 33 | 8 |
| Total | 9,833 | 9,205 | 600 | 28 |

All invocations have recorded returns. Unknown writes remain unknown; they
are not relabeled successful or replayed to obtain success. The native history
includes 944 BatchGet, 942 BatchPut, 269 Get, 268 Delete and 269 Put calls.

The 21 fault windows cover seed blackholing, pod failure on each voter,
partition, public admission overload, delay, six EIO/ENOSPC storage-error
injections, three missing-log refusals, three replacement-volume refusals,
and pending/recovered endpoint migration using the retained volume. A normal
native baseline precedes those windows. Original fault specifications and
positive-effect observations are retained alongside the operation histories.

The separate audit accepts the original histories and source/executable/process
bindings, including four fresh final voter drains. All 34 observed server
lifetimes and 25 owned containers have exited. The owned namespace was removed;
all eight historical namespace UIDs were preserved. The original independent
audit ran before cleanup and retains `cleanup_complete=false`. Final cleanup
and lifetime receipts supplement that original record without rewriting it.

This is actual fault injection on one Kind host. It does not establish
independent-host failure tolerance, power-loss recovery or the complete
dedicated client-link/quorum-loss matrix. Those industrial gates remain open.

## Exact source and prerequisites

The default ThinLTO server SHA-256 is
`616eed1b823dfaa192a22b2b03a3e7d778bf71efdad7d2b875a49c4bfc94e476`.
The four-binary image is `kv9-chaos:write-crc-slicing8-e748620-full21-first`,
image ID `sha256:edf2bb8fd1c3e1b46b59e41ab91ef6e12dccb28b5868a0c5c2326929448f67de`.
The evidence binds the 660-file source map, actual verbose build/codegen,
auxiliary binaries, linkage, image probe and loaded Kind/CRI identity. The
production server uses default features; pressure-tool testing features do
not change the server build.

The [earlier source qualification](write-reference-qualification-v1/README.md)
already passes 47 distinct Lean theorem statements, 710 workspace tests/doctests
(23 existing ignored), Clippy and ordinary process recovery with 363 operations.
The proof establishes CRC arithmetic equivalence under explicit Rust primitive,
standard-library and compiler premises; it is not a whole-Rust or Raft proof.
These separate populations are not added to the Chaos operation count.

Actual runtime session 95644 ends with `17104e`, exit 0. Session 82646 ends
with `50ccf2`, exit 0 after all six independent-audit/archive/cleanup phases.
The publication includes all three complete histories, fault-effect records,
the complete observer and admission-pressure records, acceptance results and
cleanup receipts. Its 2,296 regular members contain 441,886,011 decoded bytes
in eleven bounded parts totaling 22,883,666 compressed bytes. Every source,
published member and part passes full SHA/size verification.

The reporting subset excludes compiled binaries, WAL payloads, the duplicate
independent-copy tree and unrelated host capacity censuses. Forty-nine links
remain literal JSON metadata. The full local archive retains excluded runtime
bytes: 93,158,941 bytes, SHA-256
`55061d073c41128a16644dd1724629d7889110e99de42c72be545dffd75d877d`.
Its complete inventory/readback is published; the portable subset alone cannot
replay every original full-file audit.

## Performance gate and next work

Twenty-eight finite A/B environment controls pass: 8 driver, 15 auditor and
5 native-smoke schema controls (`c84db0`, exit 0). The rejected first draft,
its original inventory and the corrected helper-loop binding are retained.
This qualifies preparation only; no A/B workload has run.

Next compare selected and candidate servers with the same fixed native v3
client: Put and BatchPut(64), concurrency 1/64, two opposite orders, eight
two-second smokes and sixteen ten-second timed cohorts. Disk capacity must be
qualified first under the original 96-GiB preflight and unchanged retention
caps/floors. Historical artifacts may be archived only with verified byte
preservation and their original failed or incomplete outcomes intact.

The [latest selected write baseline](WRITE-REDIS3-BASELINE.md) remains
136,519.558 point writes/s and 887,520.285 BatchPut(64) items/s at concurrency
64, with loaded batch p99 of 9.437–9.568 ms. This correctness milestone does
not change those measurements. Promotion requires useful throughput and
latency results plus full point/batch and mixed-read regressions. Preserve
Raft commit, synchronization, durable apply, response and ownership fences.
No hosted CI was dispatched and no original industrial checklist item closes.
