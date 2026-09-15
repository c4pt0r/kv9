# Write diagnostic development checks

Local development validation for the [default-off write-path observer](../WRITE-PATH-DIAGNOSTICS.md).
The selected CRC algorithm remains unchanged; this is not a retained release,
recovery/Chaos acceptance or performance measurement.

| Check | Actual result |
| --- | --- |
| Initial diagnostic libraries | 255 Raft + 207 server passed, one existing server ignore; `51971/4e96fb/0` |
| Final diagnostic controls | Four Raft histogram/observer controls + one server JSON-bound control passed |
| Final default libraries | 250 Raft + 207 server passed, one existing server ignore |
| Clippy | Warnings denied; default workspace/all targets and diagnostic Raft/server library/test targets pass |
| Root feature and formatting | Diagnostic root binary check and workspace formatting pass; `30894/2e5eea/0` |

The final Clippy/test sequence exits `98704/77e66f/0`. Runtime-source hashes
remain unchanged during each command. The initial source differs from the
final source by the subsequently tested empty-histogram merge shortcut and
server JSON-size test, plus comment formatting. The inventory covers Cargo,
crate, root source and protocol inputs, not a full retained-release source
qualification. Historical checks are not relabeled as checks of a new source.

All builds run offline with four Cargo jobs and the shared retained-build lock.
Each sequence enforces an 8 GiB available-space floor, 16 GiB added-space
allowance and per-command timeout. The initial run's maximum sampled space
decrease is 2,080,428,032 bytes; the final Clippy/test sequence's is
2,644,168,704 bytes. These are shared-filesystem observations, not per-process
allocation accounting. No cleanup or historical evidence retirement occurred.

[validation.json](validation.json) lists exact commands, results and member
identities in [test-evidence.tar.gz](test-evidence.tar.gz). The archive contains
14 original logs, source-hash inventories, diffs and result records. All members
and the gzip stream were read back. It is 65,200 bytes with SHA-256
`73f831e97f4742e38722fa22578f54760fc38daf2d441458d00342ef7d03b41d`.
The source itself is in the associated commit; no executable or performance
dataset is included. The validation populations overlap and must not be added
together as distinct test cases.

No hosted workflow was dispatched. The next work is an instrumented release,
bounded real-load capture and an explicit overhead comparison. Actual
receipt-candidate hint/fallback counters remain separate follow-up work.
