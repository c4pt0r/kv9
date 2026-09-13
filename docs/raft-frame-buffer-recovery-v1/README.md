# Raft frame-buffer release and ordinary recovery evidence

Exact source `01d128fd771dfbf0e6826ee5b6821411afac1fec` passes a clean default
ThinLTO release and ordinary three-voter recovery through streaming and unary
RPCs. Both complete histories pass independent checking: **353 operations,
323 OK and 30 unknown during faults**, with six fresh voter drains and all
five server/two client lifetimes exited. Unknown operations remain explicit;
they are not replayed as known failures or counted as successful acknowledgments.

[validation-summary.json](validation-summary.json) records exact source,
server/client hashes and actual terminals. Release session 3608 ends at
`9285b7/0`; independent release readback passes `f47eb6/0`. Recovery session
67872 ends at `aa24b3/0`, and the unchanged independent recovery auditor passes
`e57916/0`. The release checks 657 source files, default runtime features,
verbose opt-level 3 / ThinLTO / one codegen-unit commands, explicit first-party
invalidation and the shared BuildCache lock. Original 96-GiB preflight,
80-GiB runtime floor, 16-GiB additional budget, five-second samples and
1,200-second outer deadline remain intact. Minimum observed release free space
is 121,465,839,616 bytes. Protected retained releases remain unchanged.

[inventory.json](inventory.json) binds **130 original files**, totaling
**3,720,389 decoded bytes**, in [original-evidence.tar.gz](original-evidence.tar.gz)
(**315,798 compressed bytes**), SHA-256
`4dc4c229a95b2a3dd6045e8028df92988d7f31f7b9c086328f0a89b008cf2e7e`.
Every member was decoded and compared with its original source (`83a353/0`).
It includes all small original process files, WALs, histories, configurations,
replica observations, release manifests/logs, helper diffs and independent
check outputs. Two retained ELF payloads are omitted and separately hash-bound;
source code remains available at the exact Git commit. The main original-evidence
archive total is 64,408,311 bytes, below its existing 64-MiB allowance.

[package.py](package.py) and [publication-terminal.json](publication-terminal.json)
record the packaging procedure and actual receipt. They and this guide are
tracked by the Git commit; the archive inventory describes the original
release/recovery members. The publisher uses original local paths and is not
a standalone live recovery runner.

The recovery runner changes only source/build/output path literals from the
accepted CRC predecessor. Seven source-relative helpers and the generic
independent auditor remain byte-identical. Pending preparation files are
preserved; final bindings use the actual completed release. No new controls
or workload reruns were needed for unchanged behavior.

This is a bounded local process-restart correctness checkpoint. The
[source-bound frame proof](https://github.com/c4pt0r/kv9/blob/01d128fd771dfbf0e6826ee5b6821411afac1fec/docs/WRITE-RAFT-FRAME-BUFFER.md)
retains its Rust/vector/compiler premises and is not a whole-Rust/Raft proof.
Actual candidate Chaos Mesh, matched throughput/latency and full regressions
remain pending. The selected runtime stays `11113f6`; no original industrial
checklist item closes and hosted CI was not dispatched.
