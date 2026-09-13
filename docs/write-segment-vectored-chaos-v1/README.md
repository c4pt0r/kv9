# Vectored WAL candidate: actual Chaos Mesh evidence

Source `cfd9c927f8ecd33974100f696e6b08b227d25a41` passes the existing
21-window actual Chaos Mesh campaign, independent complete-history audit and
owned cleanup. This is a correctness checkpoint on one Kind host.

| History | Operations | OK | Unknown | Refused |
| --- | ---: | ---: | ---: | ---: |
| CLI/catalog | 5,473 | 4,947 | 514 | 12 |
| Persistent point stream | 1,489 | 1,464 | 20 | 5 |
| Native point/atomic batch | 2,674 | 2,626 | 38 | 10 |
| Total | 9,636 | 9,037 | 572 | 27 |

All invocations have recorded returns. All four fresh final replica drains
pass. All 34 recorded server lifetimes and 25 containers have exited. The
owned namespace is absent and all eight historical namespace UIDs remain
unchanged. Unknown and refused operations retain their original outcomes;
no uncertain write was replayed for success.

The default ThinLTO server SHA-256 is
`c88b79b53f10f76f4bb87c4293e1dd0de0a753e13dc1cce27bd0b44d45694581`.
The image ID is
`sha256:3c7e2ea1c640c3d0413676821571c020f692977c805b9a160da1a71db7e9c989`.
The release, auxiliary clients, actual compiler options, loaded CRI image,
661-file source map and process identities are bound in the original records.

## Verify the reporting archive

Run from this evidence worktree:

```sh
python3 docs/write-segment-vectored-chaos-v1/verify.py
```

The unchanged verifier checks every selected member, each archive part and the
three indexed top-level files. It performs no extraction, fault injection or
new history audit. The archive contains **2,443 regular files**,
**451,167,593 decoded bytes**, in **11 parts** totaling **23,068,610 bytes**.
The concatenated compressed SHA-256 is
`3d21ba36e8154ad05e816093b392b5d2d2feadcb76700eeb9c51fd797a3b047e`.
The inventory SHA-256 is
`5f8e8038517e630296d01fc8f071b0ef0dfadab2c1569e2adf8d11b79cf0c0ac`.

The complete CLI, point-stream and native histories, fault-effect records,
observer and pressure records, independent acceptance, archive readback,
cleanup receipts and exact source helpers are included. Forty-eight links
remain literal JSON metadata. The selection excludes compiled binaries,
WAL/data payloads, the duplicate independent-copy tree and large cache censuses.
The full local archive is retained separately: 94,002,806 bytes, SHA-256
`f121d511329532c3db77861f90ab3b7cc322eab2dd57f47c84b012552fe6f1c9`.
It binds 4,256 original files containing 961,786,668 bytes. Its full inventory
and readback are published here. This reporting subset alone cannot replay
every original full-file audit.

`README.md`, the publication terminal/result and publication resource samples
are separate Git-tracked supplements written after packaging. The archive
index binds the original archive and its three indexed top-level files; it
does not self-attest these later supplements.

## Execution and retained prerequisites

Actual runtime session 22461 exits with `fdf4a8/0`. The original six-phase post
sequence is session 88213, `67d0d5/0`: independent audit, cleanup capture,
process tree, archive, exact-UID cleanup and all-lifetime readback. Neither the
workload nor the post sequence was rerun. The independent result precedes
cleanup and retains `cleanup_complete=false`; final cleanup supplements it
without rewriting it.

The auditor prospectively uses the already corrected historical timestamp
receipt path and qualified parser. The predecessor's failed audit and repair
lineage are retained as historical evidence, not failures of this run.
The candidate's initial source-format failure and a pre-image metadata lookup
mistake are also retained separately from runtime acceptance.

The distinct dev-cache cleanup followed this candidate's completed source gate.
Fresh reference and hardlink checks preceded BuildCache cleaning of only the
eight first-party dev packages. All seven protected retained binaries remained
unchanged. Observed available space rose from 114,865,537,024 to
119,622,017,024 bytes, a 4,756,480,000-byte increase, without exclusive
attribution or reuse of earlier cleanup credit. Cleanup receipts and reference
checks are included. Four large original cache censuses remain local with
explicit full SHA/size bindings in `excluded-local-bindings.json`; the reporting
verifier does not replay those excluded censuses.

Root packaging passes session 10409, `efad7a/0`; integrity verification passes
`2feb2a/0`. Original resource, archive and timeout limits are preserved.
No hosted CI was dispatched.

## Scope

The 21 windows cover seed blackholing; each voter failing; partition; overload;
delay; EIO and ENOSPC on each voter; three missing-log and three replacement-PVC
refusals; and pending/recovered retained-volume endpoint migration. A normal
native baseline is checked separately. Source proofs and ordinary recovery
are separate prerequisite populations, never added to these operation counts.

Cross-host and power-loss recovery, the complete dedicated client-link/quorum
matrix, performance and full regressions remain separate gates. The vectored
stream proof has explicit Rust primitive, compiler and solver premises and is
not a whole-Rust or Raft implementation proof. This checkpoint changes no
default runtime and closes no original industrial work package.
