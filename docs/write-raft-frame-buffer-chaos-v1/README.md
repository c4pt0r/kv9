# Frame-buffer candidate: actual Chaos Mesh evidence

Source `01d128fd771dfbf0e6826ee5b6821411afac1fec` passes the existing
21-window actual Chaos Mesh campaign, independent complete-history audit and
owned cleanup. This is a correctness checkpoint on one Kind host.

| History | Operations | OK | Unknown | Refused |
| --- | ---: | ---: | ---: | ---: |
| CLI/catalog | 5,604 | 5,093 | 499 | 12 |
| Persistent point stream | 1,512 | 1,496 | 14 | 2 |
| Native point/atomic batch | 2,702 | 2,661 | 26 | 15 |
| Total | 9,818 | 9,250 | 539 | 29 |

All four fresh final replica drains pass. All 34 recorded server lifetimes and
25 containers have exited. The owned namespace is absent and all eight
historical namespace UIDs remain unchanged. Unknown and refused operations
retain their original outcomes; no uncertain write was replayed for success.

The default server SHA-256 is
`53945784b39f7c90951b96d6f0700f432412369c24f26dcdd1d27e1988541c8c`.
The image ID is
`sha256:d207f1937c7043113f8598765ae2e34d03b5e0823a3654ede1f34294ad75b7b8`.
The release, auxiliary clients, actual compiler options, loaded CRI image,
source map and process identities are bound in the original records.

## Verify the reporting archive

Run from this evidence worktree:

```sh
python3 docs/write-raft-frame-buffer-chaos-v1/verify.py
```

The unchanged verifier checks every selected member, each archive part and the
three indexed top-level files. It performs no extraction, fault injection or
new history audit. The archive contains **2,361 regular files**,
**456,176,082 decoded bytes**, in **12 parts** totaling **23,140,146 bytes**.
The concatenated compressed SHA-256 is
`2ace2f1aab3820a160e4f21451b853bb7dcda711ce9435c3e005f09fd375d140`.
The inventory SHA-256 is
`746c37121216e391dba73b73c43fb0804d46714ea564f1698902c9a6c9e0f03f`.

The complete CLI, point-stream and native histories, fault-effect records,
observer and pressure records, independent acceptance, archive readback,
cleanup receipts and exact source helpers are included. Forty-eight links
remain literal JSON metadata. The selection excludes compiled binaries,
WAL/data payloads, the duplicate independent-copy tree and unrelated capacity
censuses. The full local archive is retained separately:
94,380,807 bytes, SHA-256
`4e56ec8b842e4e6ecc47ee27faa434016b15aba0adc05cd2daa0a4e12d3be610`.
It binds 4,238 original files containing 972,448,591 bytes. Its full inventory
and readback are published here. The portable reporting subset alone cannot
replay every original full-file audit.

`README.md`, the publication terminal/result and publication resource samples
are separate Git-tracked supplements written after packaging. The archive
index binds the original archive and its three indexed top-level files; it
does not self-attest these later supplements.

## Original failures and repair

Actual runtime session 18991 exits with `d17a3c/0`. The first separate post-run
attempt exits with `63c5fa/1` before running any audit command or copying the
artifact: the auditor looks for a historical timestamp-control receipt in an
unpopulated local directory. The repair pins and reads the original receipt
already declared in the frozen plan. The timestamp parser and acceptance
predicates are unchanged; source comparison binds the historical parser
controls. The original auditor, failed stderr/results, exact repair diff and
source are included. No live workload was rerun.

The repaired six-phase post sequence is session 36002, `bedb0d/0`: independent
audit, cleanup capture, process tree, archive, exact-UID cleanup and all-lifetime
readback. The original independent result precedes cleanup and retains
`cleanup_complete=false`; final cleanup supplements it without rewriting it.
The auxiliary preflight reader's earlier symlink-schema failure is also
preserved as a preparation failure, separate from runtime acceptance.

Root packaging passes session 94432, `b23f1c/0`; the integrity verifier passes
`5fdfad/0`. All original file, archive, resource and timeout limits are preserved.
No hosted CI was dispatched.

## Scope

The 21 windows cover seed blackholing; each voter failing; partition; overload;
delay; EIO and ENOSPC on each voter; three missing-log and three replacement-PVC
refusals; and pending/recovered retained-volume endpoint migration. A normal
native baseline is checked separately. Source proofs and ordinary recovery
are separate prerequisite populations, never added to these operation counts.

Cross-host and power-loss recovery, the complete dedicated client-link/quorum
matrix, performance and full regressions remain separate gates. This checkpoint
changes no default runtime and closes no original industrial work package.
